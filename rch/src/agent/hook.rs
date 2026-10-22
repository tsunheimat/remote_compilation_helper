//! Hook management for AI coding agents.
//!
//! Provides idempotent hook installation and management for supported agents.

use super::types::AgentKind;
use crate::error::HookError;
use crate::state::primitives::{IdempotentResult, atomic_write, create_backup};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Return true when the given command-line invokes the `rch` binary.
///
/// Parse shell quoting so paths containing spaces are recognized without
/// claiming unrelated commands that merely mention RCH in their arguments.
fn is_rch_hook_command(cmd: &str) -> bool {
    let Ok(words) = shell_words::split(cmd) else {
        return false;
    };
    words.first().is_some_and(|word| {
        std::path::Path::new(word)
            .file_name()
            .is_some_and(|name| name == "rch")
    })
}

/// Reject links (including dangling ones) and special files before reading or
/// replacing settings. A missing regular-file destination is allowed.
fn settings_metadata(path: &std::path::Path) -> Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_file(),
                "Refusing non-regular settings file {}; leaving it unchanged",
                path.display()
            );
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Cannot inspect {}", path.display())),
    }
}

/// Atomically replace settings with the original permissions already applied
/// to the temporary file, never exposing private JSON under a broader umask.
/// Backups stay beside the settings, with unique names and the same permissions;
/// this avoids global backup collisions and retention deleting unrelated files.
fn write_json_settings(path: &std::path::Path, settings: &Value) -> Result<()> {
    use std::io::Write;

    let metadata = settings_metadata(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Settings path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if let Some(metadata) = &metadata {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
        let original = std::fs::read(path)?;
        let mut backup = tempfile::Builder::new()
            .prefix(".rch-settings-backup-")
            .suffix(".bak")
            .tempfile_in(parent)?;
        backup.as_file().set_permissions(metadata.permissions())?;
        backup.write_all(&original)?;
        backup.as_file().sync_all()?;
        backup.keep().context("Failed to retain settings backup")?;
    }
    // NamedTempFile defaults to owner-only permissions for new settings.
    temporary.write_all(serde_json::to_string_pretty(settings)?.as_bytes())?;
    temporary.as_file().sync_all()?;
    settings_metadata(path)?;
    temporary
        .persist(path)
        .context("Failed to replace settings atomically")?;
    if let Ok(directory) = std::fs::File::open(parent) {
        directory.sync_all()?;
    }
    Ok(())
}

/// Read settings without ever replacing malformed user configuration.
fn read_json_settings(path: &std::path::Path) -> Result<Value> {
    settings_metadata(path)?;
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .with_context(|| format!("Invalid JSON in {}; leaving it unchanged", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(error) => Err(error).with_context(|| format!("Cannot read {}", path.display())),
    }
}

/// Validate the merge surface before making any on-disk changes.
fn validate_pre_tool_settings(settings: &Value) -> Result<()> {
    anyhow::ensure!(settings.is_object(), "Settings is not an object");
    if let Some(hooks) = settings.get("hooks") {
        anyhow::ensure!(hooks.is_object(), "Hooks is not an object");
        if let Some(groups) = hooks.get("PreToolUse") {
            let groups = groups
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("PreToolUse is not an array"))?;
            for group in groups {
                anyhow::ensure!(group.is_object(), "PreToolUse entry is not an object");
                // Accept the old RCH-owned entry only so it can be repaired.
                if group.get("matcher").is_none()
                    && group
                        .get("command")
                        .and_then(Value::as_str)
                        .is_some_and(is_rch_hook_command)
                {
                    continue;
                }
                let handlers = group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow::anyhow!("PreToolUse entry has no hooks array"))?;
                anyhow::ensure!(
                    handlers.iter().all(Value::is_object),
                    "Hook handler is not an object"
                );
            }
        }
    }
    Ok(())
}

fn is_owned_handler(handler: &Value) -> bool {
    handler.get("type").and_then(Value::as_str) == Some("command")
        && handler
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_rch_hook_command)
}

/// Remove only owned handlers, not their unrelated siblings or group metadata.
fn remove_pre_tool_handlers(settings: &mut Value) {
    if let Some(groups) = settings
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut)
    {
        groups.retain_mut(|group| {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                let before = handlers.len();
                handlers.retain(|handler| !is_owned_handler(handler));
                return before == handlers.len() || !handlers.is_empty();
            }
            !(group.get("matcher").is_none()
                && group
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_rch_hook_command))
        });
    }
}

fn pre_tool_hook_status(settings: &Value) -> HookStatus {
    if validate_pre_tool_settings(settings).is_err() {
        return HookStatus::NeedsUpdate;
    }
    let Some(groups) = settings
        .pointer("/hooks/PreToolUse")
        .and_then(Value::as_array)
    else {
        return HookStatus::NotInstalled;
    };
    let mut owned = 0;
    let mut valid = 0;
    for group in groups {
        if let Some(handlers) = group.get("hooks").and_then(Value::as_array) {
            for handler in handlers.iter().filter(|handler| is_owned_handler(handler)) {
                owned += 1;
                if matches!(
                    group.get("matcher").and_then(Value::as_str),
                    Some("Bash" | "^Bash$")
                ) && handler.get("async").is_none_or(|value| value == false)
                {
                    valid += 1;
                }
            }
        } else if group
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_rch_hook_command)
        {
            owned += 1;
        }
    }
    match (owned, valid) {
        (0, _) => HookStatus::NotInstalled,
        (1, 1) => HookStatus::Installed,
        _ => HookStatus::NeedsUpdate,
    }
}

fn installed_hook_command() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|directory| directory.join("rch")))
        .filter(|path| path.exists())
        .map(|path| shell_words::quote(&path.to_string_lossy()).into_owned())
        .unwrap_or_else(|| "rch".to_string())
}

fn install_pre_tool_hook_at_path(
    path: &std::path::Path,
    dry_run: bool,
) -> Result<IdempotentResult> {
    let mut settings = read_json_settings(path)?;
    validate_pre_tool_settings(&settings)?;
    anyhow::ensure!(
        !has_legacy_handlers(&settings),
        "Legacy remote-test handler present; preview agents migrate instead of enabling duplicate routing"
    );
    if pre_tool_hook_status(&settings) == HookStatus::Installed {
        return Ok(IdempotentResult::Unchanged);
    }
    remove_pre_tool_handlers(&mut settings);
    let hook_path = installed_hook_command();
    let hooks = settings
        .as_object_mut()
        .expect("validated settings")
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let groups = hooks
        .as_object_mut()
        .expect("validated hooks")
        .entry("PreToolUse")
        .or_insert_with(|| serde_json::json!([]));
    groups
        .as_array_mut()
        .expect("validated groups")
        .push(serde_json::json!({
            "matcher": "Bash",
            "hooks": [{ "type": "command", "command": hook_path }]
        }));
    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would add RCH hook to {}",
            path.display()
        )));
    }
    write_json_settings(path, &settings)?;
    Ok(IdempotentResult::Changed)
}

fn uninstall_pre_tool_hook_at_path(
    path: &std::path::Path,
    dry_run: bool,
) -> Result<IdempotentResult> {
    let mut settings = read_json_settings(path)?;
    validate_pre_tool_settings(&settings)?;
    let original = settings.clone();
    remove_pre_tool_handlers(&mut settings);
    if original == settings {
        return Ok(IdempotentResult::Unchanged);
    }
    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would remove RCH hook from {}",
            path.display()
        )));
    }
    write_json_settings(path, &settings)?;
    Ok(IdempotentResult::Changed)
}

/// Only the actual legacy executable is owned, never an argument mentioning it.
fn is_legacy_handler(handler: &Value) -> bool {
    if handler.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| {
            shell_words::split(command).is_ok_and(|words| {
                // Refuse shell programs/pipelines, env wrappers and extra arguments:
                // the deployed adapter is invoked as one quoted executable path.
                words.len() == 1
                    && std::path::Path::new(&words[0])
                        .file_name()
                        .is_some_and(|name| {
                            name == "remote-test-codex-hook-bootstrap"
                                || name == "remote-test-codex-hook"
                        })
            })
        })
}

fn has_legacy_handlers(settings: &Value) -> bool {
    settings
        .pointer("/hooks/PreToolUse")
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| handlers.iter().any(is_legacy_handler))
            })
        })
}

#[derive(Debug, Serialize, Deserialize)]
struct RemovedHandlers {
    index: usize,
    metadata: Value,
    handlers: Vec<(usize, Value)>,
}

/// Recovery records contain only the hooks owned by this migration, not a copy
/// of the user's settings. Rollback merges into the current settings document.
#[derive(Debug, Serialize, Deserialize)]
struct HookMigration {
    version: u32,
    rolled_back: bool,
    removed: Vec<RemovedHandlers>,
    added: Option<Value>,
}

fn migration_journal(path: &std::path::Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Missing settings name"))?;
    Ok(path.with_file_name(format!(".{}.rch-migration.json", name.to_string_lossy())))
}

fn group_metadata(group: &Value) -> Value {
    let mut metadata = group.clone();
    metadata
        .as_object_mut()
        .expect("validated group")
        .remove("hooks");
    metadata
}

fn replace_json_if_unchanged(path: &std::path::Path, before: &Value, after: &Value) -> Result<()> {
    anyhow::ensure!(
        read_json_settings(path)? == *before,
        "Settings changed during migration; retry rather than overwriting unrelated edits"
    );
    write_json_settings(path, after)
}

fn migration_settings_path(kind: AgentKind) -> Result<PathBuf> {
    match kind {
        AgentKind::ClaudeCode => claude_code_settings_path(),
        AgentKind::CodexCli => codex_cli_config_path(),
        _ => anyhow::bail!("Legacy migration supports claude-code, codex-cli and hermes only"),
    }
    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))
}

/// Preview or apply an explicitly requested legacy migration. No runtime restart
/// or generic PATH activation is performed; the caller stages those separately.
pub fn migrate_hook(kind: AgentKind, dry_run: bool) -> Result<IdempotentResult> {
    if kind == AgentKind::Hermes {
        return migrate_hermes_at_path(&hermes_plugin_path()?, dry_run, false);
    }
    migrate_pre_tool_at_path(&migration_settings_path(kind)?, dry_run)
}

/// Undo only this migration's owned changes, preserving subsequent user edits.
pub fn rollback_hook(kind: AgentKind, dry_run: bool) -> Result<IdempotentResult> {
    if kind == AgentKind::Hermes {
        return migrate_hermes_at_path(&hermes_plugin_path()?, dry_run, true);
    }
    rollback_pre_tool_at_path(&migration_settings_path(kind)?, dry_run)
}

fn migrate_pre_tool_at_path(path: &std::path::Path, dry_run: bool) -> Result<IdempotentResult> {
    let before = read_json_settings(path)?;
    validate_pre_tool_settings(&before)?;
    let journal_path = migration_journal(path)?;
    if settings_metadata(&journal_path)?.is_some() {
        let journal: HookMigration = serde_json::from_value(read_json_settings(&journal_path)?)?;
        anyhow::ensure!(journal.version == 1, "Unknown migration journal version");
        if !journal.rolled_back {
            if !has_legacy_handlers(&before)
                && pre_tool_hook_status(&before) == HookStatus::Installed
            {
                return Ok(IdempotentResult::Unchanged);
            }
            anyhow::bail!("Migration journal already exists; run agents rollback before retrying");
        }
    }
    if !has_legacy_handlers(&before) {
        return Ok(IdempotentResult::Unchanged);
    }
    anyhow::ensure!(
        pre_tool_hook_status(&before) == HookStatus::NotInstalled,
        "Disable existing RCH hooks before migrating legacy routing (rollback must not restore duplicate routing)"
    );
    let mut after = before.clone();
    let mut removed = Vec::new();
    let groups = after
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut)
        .expect("legacy handlers require groups");
    for (index, group) in groups.iter_mut().enumerate() {
        let metadata = group_metadata(group);
        let handlers = group["hooks"].as_array_mut().expect("validated handlers");
        let owned: Vec<_> = handlers
            .iter()
            .enumerate()
            .filter(|(_, handler)| is_legacy_handler(handler))
            .map(|(index, handler)| (index, handler.clone()))
            .collect();
        if !owned.is_empty() {
            handlers.retain(|handler| !is_legacy_handler(handler));
            // Keep even empty groups, so group metadata and positions survive.
            removed.push(RemovedHandlers {
                index,
                metadata,
                handlers: owned,
            });
        }
    }
    let added = if pre_tool_hook_status(&before) == HookStatus::NotInstalled {
        let group = serde_json::json!({"matcher": "Bash", "hooks": [{"type": "command",
            "command": installed_hook_command()}]});
        groups.push(group.clone());
        Some(group)
    } else {
        None
    };
    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would replace {} exact legacy handler(s) in {}; journal {}; no restart or PATH change",
            removed
                .iter()
                .map(|group| group.handlers.len())
                .sum::<usize>(),
            path.display(),
            journal_path.display()
        )));
    }
    let journal = HookMigration {
        version: 1,
        rolled_back: false,
        removed,
        added,
    };
    // Persist recovery intent before the one atomic settings switch. A crash
    // before or after the switch is recoverable by the same rollback operation.
    write_json_settings(&journal_path, &serde_json::to_value(&journal)?)?;
    replace_json_if_unchanged(path, &before, &after)?;
    Ok(IdempotentResult::Changed)
}

fn rollback_pre_tool_at_path(path: &std::path::Path, dry_run: bool) -> Result<IdempotentResult> {
    let journal_path = migration_journal(path)?;
    if settings_metadata(&journal_path)?.is_none() {
        return Ok(IdempotentResult::Unchanged);
    }
    let mut journal: HookMigration = serde_json::from_value(read_json_settings(&journal_path)?)?;
    anyhow::ensure!(journal.version == 1, "Unknown migration journal version");
    if journal.rolled_back {
        return Ok(IdempotentResult::Unchanged);
    }
    let before = read_json_settings(path)?;
    validate_pre_tool_settings(&before)?;
    let mut after = before.clone();
    let groups = after
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("Hook groups changed since migration; refusing rollback"))?;
    if let Some(added) = &journal.added {
        let owned = &added["hooks"][0];
        let matches: Vec<_> = groups
            .iter()
            .enumerate()
            .filter_map(|(index, group)| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .filter(|handlers| handlers.contains(owned))
                    .map(|_| index)
            })
            .collect();
        anyhow::ensure!(
            matches.len() <= 1,
            "Migrated hook duplicated; refusing ambiguous rollback"
        );
        if let Some(index) = matches.first().copied() {
            let group = &mut groups[index];
            group["hooks"]
                .as_array_mut()
                .expect("validated handlers")
                .retain(|handler| handler != owned);
            if group["hooks"]
                .as_array()
                .expect("validated handlers")
                .is_empty()
                && group_metadata(group) == group_metadata(added)
            {
                groups.remove(index);
            }
        } else {
            anyhow::ensure!(
                pre_tool_hook_status(&before) == HookStatus::NotInstalled,
                "Migrated RCH handler changed; refusing to restore duplicate legacy routing"
            );
        }
    }
    // A separately added RCH handler must not coexist with restored legacy
    // routing, even if our exact handler was still present and removed above.
    anyhow::ensure!(
        groups.iter().all(|group| group
            .get("hooks")
            .and_then(Value::as_array)
            .is_none_or(|handlers| !handlers.iter().any(is_owned_handler))),
        "Additional RCH handlers added after migration; refusing duplicate-routing rollback"
    );
    for removed in &journal.removed {
        let matches: Vec<_> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group_metadata(group) == removed.metadata)
            .map(|(index, _)| index)
            .collect();
        let index = if matches.contains(&removed.index) {
            removed.index
        } else {
            anyhow::ensure!(
                matches.len() == 1,
                "Original hook group changed or is ambiguous; refusing rollback"
            );
            matches[0]
        };
        let handlers = groups[index]["hooks"]
            .as_array_mut()
            .expect("validated handlers");
        // Atomic publication means recovery sees either all the old handlers or
        // none. Preserve their multiplicity (legacy installs may contain duplicates).
        let already_restored = removed
            .handlers
            .iter()
            .all(|(position, handler)| handlers.get(*position) == Some(handler));
        if !already_restored {
            anyhow::ensure!(
                !handlers.iter().any(is_legacy_handler),
                "Legacy handlers changed or moved; refusing ambiguous rollback"
            );
            for (position, handler) in &removed.handlers {
                handlers.insert((*position).min(handlers.len()), handler.clone());
            }
        }
    }
    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would restore owned legacy handlers in {}; unrelated settings retained",
            path.display()
        )));
    }
    replace_json_if_unchanged(path, &before, &after)?;
    journal.rolled_back = true;
    write_json_settings(&journal_path, &serde_json::to_value(journal)?)?;
    Ok(IdempotentResult::Changed)
}

/// Status of RCH hook for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookStatus {
    /// Hook is installed and configured correctly.
    Installed,
    /// Hook is installed but outdated or misconfigured.
    NeedsUpdate,
    /// Hook is not installed.
    NotInstalled,
    /// Agent doesn't support hooks.
    NotSupported,
}

impl std::fmt::Display for HookStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookStatus::Installed => write!(f, "Installed"),
            HookStatus::NeedsUpdate => write!(f, "Needs update"),
            HookStatus::NotInstalled => write!(f, "Not installed"),
            HookStatus::NotSupported => write!(f, "Not supported"),
        }
    }
}

/// Check the hook status for an agent.
pub fn check_hook_status(kind: AgentKind) -> Result<HookStatus> {
    if !kind.hook_support().can_install_hook() {
        return Ok(HookStatus::NotSupported);
    }

    match kind {
        AgentKind::ClaudeCode => check_claude_code_hook(),
        AgentKind::GeminiCli => check_gemini_cli_hook(),
        AgentKind::CodexCli => check_codex_cli_hook(),
        AgentKind::Hermes => check_hermes_hook_at_path(&hermes_plugin_path()?),
        AgentKind::ContinueDev => check_continue_dev_hook(),
        _ => Ok(HookStatus::NotSupported),
    }
}

/// Install the RCH hook for an agent.
pub fn install_hook(kind: AgentKind, dry_run: bool) -> Result<IdempotentResult> {
    if !kind.hook_support().can_install_hook() {
        return Err(HookError::NotSupported {
            agent: kind.name().to_string(),
            operation: "installation".to_string(),
            reason: kind.hook_support().to_string(),
        }
        .into());
    }

    match kind {
        AgentKind::ClaudeCode => install_claude_code_hook(dry_run),
        AgentKind::GeminiCli => install_gemini_cli_hook(dry_run),
        AgentKind::CodexCli => install_codex_cli_hook(dry_run),
        AgentKind::Hermes => install_hermes_hook_at_path(&hermes_plugin_path()?, dry_run),
        AgentKind::ContinueDev => install_continue_dev_hook(dry_run),
        _ => Err(HookError::NotImplemented {
            agent: kind.name().to_string(),
            operation: "installation".to_string(),
        }
        .into()),
    }
}

/// Uninstall the RCH hook from an agent.
pub fn uninstall_hook(kind: AgentKind, dry_run: bool) -> Result<IdempotentResult> {
    if !kind.hook_support().can_install_hook() {
        return Err(HookError::NotSupported {
            agent: kind.name().to_string(),
            operation: "uninstallation".to_string(),
            reason: kind.hook_support().to_string(),
        }
        .into());
    }

    match kind {
        AgentKind::ClaudeCode => uninstall_claude_code_hook(dry_run),
        AgentKind::GeminiCli => uninstall_gemini_cli_hook(dry_run),
        AgentKind::CodexCli => uninstall_codex_cli_hook(dry_run),
        AgentKind::Hermes => uninstall_hermes_hook_at_path(&hermes_plugin_path()?, dry_run),
        AgentKind::ContinueDev => uninstall_continue_dev_hook(dry_run),
        _ => Err(HookError::NotImplemented {
            agent: kind.name().to_string(),
            operation: "uninstallation".to_string(),
        }
        .into()),
    }
}

// === Claude Code Hook ===

fn claude_code_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

fn check_claude_code_hook() -> Result<HookStatus> {
    let settings_path = match claude_code_settings_path() {
        Some(p) => p,
        None => return Ok(HookStatus::NotInstalled),
    };
    check_claude_code_hook_at_path(&settings_path)
}

/// Internal implementation that accepts a path for testability.
fn check_claude_code_hook_at_path(settings_path: &std::path::Path) -> Result<HookStatus> {
    if !settings_path.exists() {
        return Ok(HookStatus::NotInstalled);
    }

    let content =
        std::fs::read_to_string(settings_path).context("Failed to read Claude Code settings")?;

    let settings: Value = match serde_json::from_str(&content) {
        Ok(settings) => settings,
        Err(_) => {
            // Malformed JSON should not crash status checks. Treat as misconfigured.
            return Ok(HookStatus::NeedsUpdate);
        }
    };

    Ok(pre_tool_hook_status(&settings))
}

fn install_claude_code_hook(dry_run: bool) -> Result<IdempotentResult> {
    let settings_path = claude_code_settings_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    install_claude_code_hook_at_path(&settings_path, dry_run)
}

/// Internal implementation that accepts a path for testability.
fn install_claude_code_hook_at_path(
    settings_path: &std::path::Path,
    dry_run: bool,
) -> Result<IdempotentResult> {
    // Check if the .claude directory exists - if not, user doesn't use Claude Code
    // We should NOT create the directory for them
    let claude_dir = settings_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Could not determine .claude directory"))?;

    if !claude_dir.exists() {
        return Ok(IdempotentResult::NotApplicable(
            "~/.claude directory does not exist (user does not use Claude Code)".to_string(),
        ));
    }

    install_pre_tool_hook_at_path(settings_path, dry_run)
}

fn uninstall_claude_code_hook(dry_run: bool) -> Result<IdempotentResult> {
    let settings_path = claude_code_settings_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    uninstall_pre_tool_hook_at_path(&settings_path, dry_run)
}

// === Gemini CLI Hook ===

fn gemini_cli_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".gemini").join("settings.json"))
}

fn check_gemini_cli_hook() -> Result<HookStatus> {
    let settings_path = match gemini_cli_settings_path() {
        Some(p) => p,
        None => return Ok(HookStatus::NotInstalled),
    };

    if !settings_path.exists() {
        return Ok(HookStatus::NotInstalled);
    }

    let content = std::fs::read_to_string(&settings_path)?;
    let settings: Value = serde_json::from_str(&content)?;

    // Check for pre_tool_use hook with rch
    if let Some(hooks) = settings.get("hooks")
        && let Some(pre_tool_use) = hooks.get("pre_tool_use")
        && let Some(arr) = pre_tool_use.as_array()
    {
        for hook in arr {
            if let Some(cmd) = hook.get("command").and_then(|c| c.as_str())
                && is_rch_hook_command(cmd)
            {
                return Ok(HookStatus::Installed);
            }
        }
    }

    Ok(HookStatus::NotInstalled)
}

fn install_gemini_cli_hook(dry_run: bool) -> Result<IdempotentResult> {
    let settings_path = gemini_cli_settings_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would add RCH hook to {}",
            settings_path.display()
        )));
    }

    // Similar structure to Claude Code
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut settings: Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if check_gemini_cli_hook()? == HookStatus::Installed {
        return Ok(IdempotentResult::Unchanged);
    }

    if settings_path.exists() {
        create_backup(&settings_path)?;
    }

    let hook_entry = serde_json::json!({
        "command": "rch",
        "description": "Remote Compilation Helper"
    });

    let hooks = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Settings is not an object"))?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));

    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Hooks is not an object"))?;

    let pre_tool_use = hooks_obj
        .entry("pre_tool_use")
        .or_insert_with(|| serde_json::json!([]));

    if let Some(arr) = pre_tool_use.as_array_mut() {
        arr.push(hook_entry);
    }

    let content = serde_json::to_string_pretty(&settings)?;
    atomic_write(&settings_path, content.as_bytes())?;

    Ok(IdempotentResult::Changed)
}

fn uninstall_gemini_cli_hook(dry_run: bool) -> Result<IdempotentResult> {
    let settings_path = gemini_cli_settings_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if !settings_path.exists() {
        return Ok(IdempotentResult::Unchanged);
    }

    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would remove RCH hook from {}",
            settings_path.display()
        )));
    }

    let content = std::fs::read_to_string(&settings_path)?;
    let mut settings: Value = serde_json::from_str(&content)?;

    if check_gemini_cli_hook()? == HookStatus::NotInstalled {
        return Ok(IdempotentResult::Unchanged);
    }

    create_backup(&settings_path)?;

    if let Some(hooks) = settings.get_mut("hooks")
        && let Some(pre_tool_use) = hooks.get_mut("pre_tool_use")
        && let Some(arr) = pre_tool_use.as_array_mut()
    {
        arr.retain(|hook| {
            !hook
                .get("command")
                .and_then(|c| c.as_str())
                .map(is_rch_hook_command)
                .unwrap_or(false)
        });
    }

    let content = serde_json::to_string_pretty(&settings)?;
    atomic_write(&settings_path, content.as_bytes())?;

    Ok(IdempotentResult::Changed)
}

// === Codex CLI Hook ===

fn codex_cli_config_path() -> Option<PathBuf> {
    AgentKind::CodexCli.default_config_path()
}

fn check_codex_cli_hook() -> Result<HookStatus> {
    match codex_cli_config_path() {
        Some(path) => check_claude_code_hook_at_path(&path),
        None => Ok(HookStatus::NotInstalled),
    }
}

fn install_codex_cli_hook(dry_run: bool) -> Result<IdempotentResult> {
    let path = codex_cli_config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    // Codex consumes PreToolUse JSON in hooks.json. Never modify config.toml
    // or trust state; hook approval remains an explicit user decision in Codex.
    install_pre_tool_hook_at_path(&path, dry_run)
}

fn uninstall_codex_cli_hook(dry_run: bool) -> Result<IdempotentResult> {
    let path = codex_cli_config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    uninstall_pre_tool_hook_at_path(&path, dry_run)
}

// === Hermes plugin ===
// Hermes has no PreToolUse JSON settings entry: its plugin API registers a
// Python pre_tool_call callback. Keep these assets independent of remote-test.
const HERMES_PLUGIN: &str = include_str!("hermes_plugin.py");
const HERMES_MANIFEST: &str = "name: rch-rewrite\nversion: \"0.1.0\"\ndescription: Rewrite Hermes terminal builds through RCH.\nauthor: local\nhooks:\n  - pre_tool_call\nprovides_hooks:\n  - pre_tool_call\n";
const HERMES_DISABLED: &str =
    "\"\"\"RCH integration disabled; no hooks registered.\"\"\"\ndef register(ctx):\n    pass\n";

fn hermes_plugin_path() -> Result<PathBuf> {
    AgentKind::Hermes
        .default_config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))
}

fn check_hermes_hook_at_path(path: &std::path::Path) -> Result<HookStatus> {
    let source = path.join("__init__.py");
    if !source.exists() {
        return Ok(HookStatus::NotInstalled);
    }
    let content = std::fs::read_to_string(source)?;
    if content == HERMES_DISABLED {
        return Ok(HookStatus::NotInstalled);
    }
    if content == HERMES_PLUGIN
        && std::fs::read_to_string(path.join("plugin.yaml"))
            .is_ok_and(|manifest| manifest == HERMES_MANIFEST)
    {
        return Ok(HookStatus::Installed);
    }
    Ok(HookStatus::NeedsUpdate)
}

/// Refuse symlinks and unrecognized content rather than overwriting another
/// plugin or a user-edited copy. No activation/restart is performed here.
fn validate_hermes_assets(path: &std::path::Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if let Ok(metadata) = std::fs::symlink_metadata(ancestor) {
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "Refusing symlinked plugin directory {}",
                ancestor.display()
            );
        }
    }
    for (name, expected) in [
        ("__init__.py", HERMES_PLUGIN),
        ("plugin.yaml", HERMES_MANIFEST),
    ] {
        let file = path.join(name);
        match std::fs::symlink_metadata(&file) {
            Ok(metadata) => {
                anyhow::ensure!(
                    metadata.is_file(),
                    "Refusing non-regular plugin asset {}",
                    file.display()
                );
                let content = std::fs::read_to_string(&file)?;
                anyhow::ensure!(
                    content == expected || (name == "__init__.py" && content == HERMES_DISABLED),
                    "User-modified or unrecognized Hermes asset {}; leaving it unchanged",
                    file.display()
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn install_hermes_hook_at_path(path: &std::path::Path, dry_run: bool) -> Result<IdempotentResult> {
    validate_hermes_assets(path)?;
    if let Some(parent) = path.parent() {
        let legacy = parent.join("remote-test-rewrite").join("__init__.py");
        if settings_metadata(&legacy)?.is_some() {
            anyhow::ensure!(
                std::fs::read_to_string(&legacy)?.ends_with(LEGACY_HERMES_DISABLE),
                "Legacy Hermes plugin present; use agents migrate instead of enabling duplicate routing"
            );
        }
    }
    if check_hermes_hook_at_path(path)? == HookStatus::Installed {
        return Ok(IdempotentResult::Unchanged);
    }
    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would install Hermes plugin at {} (no restart)",
            path.display()
        )));
    }
    // Write the registration callback last. A partial first install cannot
    // register a hook before both assets are present.
    for (name, content) in [
        ("plugin.yaml", HERMES_MANIFEST),
        ("__init__.py", HERMES_PLUGIN),
    ] {
        let file = path.join(name);
        if std::fs::read_to_string(&file).is_ok_and(|current| current == content) {
            continue;
        }
        if file.exists() {
            create_backup(&file)?;
        }
        atomic_write(&file, content.as_bytes())?;
    }
    Ok(IdempotentResult::Changed)
}

fn uninstall_hermes_hook_at_path(
    path: &std::path::Path,
    dry_run: bool,
) -> Result<IdempotentResult> {
    validate_hermes_assets(path)?;
    if check_hermes_hook_at_path(path)? == HookStatus::NotInstalled {
        return Ok(IdempotentResult::Unchanged);
    }
    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would disable Hermes plugin at {} (no deletion or restart)",
            path.display()
        )));
    }
    let source = path.join("__init__.py");
    create_backup(&source)?;
    atomic_write(&source, HERMES_DISABLED.as_bytes())?;
    Ok(IdempotentResult::Changed)
}

// An explicit inert register override is the disable marker understood by the
// existing plugin's Python API; a sentinel file alone would not disable it.
const LEGACY_HERMES_DISABLE: &str = "\n# RCH migration: legacy routing disabled; original source retained above.\ndef register(ctx):\n    pass\n";

#[derive(Serialize, Deserialize)]
struct HermesMigration {
    version: u32,
    original_sha256: String,
    rolled_back: bool,
}

fn sha256_hex(content: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn write_plugin_if_unchanged(path: &std::path::Path, before: &str, after: &str) -> Result<()> {
    use std::io::Write;
    let metadata = settings_metadata(path)?.ok_or_else(|| anyhow::anyhow!("Plugin disappeared"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().expect("plugin parent"))?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())?;
    temporary.write_all(after.as_bytes())?;
    temporary.as_file().sync_all()?;
    anyhow::ensure!(
        std::fs::read_to_string(path)? == before,
        "Plugin changed during migration; retry"
    );
    settings_metadata(path)?;
    temporary
        .persist(path)
        .context("Failed to replace plugin atomically")?;
    std::fs::File::open(path.parent().expect("plugin parent"))?.sync_all()?;
    Ok(())
}

fn migrate_hermes_at_path(
    path: &std::path::Path,
    dry_run: bool,
    rollback: bool,
) -> Result<IdempotentResult> {
    validate_hermes_assets(path)?;
    let legacy = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Plugin has no parent"))?
        .join("remote-test-rewrite");
    for ancestor in legacy.ancestors() {
        if let Ok(metadata) = std::fs::symlink_metadata(ancestor) {
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "Refusing symlinked plugin directory"
            );
        }
    }
    let source = legacy.join("__init__.py");
    let journal_path = legacy.join(".rch-migration.json");
    if rollback {
        if settings_metadata(&journal_path)?.is_none() {
            return Ok(IdempotentResult::Unchanged);
        }
        let mut journal: HermesMigration =
            serde_json::from_value(read_json_settings(&journal_path)?)?;
        anyhow::ensure!(
            journal.version == 1,
            "Unknown Hermes migration journal version"
        );
        if journal.rolled_back {
            return Ok(IdempotentResult::Unchanged);
        }
        settings_metadata(&source)?;
        let current = std::fs::read_to_string(&source)?;
        let original = current
            .strip_suffix(LEGACY_HERMES_DISABLE)
            .unwrap_or(&current);
        anyhow::ensure!(
            sha256_hex(original.as_bytes()) == journal.original_sha256,
            "Legacy plugin changed after migration; refusing to overwrite edits"
        );
        if dry_run {
            return Ok(IdempotentResult::WouldChange("Would disable RCH Hermes routing and restore original register callback; restart Hermes afterwards".into()));
        }
        // Disable the replacement first: recovery never enables both adapters.
        uninstall_hermes_hook_at_path(path, false)?;
        if current != original {
            write_plugin_if_unchanged(&source, &current, original)?;
        }
        journal.rolled_back = true;
        write_json_settings(&journal_path, &serde_json::to_value(journal)?)?;
        return Ok(IdempotentResult::Changed);
    }
    if settings_metadata(&source)?.is_none() {
        return Ok(IdempotentResult::Unchanged);
    }
    let current = std::fs::read_to_string(&source)?;
    if settings_metadata(&journal_path)?.is_some() {
        let journal: HermesMigration = serde_json::from_value(read_json_settings(&journal_path)?)?;
        anyhow::ensure!(
            journal.version == 1,
            "Unknown Hermes migration journal version"
        );
        if !journal.rolled_back {
            if current
                .strip_suffix(LEGACY_HERMES_DISABLE)
                .is_some_and(|original| sha256_hex(original.as_bytes()) == journal.original_sha256)
                && check_hermes_hook_at_path(path)? == HookStatus::Installed
            {
                return Ok(IdempotentResult::Unchanged);
            }
            anyhow::bail!("Incomplete Hermes migration; run agents rollback before retrying");
        }
    }
    anyhow::ensure!(
        check_hermes_hook_at_path(path)? == HookStatus::NotInstalled,
        "RCH Hermes plugin already active; disable it before migrating legacy routing"
    );
    let marker = read_json_settings(&legacy.join(".remote-test-managed"))?;
    anyhow::ensure!(
        marker["managed_by"] == "remote-test-install" && marker["agent"] == "hermes",
        "Legacy Hermes ownership marker missing or invalid; plugin left untouched"
    );
    for name in ["__init__.py", "plugin.yaml"] {
        let asset = legacy.join(name);
        settings_metadata(&asset)?.ok_or_else(|| anyhow::anyhow!("Missing legacy plugin asset"))?;
        anyhow::ensure!(
            marker["files"][name].as_str() == Some(sha256_hex(&std::fs::read(asset)?).as_str()),
            "Legacy Hermes asset changed from its managed digest; refusing migration"
        );
    }
    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would append explicit disable marker to {}, preserve legacy files, then install RCH plugin; restart Hermes afterwards",
            source.display()
        )));
    }
    let journal = HermesMigration {
        version: 1,
        original_sha256: sha256_hex(current.as_bytes()),
        rolled_back: false,
    };
    write_json_settings(&journal_path, &serde_json::to_value(journal)?)?;
    write_plugin_if_unchanged(
        &source,
        &current,
        &format!("{current}{LEGACY_HERMES_DISABLE}"),
    )?;
    install_hermes_hook_at_path(path, false)?;
    Ok(IdempotentResult::Changed)
}

// === Continue.dev Hook ===

fn continue_dev_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".continue").join("config.json"))
}

fn check_continue_dev_hook() -> Result<HookStatus> {
    let config_path = match continue_dev_config_path() {
        Some(p) => p,
        None => return Ok(HookStatus::NotInstalled),
    };

    if !config_path.exists() {
        return Ok(HookStatus::NotInstalled);
    }

    let content = std::fs::read_to_string(&config_path)?;
    let config: Value = serde_json::from_str(&content)?;

    // Check for rch in experimental features or custom commands
    if let Some(experimental) = config.get("experimental")
        && let Some(pre_cmd) = experimental.get("preCompileCommand")
        && pre_cmd.as_str().map(is_rch_hook_command).unwrap_or(false)
    {
        return Ok(HookStatus::Installed);
    }

    Ok(HookStatus::NotInstalled)
}

fn install_continue_dev_hook(dry_run: bool) -> Result<IdempotentResult> {
    let config_path = continue_dev_config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would add RCH hook to {}",
            config_path.display()
        )));
    }

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config: Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if check_continue_dev_hook()? == HookStatus::Installed {
        return Ok(IdempotentResult::Unchanged);
    }

    if config_path.exists() {
        create_backup(&config_path)?;
    }

    let experimental = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Config is not an object"))?
        .entry("experimental")
        .or_insert_with(|| serde_json::json!({}));

    if let Some(exp_obj) = experimental.as_object_mut() {
        exp_obj.insert("preCompileCommand".to_string(), serde_json::json!("rch"));
    }

    let content = serde_json::to_string_pretty(&config)?;
    atomic_write(&config_path, content.as_bytes())?;

    Ok(IdempotentResult::Changed)
}

fn uninstall_continue_dev_hook(dry_run: bool) -> Result<IdempotentResult> {
    let config_path = continue_dev_config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if !config_path.exists() {
        return Ok(IdempotentResult::Unchanged);
    }

    if dry_run {
        return Ok(IdempotentResult::WouldChange(format!(
            "Would remove RCH hook from {}",
            config_path.display()
        )));
    }

    if check_continue_dev_hook()? == HookStatus::NotInstalled {
        return Ok(IdempotentResult::Unchanged);
    }

    create_backup(&config_path)?;

    let content = std::fs::read_to_string(&config_path)?;
    let mut config: Value = serde_json::from_str(&content)?;

    if let Some(experimental) = config.get_mut("experimental")
        && let Some(exp_obj) = experimental.as_object_mut()
    {
        exp_obj.remove("preCompileCommand");
    }

    let content = serde_json::to_string_pretty(&config)?;
    atomic_write(&config_path, content.as_bytes())?;

    Ok(IdempotentResult::Changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_matching_uses_exact_executable_not_mentions_or_shell_programs() {
        for command in [
            "remote-test-codex-hook-bootstrap",
            "'/opt/old tools/remote-test-codex-hook-bootstrap'",
            "/usr/local/libexec/remote-test-codex-hook",
        ] {
            assert!(is_legacy_handler(
                &serde_json::json!({"type": "command", "command": command})
            ));
        }
        for command in [
            "echo remote-test-codex-hook-bootstrap",
            "remote-test-codex-hook-bootstrap-extra",
            "remote-test-codex-hook-bootstrap && audit",
            "env remote-test-codex-hook-bootstrap",
            "remote-test-codex-hook-bootstrap --other",
        ] {
            assert!(!is_legacy_handler(
                &serde_json::json!({"type": "command", "command": command})
            ));
        }
        assert!(!is_legacy_handler(
            &serde_json::json!({"type": "prompt", "command": "remote-test-codex-hook-bootstrap"})
        ));
    }

    fn legacy_fixture() -> Value {
        serde_json::json!({"secret": "keep", "hooks": {"PreToolUse": [{"matcher": "Bash", "custom": true,
            "hooks": [{"type": "command", "command": "remote-test-codex-hook-bootstrap"},
                {"type": "command", "command": "audit", "timeout": 7}]}], "Stop": ["unrelated"]}})
    }

    #[test]
    fn migration_preview_apply_rollback_merges_later_unrelated_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let original = legacy_fixture();
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(install_pre_tool_hook_at_path(&path, false).is_err());
        assert!(matches!(
            migrate_pre_tool_at_path(&path, true).unwrap(),
            IdempotentResult::WouldChange(_)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(matches!(
            migrate_pre_tool_at_path(&path, false).unwrap(),
            IdempotentResult::Changed
        ));
        let mut current = read_json_settings(&path).unwrap();
        assert!(!has_legacy_handlers(&current));
        assert_eq!(pre_tool_hook_status(&current), HookStatus::Installed);
        assert!(matches!(
            migrate_pre_tool_at_path(&path, false).unwrap(),
            IdempotentResult::Unchanged
        ));
        current["later"] = serde_json::json!({"keep": true});
        current["hooks"]["PreToolUse"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"type": "command", "command": "later-audit"}));
        std::fs::write(&path, serde_json::to_vec(&current).unwrap()).unwrap();
        rollback_pre_tool_at_path(&path, true).unwrap();
        assert_eq!(read_json_settings(&path).unwrap(), current);
        rollback_pre_tool_at_path(&path, false).unwrap();
        let restored = read_json_settings(&path).unwrap();
        assert!(has_legacy_handlers(&restored));
        assert_eq!(pre_tool_hook_status(&restored), HookStatus::NotInstalled);
        assert_eq!(restored["later"], current["later"]);
        assert_eq!(restored["hooks"]["Stop"], original["hooks"]["Stop"]);
        assert_eq!(
            restored["hooks"]["PreToolUse"][0]["hooks"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(matches!(
            rollback_pre_tool_at_path(&path, false).unwrap(),
            IdempotentResult::Unchanged
        ));
    }

    #[test]
    fn rollback_refuses_changed_owned_handler_and_recovers_uncommitted_intent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let original = legacy_fixture();
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        migrate_pre_tool_at_path(&path, false).unwrap();
        let mut current = read_json_settings(&path).unwrap();
        current["hooks"]["PreToolUse"][1]["hooks"][0]["timeout"] = serde_json::json!(999);
        std::fs::write(&path, serde_json::to_vec(&current).unwrap()).unwrap();
        assert!(rollback_pre_tool_at_path(&path, false).is_err());
        assert_eq!(read_json_settings(&path).unwrap(), current);
        // Simulate a crash after journal sync but before settings publication.
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        rollback_pre_tool_at_path(&path, false).unwrap();
        assert_eq!(read_json_settings(&path).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn migration_preserves_private_modes_and_rejects_invalid_json() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        std::fs::write(&path, "{ malformed").unwrap();
        assert!(migrate_pre_tool_at_path(&path, false).is_err());
        assert!(!migration_journal(&path).unwrap().exists());
        std::fs::write(&path, serde_json::to_vec(&legacy_fixture()).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        migrate_pre_tool_at_path(&path, false).unwrap();
        rollback_pre_tool_at_path(&path, false).unwrap();
        for file in std::fs::read_dir(dir.path()).unwrap() {
            assert_eq!(
                file.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn hermes_migration_preserves_source_and_rolls_back_without_deletion() {
        let dir = tempfile::tempdir().unwrap();
        // macOS exposes its temporary root through /var -> /private/var.
        // Use the real owned directory for this positive fixture; the plugin
        // installer must still reject symlinks in its supplied authority path.
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("rch-rewrite");
        let legacy = root.join("remote-test-rewrite");
        std::fs::create_dir(&legacy).unwrap();
        let source = "def register(ctx):\n    ctx.register_hook('pre_tool_call', callback)\n";
        let manifest = "name: remote-test-rewrite\n";
        std::fs::write(legacy.join("__init__.py"), source).unwrap();
        std::fs::write(legacy.join("plugin.yaml"), manifest).unwrap();
        assert!(install_hermes_hook_at_path(&path, false).is_err());
        assert!(migrate_hermes_at_path(&path, false, false).is_err());
        std::fs::write(legacy.join(".remote-test-managed"), serde_json::to_vec(&serde_json::json!({
            "managed_by": "remote-test-install", "agent": "hermes", "files": {
                "__init__.py": sha256_hex(source.as_bytes()), "plugin.yaml": sha256_hex(manifest.as_bytes())
            }
        })).unwrap()).unwrap();
        migrate_hermes_at_path(&path, true, false).unwrap();
        assert!(!path.exists());
        assert!(!legacy.join(".rch-migration.json").exists());
        migrate_hermes_at_path(&path, false, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(legacy.join("__init__.py")).unwrap(),
            format!("{source}{LEGACY_HERMES_DISABLE}")
        );
        assert_eq!(
            check_hermes_hook_at_path(&path).unwrap(),
            HookStatus::Installed
        );
        assert!(matches!(
            migrate_hermes_at_path(&path, false, false).unwrap(),
            IdempotentResult::Unchanged
        ));
        migrate_hermes_at_path(&path, false, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(legacy.join("__init__.py")).unwrap(),
            source
        );
        assert_eq!(
            std::fs::read_to_string(legacy.join("plugin.yaml")).unwrap(),
            manifest
        );
        assert_eq!(
            check_hermes_hook_at_path(&path).unwrap(),
            HookStatus::NotInstalled
        );
        assert!(path.join("__init__.py").exists());
    }

    // ===== TEST: is_rch_hook_command =====

    #[test]
    fn is_rch_hook_command_matches_bare_name() {
        assert!(is_rch_hook_command("rch"));
    }

    #[test]
    fn is_rch_hook_command_matches_absolute_path() {
        assert!(is_rch_hook_command("/usr/local/bin/rch"));
        assert!(is_rch_hook_command("/home/user/.local/bin/rch"));
    }

    #[test]
    fn is_rch_hook_command_matches_with_args() {
        assert!(is_rch_hook_command("rch --some-flag"));
        assert!(is_rch_hook_command("/opt/rch/bin/rch --verbose arg"));
    }

    #[test]
    fn is_rch_hook_command_rejects_substring_false_positives() {
        // Regression: the previous `contains("rch")` implementation
        // matched any of these, which caused `rch hook uninstall` to
        // silently remove unrelated hooks from the user's settings.
        assert!(!is_rch_hook_command("search"));
        assert!(!is_rch_hook_command("/usr/bin/search"));
        assert!(!is_rch_hook_command("archive"));
        assert!(!is_rch_hook_command("rearchive"));
        assert!(!is_rch_hook_command("arch"));
        assert!(!is_rch_hook_command("rch-wkr"));
        assert!(!is_rch_hook_command("/usr/bin/rchd"));
        assert!(!is_rch_hook_command("myrchwrapper"));
        assert!(!is_rch_hook_command("rchextra"));
    }

    #[test]
    fn is_rch_hook_command_rejects_empty() {
        assert!(!is_rch_hook_command(""));
        assert!(!is_rch_hook_command("   "));
    }

    #[test]
    fn quoted_hook_paths_and_unrelated_commands() {
        assert!(is_rch_hook_command("'/opt/my tools/rch'"));
        assert!(is_rch_hook_command("\"/opt/my tools/rch\" --verbose"));
        assert!(!is_rch_hook_command("'/opt/my tools/rch"));
        assert!(!is_rch_hook_command("printf rch"));
        assert!(!is_rch_hook_command("remote-test-codex-hook-bootstrap"));
    }

    #[test]
    fn codex_json_roundtrip_preserves_sibling_hooks_and_trust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let config = dir.path().join("config.toml");
        let toml = "# user trust decision\n[features]\nhooks = true\n";
        std::fs::write(&config, toml).unwrap();
        let original = serde_json::json!({
            "other": {"retained": true},
            "hooks": {"PreToolUse": [{
                "matcher": "Bash", "custom": "group metadata",
                "hooks": [
                    {"type": "command", "command": "/usr/local/libexec/remote-test-codex-hook-bootstrap-audit"},
                    {"type": "command", "command": "dcg", "timeout": 7}
                ]
            }], "Stop": [{"hooks": [{"type": "command", "command": "log-stop"}]}]}
        });
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(matches!(
            install_pre_tool_hook_at_path(&path, false).unwrap(),
            IdempotentResult::Changed
        ));
        let installed = read_json_settings(&path).unwrap();
        assert_eq!(
            installed["hooks"]["PreToolUse"][0],
            original["hooks"]["PreToolUse"][0]
        );
        assert_eq!(pre_tool_hook_status(&installed), HookStatus::Installed);
        let bytes = std::fs::read(&path).unwrap();
        assert!(matches!(
            install_pre_tool_hook_at_path(&path, false).unwrap(),
            IdempotentResult::Unchanged
        ));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(matches!(
            uninstall_pre_tool_hook_at_path(&path, false).unwrap(),
            IdempotentResult::Changed
        ));
        assert_eq!(read_json_settings(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(config).unwrap(), toml);
    }

    #[test]
    fn uninstall_preserves_mixed_group_and_non_command_handlers() {
        let mut settings = serde_json::json!({"hooks": {"PreToolUse": [{
            "matcher": "Bash", "custom": 42,
            "hooks": [
                {"type": "command", "command": "'/opt/rch tools/rch'"},
                {"type": "command", "command": "dcg"},
                {"type": "prompt", "command": "rch", "prompt": "keep"}
            ]
        }]}});
        remove_pre_tool_handlers(&mut settings);
        assert_eq!(
            settings["hooks"]["PreToolUse"][0],
            serde_json::json!({
                "matcher": "Bash", "custom": 42,
                "hooks": [
                    {"type": "command", "command": "dcg"},
                    {"type": "prompt", "command": "rch", "prompt": "keep"}
                ]
            })
        );
    }

    #[test]
    fn repair_wrong_matcher_async_and_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let original = serde_json::json!({"hooks": {"PreToolUse": [
            {"matcher": "Read", "hooks": [{"type": "command", "command": "rch"}, {"type": "command", "command": "audit"}]},
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "rch", "async": true}]}
        ]}});
        assert_eq!(pre_tool_hook_status(&original), HookStatus::NeedsUpdate);
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        install_pre_tool_hook_at_path(&path, false).unwrap();
        let installed = read_json_settings(&path).unwrap();
        assert_eq!(pre_tool_hook_status(&installed), HookStatus::Installed);
        assert_eq!(
            installed["hooks"]["PreToolUse"][0],
            serde_json::json!({"matcher": "Read", "hooks": [{"type": "command", "command": "audit"}]})
        );
    }

    #[test]
    fn invalid_shapes_leave_configuration_bytes_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        for input in [
            "{ invalid",
            "null",
            "[]",
            r#"{"hooks":[]}"#,
            r#"{"hooks":{"PreToolUse":{}}}"#,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[null]}]}}"#,
        ] {
            std::fs::write(&path, input).unwrap();
            assert!(install_pre_tool_hook_at_path(&path, false).is_err());
            assert!(install_pre_tool_hook_at_path(&path, true).is_err());
            assert!(uninstall_pre_tool_hook_at_path(&path, false).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), input);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn settings_permissions_preserved_on_install_uninstall_and_backup() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        // Uninstall removes owned handlers, not pre-existing settings containers.
        // Start with that container so full document equality tests preservation
        // without requiring the installer to track ownership of JSON objects.
        let original = serde_json::json!({"secret": "private", "hooks": {"PreToolUse": []}});
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        install_pre_tool_hook_at_path(&path, false).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        uninstall_pre_tool_hook_at_path(&path, false).unwrap();
        assert_eq!(read_json_settings(&path).unwrap(), original);
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect();
        assert_eq!(entries.len(), 3, "settings and two distinct backups");
        for entry in entries {
            assert_eq!(
                entry.metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn settings_symlinks_and_nonregular_targets_are_preserved() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.json");
        std::fs::write(&target, "{\"secret\":\"private\"}").unwrap();
        let link = dir.path().join("hooks.json");
        symlink(&target, &link).unwrap();
        let dangling = dir.path().join("dangling.json");
        symlink(dir.path().join("missing.json"), &dangling).unwrap();
        let directory = dir.path().join("directory.json");
        std::fs::create_dir(&directory).unwrap();
        for path in [&link, &dangling, &directory] {
            for dry_run in [true, false] {
                assert!(install_pre_tool_hook_at_path(path, dry_run).is_err());
                assert!(uninstall_pre_tool_hook_at_path(path, dry_run).is_err());
            }
        }
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert!(
            std::fs::symlink_metadata(&dangling)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(directory.is_dir());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "{\"secret\":\"private\"}"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
    }

    #[test]
    fn hermes_install_disable_and_reinstall_preserve_other_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let sibling = root.join("unrelated-rewrite");
        std::fs::create_dir(&sibling).unwrap();
        std::fs::write(sibling.join("__init__.py"), "user plugin").unwrap();
        let path = root.join("rch-rewrite");
        assert!(matches!(
            install_hermes_hook_at_path(&path, true).unwrap(),
            IdempotentResult::WouldChange(_)
        ));
        assert!(!path.exists());
        assert!(matches!(
            install_hermes_hook_at_path(&path, false).unwrap(),
            IdempotentResult::Changed
        ));
        assert_eq!(
            check_hermes_hook_at_path(&path).unwrap(),
            HookStatus::Installed
        );
        assert!(matches!(
            install_hermes_hook_at_path(&path, false).unwrap(),
            IdempotentResult::Unchanged
        ));
        assert!(matches!(
            uninstall_hermes_hook_at_path(&path, false).unwrap(),
            IdempotentResult::Changed
        ));
        assert_eq!(
            check_hermes_hook_at_path(&path).unwrap(),
            HookStatus::NotInstalled
        );
        assert!(path.join("__init__.py").exists());
        assert!(path.join("plugin.yaml").exists());
        install_hermes_hook_at_path(&path, false).unwrap();
        assert_eq!(
            check_hermes_hook_at_path(&path).unwrap(),
            HookStatus::Installed
        );
        assert_eq!(
            std::fs::read_to_string(sibling.join("__init__.py")).unwrap(),
            "user plugin"
        );
    }

    #[test]
    fn hermes_refuses_user_modified_plugin_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("rch-rewrite");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("__init__.py"), "user code").unwrap();
        assert!(install_hermes_hook_at_path(&path, false).is_err());
        assert!(uninstall_hermes_hook_at_path(&path, false).is_err());
        assert!(!path.join("plugin.yaml").exists());
        assert_eq!(
            std::fs::read_to_string(path.join("__init__.py")).unwrap(),
            "user code"
        );
    }

    // ===== TEST: HookStatus =====

    #[test]
    fn test_hook_status_display() {
        eprintln!("TEST START: test_hook_status_display");
        assert_eq!(HookStatus::Installed.to_string(), "Installed");
        assert_eq!(HookStatus::NeedsUpdate.to_string(), "Needs update");
        assert_eq!(HookStatus::NotInstalled.to_string(), "Not installed");
        assert_eq!(HookStatus::NotSupported.to_string(), "Not supported");
        eprintln!("TEST PASS: test_hook_status_display");
    }

    #[test]
    fn test_hook_status_equality() {
        eprintln!("TEST START: test_hook_status_equality");
        assert_eq!(HookStatus::Installed, HookStatus::Installed);
        assert_ne!(HookStatus::Installed, HookStatus::NotInstalled);
        assert_ne!(HookStatus::NeedsUpdate, HookStatus::NotSupported);
        eprintln!("TEST PASS: test_hook_status_equality");
    }

    #[test]
    fn test_hook_status_serde_roundtrip() {
        eprintln!("TEST START: test_hook_status_serde_roundtrip");
        let statuses = [
            HookStatus::Installed,
            HookStatus::NeedsUpdate,
            HookStatus::NotInstalled,
            HookStatus::NotSupported,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).expect("serialize");
            let parsed: HookStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(status, parsed);
            eprintln!("  {:?} <-> {}", status, json);
        }
        eprintln!("TEST PASS: test_hook_status_serde_roundtrip");
    }

    // ===== TEST: check_hook_status =====

    #[test]
    fn test_check_unsupported_agents() {
        eprintln!("TEST START: test_check_unsupported_agents");
        // Agents without hook support should return NotSupported
        assert_eq!(
            check_hook_status(AgentKind::Cursor).unwrap(),
            HookStatus::NotSupported
        );
        assert_eq!(
            check_hook_status(AgentKind::Aider).unwrap(),
            HookStatus::NotSupported
        );
        assert_eq!(
            check_hook_status(AgentKind::Windsurf).unwrap(),
            HookStatus::NotSupported
        );
        assert_eq!(
            check_hook_status(AgentKind::Cline).unwrap(),
            HookStatus::NotSupported
        );
        eprintln!("TEST PASS: test_check_unsupported_agents");
    }

    #[test]
    fn test_check_hook_status_all_agents_no_panic() {
        eprintln!("TEST START: test_check_hook_status_all_agents_no_panic");
        for kind in AgentKind::ALL {
            let result = check_hook_status(*kind);
            assert!(
                result.is_ok(),
                "check_hook_status({:?}) should not error",
                kind
            );
            let status = result.unwrap();
            eprintln!("  {:?}: {:?}", kind, status);

            // Verify consistency with hook_support
            if !kind.hook_support().can_install_hook() {
                assert_eq!(
                    status,
                    HookStatus::NotSupported,
                    "Agent {:?} should be NotSupported",
                    kind
                );
            }
        }
        eprintln!("TEST PASS: test_check_hook_status_all_agents_no_panic");
    }

    #[test]
    fn test_check_hook_status_supported_agents() {
        eprintln!("TEST START: test_check_hook_status_supported_agents");
        // Supported agents: ClaudeCode, GeminiCli, CodexCli, ContinueDev
        let supported = [
            AgentKind::ClaudeCode,
            AgentKind::GeminiCli,
            AgentKind::CodexCli,
            AgentKind::ContinueDev,
        ];

        for kind in supported {
            let status = check_hook_status(kind).expect("should not error");
            // Status should be one of Installed, NeedsUpdate, or NotInstalled
            assert!(
                matches!(
                    status,
                    HookStatus::Installed | HookStatus::NeedsUpdate | HookStatus::NotInstalled
                ),
                "Unexpected status for {:?}: {:?}",
                kind,
                status
            );
            eprintln!("  {:?}: {:?}", kind, status);
        }
        eprintln!("TEST PASS: test_check_hook_status_supported_agents");
    }

    // ===== TEST: install_hook / uninstall_hook =====

    #[test]
    fn test_install_hook_unsupported_agents_error() {
        eprintln!("TEST START: test_install_hook_unsupported_agents_error");
        let unsupported = [
            AgentKind::Cursor,
            AgentKind::Windsurf,
            AgentKind::Aider,
            AgentKind::Cline,
        ];

        for kind in unsupported {
            let result = install_hook(kind, true); // dry_run = true
            assert!(
                result.is_err(),
                "install_hook({:?}) should error for unsupported agent",
                kind
            );
            eprintln!("  {:?}: correctly rejected", kind);
        }
        eprintln!("TEST PASS: test_install_hook_unsupported_agents_error");
    }

    #[test]
    fn test_uninstall_hook_unsupported_agents_error() {
        eprintln!("TEST START: test_uninstall_hook_unsupported_agents_error");
        let unsupported = [
            AgentKind::Cursor,
            AgentKind::Windsurf,
            AgentKind::Aider,
            AgentKind::Cline,
        ];

        for kind in unsupported {
            let result = uninstall_hook(kind, true); // dry_run = true
            assert!(
                result.is_err(),
                "uninstall_hook({:?}) should error for unsupported agent",
                kind
            );
            eprintln!("  {:?}: correctly rejected", kind);
        }
        eprintln!("TEST PASS: test_uninstall_hook_unsupported_agents_error");
    }

    /// Re-execute the public dispatch test with a private home. Child-local env
    /// avoids both inspecting live hooks and racing parallel tests via set_var.
    fn run_with_isolated_agent_home(test: &str) -> bool {
        const CHILD: &str = "RCH_AGENT_HOOK_FIXTURE_CHILD";
        if std::env::var(CHILD).as_deref() == Ok(test) {
            return false;
        }
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD, test)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("CODEX_HOME", home.join(".codex"))
            .env("HERMES_HOME", home.join(".hermes"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated test {test} failed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "test selector did not execute {test}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            std::fs::read_dir(home.join(".claude")).unwrap().count(),
            0,
            "dry run must not create settings or backups"
        );
        true
    }

    #[test]
    fn test_install_hook_dry_run_supported_agents() {
        if run_with_isolated_agent_home(
            "agent::hook::tests::test_install_hook_dry_run_supported_agents",
        ) {
            return;
        }
        eprintln!("TEST START: test_install_hook_dry_run_supported_agents");
        let supported = [
            AgentKind::ClaudeCode,
            AgentKind::GeminiCli,
            AgentKind::CodexCli,
            AgentKind::ContinueDev,
        ];

        for kind in supported {
            let result = install_hook(kind, true); // dry_run = true
            assert!(
                result.is_ok(),
                "install_hook({:?}, dry_run=true) should not error: {:?}",
                kind,
                result
            );
            let outcome = result.unwrap();
            eprintln!("  {:?}: {:?}", kind, outcome);
        }
        eprintln!("TEST PASS: test_install_hook_dry_run_supported_agents");
    }

    #[test]
    fn test_uninstall_hook_dry_run_supported_agents() {
        if run_with_isolated_agent_home(
            "agent::hook::tests::test_uninstall_hook_dry_run_supported_agents",
        ) {
            return;
        }
        eprintln!("TEST START: test_uninstall_hook_dry_run_supported_agents");
        let supported = [
            AgentKind::ClaudeCode,
            AgentKind::GeminiCli,
            AgentKind::CodexCli,
            AgentKind::ContinueDev,
        ];

        for kind in supported {
            let result = uninstall_hook(kind, true); // dry_run = true
            assert!(
                result.is_ok(),
                "uninstall_hook({:?}, dry_run=true) should not error: {:?}",
                kind,
                result
            );
            let outcome = result.unwrap();
            eprintln!("  {:?}: {:?}", kind, outcome);
        }
        eprintln!("TEST PASS: test_uninstall_hook_dry_run_supported_agents");
    }

    // ===== TEST: Config path helpers =====

    #[test]
    fn test_claude_code_settings_path() {
        eprintln!("TEST START: test_claude_code_settings_path");
        let path = claude_code_settings_path();
        if let Some(p) = &path {
            assert!(p.to_string_lossy().contains(".claude"));
            assert!(p.to_string_lossy().contains("settings.json"));
            eprintln!("  Path: {:?}", p);
        } else {
            eprintln!("  Path: None (HOME not set)");
        }
        eprintln!("TEST PASS: test_claude_code_settings_path");
    }

    #[test]
    fn test_gemini_cli_settings_path() {
        eprintln!("TEST START: test_gemini_cli_settings_path");
        let path = gemini_cli_settings_path();
        if let Some(p) = &path {
            assert!(p.to_string_lossy().contains(".gemini"));
            assert!(p.to_string_lossy().contains("settings.json"));
            eprintln!("  Path: {:?}", p);
        } else {
            eprintln!("  Path: None (HOME not set)");
        }
        eprintln!("TEST PASS: test_gemini_cli_settings_path");
    }

    #[test]
    fn test_codex_cli_config_path() {
        eprintln!("TEST START: test_codex_cli_config_path");
        let path = codex_cli_config_path();
        if let Some(p) = &path {
            assert!(p.to_string_lossy().contains(".codex"));
            assert!(p.to_string_lossy().contains("hooks.json"));
            eprintln!("  Path: {:?}", p);
        } else {
            eprintln!("  Path: None (HOME not set)");
        }
        eprintln!("TEST PASS: test_codex_cli_config_path");
    }

    #[test]
    fn test_continue_dev_config_path() {
        eprintln!("TEST START: test_continue_dev_config_path");
        let path = continue_dev_config_path();
        if let Some(p) = &path {
            assert!(p.to_string_lossy().contains(".continue"));
            assert!(p.to_string_lossy().contains("config.json"));
            eprintln!("  Path: {:?}", p);
        } else {
            eprintln!("  Path: None (HOME not set)");
        }
        eprintln!("TEST PASS: test_continue_dev_config_path");
    }

    // ===== TEST: Self-Healing Hook Installation (bd-59kg) =====
    //
    // These tests use the internal `_at_path` functions directly to avoid
    // needing to manipulate HOME env var (which requires unsafe in Rust 2024).

    /// Test environment helper for isolated hook tests.
    struct TestEnv {
        temp_dir: tempfile::TempDir,
    }

    impl TestEnv {
        fn new() -> Self {
            Self {
                temp_dir: tempfile::TempDir::new().expect("Failed to create temp dir"),
            }
        }

        fn claude_dir(&self) -> std::path::PathBuf {
            self.temp_dir.path().join(".claude")
        }

        fn settings_path(&self) -> std::path::PathBuf {
            self.claude_dir().join("settings.json")
        }

        fn create_claude_dir(&self) {
            std::fs::create_dir_all(self.claude_dir()).expect("Failed to create .claude dir");
        }

        fn write_settings_json(&self, value: &serde_json::Value) {
            std::fs::write(
                self.settings_path(),
                serde_json::to_string_pretty(value).unwrap(),
            )
            .expect("Failed to write settings.json");
        }

        fn read_settings_json(&self) -> serde_json::Value {
            let content = std::fs::read_to_string(self.settings_path())
                .expect("Failed to read settings.json");
            serde_json::from_str(&content).expect("Failed to parse settings.json")
        }
    }

    /// CRITICAL TEST: When ~/.claude directory doesn't exist, return NotApplicable
    /// and do NOT create the directory.
    #[test]
    fn test_install_hook_at_path_no_claude_dir_returns_not_applicable() {
        eprintln!("TEST START: test_install_hook_at_path_no_claude_dir_returns_not_applicable");
        let test_env = TestEnv::new();

        // Verify .claude doesn't exist
        assert!(
            !test_env.claude_dir().exists(),
            ".claude should not exist initially"
        );

        // Action: call install_claude_code_hook_at_path
        let result = install_claude_code_hook_at_path(&test_env.settings_path(), false);
        eprintln!("  Result: {:?}", result);

        // Assert: returns NotApplicable (not error)
        assert!(result.is_ok(), "Should not error");
        let outcome = result.unwrap();
        assert!(
            matches!(outcome, IdempotentResult::NotApplicable(_)),
            "Expected NotApplicable, got: {:?}",
            outcome
        );

        // Assert: ~/.claude was NOT created
        assert!(
            !test_env.claude_dir().exists(),
            ".claude should NOT be created"
        );

        eprintln!("TEST PASS: test_install_hook_at_path_no_claude_dir_returns_not_applicable");
    }

    /// When .claude exists but settings.json doesn't, hook should be installed.
    #[test]
    fn test_install_hook_at_path_claude_dir_exists_no_settings() {
        eprintln!("TEST START: test_install_hook_at_path_claude_dir_exists_no_settings");
        let test_env = TestEnv::new();

        // Setup: create .claude directory (user uses Claude Code)
        test_env.create_claude_dir();
        assert!(!test_env.settings_path().exists());

        // Action: install hook
        let result = install_claude_code_hook_at_path(&test_env.settings_path(), false);
        eprintln!("  Result: {:?}", result);

        // Assert: returns Changed
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(
            matches!(outcome, IdempotentResult::Changed),
            "Expected Changed, got: {:?}",
            outcome
        );

        // Assert: settings.json created with hook
        assert!(test_env.settings_path().exists());
        let settings = test_env.read_settings_json();
        let hooks = settings["hooks"]["PreToolUse"].as_array();
        assert!(hooks.is_some(), "PreToolUse should exist");
        let has_rch = hooks.unwrap().iter().any(|h| {
            h.get("hooks")
                .and_then(|h| h.as_array())
                .map(|arr| {
                    arr.iter().any(|entry| {
                        entry
                            .get("command")
                            .and_then(|c| c.as_str())
                            .map(is_rch_hook_command)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        });
        assert!(has_rch, "RCH hook should be present");

        eprintln!("TEST PASS: test_install_hook_at_path_claude_dir_exists_no_settings");
    }

    /// When hook is already installed, return Unchanged.
    #[test]
    fn test_install_hook_at_path_already_installed() {
        eprintln!("TEST START: test_install_hook_at_path_already_installed");
        let test_env = TestEnv::new();

        // Setup: create .claude directory and settings with RCH hook
        test_env.create_claude_dir();
        test_env.write_settings_json(&serde_json::json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            {
                                "type": "command",
                                "command": "rch"
                            }
                        ]
                    }
                ]
            }
        }));

        // Action: install hook (should detect already installed)
        let result = install_claude_code_hook_at_path(&test_env.settings_path(), false);
        eprintln!("  Result: {:?}", result);

        // Assert: returns Unchanged
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(
            matches!(outcome, IdempotentResult::Unchanged),
            "Expected Unchanged, got: {:?}",
            outcome
        );

        // Assert: file unchanged (still only one hook)
        let settings = test_env.read_settings_json();
        let hooks = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(hooks.len(), 1, "Should still have exactly one hook");

        eprintln!("TEST PASS: test_install_hook_at_path_already_installed");
    }

    /// When other hooks exist, RCH hook should be added while preserving others.
    #[test]
    fn test_install_hook_at_path_preserves_existing_hooks() {
        eprintln!("TEST START: test_install_hook_at_path_preserves_existing_hooks");
        let test_env = TestEnv::new();

        // Setup: settings.json with other hooks (e.g., DCG)
        test_env.create_claude_dir();
        test_env.write_settings_json(&serde_json::json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            {
                                "type": "command",
                                "command": "dcg"
                            }
                        ]
                    }
                ]
            },
            "other_setting": "preserved"
        }));

        // Action: install RCH hook
        let result = install_claude_code_hook_at_path(&test_env.settings_path(), false);
        eprintln!("  Result: {:?}", result);

        // Assert: returns Changed
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(
            matches!(outcome, IdempotentResult::Changed),
            "Expected Changed, got: {:?}",
            outcome
        );

        // Assert: DCG hook still present
        let settings = test_env.read_settings_json();
        let hooks = settings["hooks"]["PreToolUse"].as_array().unwrap();
        let has_dcg = hooks.iter().any(|h| {
            h.get("hooks")
                .and_then(|arr| arr.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|e| e.get("command").and_then(|c| c.as_str()) == Some("dcg"))
                })
                .unwrap_or(false)
        });
        assert!(has_dcg, "DCG hook should still be present");

        // Assert: RCH hook added
        let has_rch = hooks.iter().any(|h| {
            h.get("hooks")
                .and_then(|arr| arr.as_array())
                .map(|arr| {
                    arr.iter().any(|e| {
                        e.get("command")
                            .and_then(|c| c.as_str())
                            .map(is_rch_hook_command)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        });
        assert!(has_rch, "RCH hook should be added");

        // Assert: other settings preserved
        assert_eq!(
            settings["other_setting"], "preserved",
            "Other settings should be preserved"
        );

        eprintln!("TEST PASS: test_install_hook_at_path_preserves_existing_hooks");
    }

    /// Malformed settings must remain byte-for-byte unchanged.
    #[test]
    fn test_install_hook_at_path_malformed_json_preserved() {
        eprintln!("TEST START: test_install_hook_at_path_malformed_json_recovers");
        let test_env = TestEnv::new();

        // Setup: settings.json with invalid JSON
        test_env.create_claude_dir();
        std::fs::write(test_env.settings_path(), "{ invalid json }").unwrap();

        // Action: install hook
        let result = install_claude_code_hook_at_path(&test_env.settings_path(), false);
        eprintln!("  Result: {:?}", result);

        assert!(
            result.is_err(),
            "Malformed settings must not be overwritten"
        );
        let content = std::fs::read_to_string(test_env.settings_path()).unwrap();
        assert_eq!(content, "{ invalid json }");

        eprintln!("TEST PASS: test_install_hook_at_path_malformed_json_recovers");
    }

    /// Dry run should return WouldChange without modifying anything.
    #[test]
    fn test_install_hook_at_path_dry_run_no_changes() {
        eprintln!("TEST START: test_install_hook_at_path_dry_run_no_changes");
        let test_env = TestEnv::new();

        // Setup: .claude directory exists but no settings
        test_env.create_claude_dir();
        assert!(!test_env.settings_path().exists());

        // Action: install hook with dry_run = true
        let result = install_claude_code_hook_at_path(&test_env.settings_path(), true);
        eprintln!("  Result: {:?}", result);

        // Assert: returns WouldChange
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(
            matches!(outcome, IdempotentResult::WouldChange(_)),
            "Expected WouldChange, got: {:?}",
            outcome
        );

        // Assert: settings.json NOT created
        assert!(
            !test_env.settings_path().exists(),
            "settings.json should NOT be created in dry run"
        );

        eprintln!("TEST PASS: test_install_hook_at_path_dry_run_no_changes");
    }

    /// Check hook status on non-existent settings file returns NotInstalled.
    #[test]
    fn test_check_hook_at_path_not_exists() {
        eprintln!("TEST START: test_check_hook_at_path_not_exists");
        let test_env = TestEnv::new();
        test_env.create_claude_dir();

        let result = check_claude_code_hook_at_path(&test_env.settings_path());
        eprintln!("  Result: {:?}", result);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), HookStatus::NotInstalled);

        eprintln!("TEST PASS: test_check_hook_at_path_not_exists");
    }

    /// Check hook status when installed returns Installed.
    #[test]
    fn test_check_hook_at_path_installed() {
        eprintln!("TEST START: test_check_hook_at_path_installed");
        let test_env = TestEnv::new();
        test_env.create_claude_dir();
        test_env.write_settings_json(&serde_json::json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            { "type": "command", "command": "rch" }
                        ]
                    }
                ]
            }
        }));

        let result = check_claude_code_hook_at_path(&test_env.settings_path());
        eprintln!("  Result: {:?}", result);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), HookStatus::Installed);

        eprintln!("TEST PASS: test_check_hook_at_path_installed");
    }

    /// Check hook status with obsolete format returns NeedsUpdate.
    #[test]
    fn test_check_hook_at_path_needs_update() {
        eprintln!("TEST START: test_check_hook_at_path_needs_update");
        let test_env = TestEnv::new();
        test_env.create_claude_dir();
        // Obsolete format: command at top level without matcher
        test_env.write_settings_json(&serde_json::json!({
            "hooks": {
                "PreToolUse": [
                    { "command": "rch", "description": "Old format" }
                ]
            }
        }));

        let result = check_claude_code_hook_at_path(&test_env.settings_path());
        eprintln!("  Result: {:?}", result);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), HookStatus::NeedsUpdate);

        eprintln!("TEST PASS: test_check_hook_at_path_needs_update");
    }
}
