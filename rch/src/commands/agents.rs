//! Agent management commands.
//!
//! This module contains commands for listing, inspecting, and managing
//! AI coding agent integrations (Claude Code, Cursor, etc.) and their RCH hooks.

use anyhow::Result;
use rch_common::ApiResponse;

use crate::ui::context::OutputContext;
use crate::ui::theme::StatusIndicator;

// =============================================================================
// Agents Commands
// =============================================================================

/// Preview/apply an explicit migration of exactly owned legacy handlers.
pub fn migrate(agent: &str, dry_run: bool, ctx: &OutputContext) -> Result<()> {
    migration_command(agent, dry_run, false, ctx)
}

/// Restore only migration-owned handlers, merging with current configuration.
pub fn rollback(agent: &str, dry_run: bool, ctx: &OutputContext) -> Result<()> {
    migration_command(agent, dry_run, true, ctx)
}

fn migration_command(
    agent: &str,
    dry_run: bool,
    rollback: bool,
    ctx: &OutputContext,
) -> Result<()> {
    use crate::agent::{AgentKind, migrate_hook, rollback_hook};
    let kind =
        AgentKind::from_id(agent).ok_or_else(|| anyhow::anyhow!("Unknown agent '{agent}'"))?;
    let action = if rollback { "rollback" } else { "migrate" };
    let result = if rollback {
        rollback_hook(kind, dry_run)?
    } else {
        migrate_hook(kind, dry_run)?
    };
    if ctx.is_json() {
        let _ = ctx.json(&ApiResponse::ok(
            format!("agents {action}"),
            serde_json::json!({
                "agent": kind.id(), "dry_run": dry_run, "result": format!("{result:?}"),
                "restart_performed": false, "path_changed": false,
            }),
        ));
    } else {
        println!("{} {action}: {result:?}", kind.name());
        if kind == AgentKind::Hermes {
            eprintln!(
                "Restart Hermes after applying migration or rollback; loaded callbacks are unchanged until restart."
            );
        }
    }
    Ok(())
}

/// List detected AI coding agents and their RCH hook status.
pub fn agents_list(all: bool, ctx: &OutputContext) -> Result<()> {
    use crate::agent::{AgentKind, HookStatus, check_hook_status, detect_agents};

    let style = ctx.theme();

    // Detect all installed agents
    let detected = detect_agents()?;

    // Build agent info list
    let mut agent_infos: Vec<serde_json::Value> = Vec::new();

    // If --all flag, show all known agents; otherwise only detected ones
    let agents_to_show: Vec<AgentKind> = if all {
        AgentKind::ALL.to_vec()
    } else {
        detected.iter().map(|a| a.kind).collect()
    };

    for kind in &agents_to_show {
        let detected_info = detected.iter().find(|a| a.kind == *kind);
        let is_detected = detected_info.is_some();
        let hook_status = check_hook_status(*kind).unwrap_or(HookStatus::NotSupported);

        let info = serde_json::json!({
            "name": kind.name(),
            "kind": format!("{:?}", kind),
            "detected": is_detected,
            "version": detected_info.and_then(|d| d.version.clone()),
            "config_path": detected_info.and_then(|d| d.config_path.as_ref().map(|p| p.display().to_string())),
            "hook_support": format!("{}", kind.hook_support()),
            "hook_status": hook_status.to_string(),
            "can_install_hook": kind.hook_support().can_install_hook(),
        });
        agent_infos.push(info);
    }

    if ctx.is_json() {
        let _ = ctx.json(&ApiResponse::ok(
            "agents list",
            serde_json::json!({
                "agents": agent_infos,
                "total": agent_infos.len(),
                "detected": detected.len(),
            }),
        ));
        return Ok(());
    }

    // Human-readable output
    println!("{}", style.format_header("Detected AI Coding Agents"));
    println!();

    if agents_to_show.is_empty() {
        println!("  {} No AI coding agents detected.", style.muted("ℹ"));
        println!();
        println!("  Run with --all to see all known agents.");
        return Ok(());
    }

    for kind in &agents_to_show {
        let detected_info = detected.iter().find(|a| a.kind == *kind);
        let is_detected = detected_info.is_some();
        let hook_status = check_hook_status(*kind).unwrap_or(HookStatus::NotSupported);

        let status_indicator = if is_detected {
            match hook_status {
                HookStatus::Installed => StatusIndicator::Success,
                HookStatus::NeedsUpdate => StatusIndicator::Warning,
                HookStatus::NotInstalled => StatusIndicator::Info,
                HookStatus::NotSupported => StatusIndicator::Pending,
            }
        } else {
            StatusIndicator::Pending
        };

        let detected_label = if is_detected {
            style.success("detected")
        } else {
            style.muted("not found")
        };

        println!(
            "  {} {} {}",
            status_indicator.display(style),
            style.key(kind.name()),
            detected_label
        );

        if let Some(info) = detected_info {
            if let Some(ref version) = info.version {
                println!("      {} {}", style.muted("Version:"), style.value(version));
            }
            if let Some(ref path) = info.config_path {
                println!(
                    "      {} {}",
                    style.muted("Config:"),
                    style.value(&path.display().to_string())
                );
            }
        }

        println!(
            "      {} {}",
            style.muted("Hook:"),
            match hook_status {
                HookStatus::Installed => style.success(&hook_status.to_string()),
                HookStatus::NeedsUpdate => style.warning(&hook_status.to_string()),
                HookStatus::NotInstalled => style.value(&hook_status.to_string()),
                HookStatus::NotSupported => style.muted(&hook_status.to_string()),
            }
        );
        println!();
    }

    Ok(())
}

/// Show detailed status for a specific agent or all detected agents.
pub fn agents_status(agent: Option<String>, ctx: &OutputContext) -> Result<()> {
    use crate::agent::{AgentKind, HookStatus, check_hook_status, detect_single_agent};

    let style = ctx.theme();

    // If specific agent requested, show only that one
    if let Some(ref agent_name) = agent {
        let kind = AgentKind::from_id(agent_name).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown agent: {}. Valid agents: {:?}",
                agent_name,
                AgentKind::ALL.iter().map(|k| k.id()).collect::<Vec<_>>()
            )
        })?;

        let detected = detect_single_agent(kind)?;
        let hook_status = check_hook_status(kind).unwrap_or(HookStatus::NotSupported);

        let info = serde_json::json!({
            "name": kind.name(),
            "kind": format!("{:?}", kind),
            "detected": detected.is_some(),
            "version": detected.as_ref().and_then(|d| d.version.clone()),
            "config_path": detected.as_ref().and_then(|d| d.config_path.as_ref().map(|p| p.display().to_string())),
            "detection_method": detected.as_ref().map(|d| format!("{:?}", d.detection_method)),
            "hook_support": format!("{}", kind.hook_support()),
            "hook_status": hook_status.to_string(),
            "can_install_hook": kind.hook_support().can_install_hook(),
        });

        if ctx.is_json() {
            let _ = ctx.json(&ApiResponse::ok("agents status", info));
            return Ok(());
        }

        println!(
            "{}",
            style.format_header(&format!("Agent: {}", kind.name()))
        );
        println!();
        println!(
            "  {} {}",
            style.key("Detected:"),
            if detected.is_some() {
                style.success("Yes")
            } else {
                style.warning("No")
            }
        );
        if let Some(ref d) = detected {
            if let Some(ref version) = d.version {
                println!("  {} {}", style.key("Version:"), style.value(version));
            }
            if let Some(ref path) = d.config_path {
                println!(
                    "  {} {}",
                    style.key("Config:"),
                    style.value(&path.display().to_string())
                );
            }
            println!("  {} {:?}", style.key("Detection:"), d.detection_method);
        }
        println!(
            "  {} {}",
            style.key("Hook Support:"),
            style.value(&format!("{}", kind.hook_support()))
        );
        println!(
            "  {} {}",
            style.key("Hook Status:"),
            match hook_status {
                HookStatus::Installed => style.success(&hook_status.to_string()),
                HookStatus::NeedsUpdate => style.warning(&hook_status.to_string()),
                HookStatus::NotInstalled => style.value(&hook_status.to_string()),
                HookStatus::NotSupported => style.muted(&hook_status.to_string()),
            }
        );

        return Ok(());
    }

    // No specific agent - show all detected agents
    agents_list(false, ctx)
}

/// Install the RCH hook for a specified AI coding agent.
pub fn agents_install_hook(agent: &str, dry_run: bool, ctx: &OutputContext) -> Result<()> {
    use crate::agent::{AgentKind, install_hook};
    use crate::state::primitives::IdempotentResult;

    let style = ctx.theme();

    // Parse the agent name
    let kind = AgentKind::from_id(agent).ok_or_else(|| {
        anyhow::anyhow!(
            "Unknown agent '{}'. Supported agents: {}",
            agent,
            AgentKind::ALL
                .iter()
                .map(|k| k.id())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    if !ctx.is_json() {
        if dry_run {
            println!("Dry run: checking hook installation for {}...", kind.name());
        } else {
            println!("Installing RCH hook for {}...", kind.name());
        }
    }

    let result = install_hook(kind, dry_run)?;

    if ctx.is_json() {
        let _ = ctx.json(&serde_json::json!({
            "agent": kind.id(),
            "action": "install",
            "result": format!("{:?}", result),
            "dry_run": dry_run
        }));
    } else {
        match result {
            IdempotentResult::Changed => {
                println!(
                    "{} RCH hook installed for {}",
                    StatusIndicator::Success.display(style),
                    kind.name()
                );
            }
            IdempotentResult::Unchanged => {
                println!(
                    "{} RCH hook already installed for {}",
                    StatusIndicator::Success.display(style),
                    kind.name()
                );
            }
            IdempotentResult::WouldChange(msg) => {
                println!("{} {}", StatusIndicator::Info.display(style), msg);
            }
            IdempotentResult::NotApplicable(msg) => {
                println!("{} {}", StatusIndicator::Warning.display(style), msg);
            }
            // Other variants (Created, AlreadyExists, Updated, DryRun) not returned by hook install
            other => {
                println!(
                    "{} Hook installation result: {:?}",
                    StatusIndicator::Info.display(style),
                    other
                );
            }
        }
    }

    Ok(())
}

/// Uninstall the RCH hook from a specified AI coding agent.
pub fn agents_uninstall_hook(agent: &str, dry_run: bool, ctx: &OutputContext) -> Result<()> {
    use crate::agent::{AgentKind, uninstall_hook};
    use crate::state::primitives::IdempotentResult;

    let style = ctx.theme();

    // Parse the agent name
    let kind = AgentKind::from_id(agent).ok_or_else(|| {
        anyhow::anyhow!(
            "Unknown agent '{}'. Supported agents: {}",
            agent,
            AgentKind::ALL
                .iter()
                .map(|k| k.id())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    if !ctx.is_json() {
        if dry_run {
            println!("Dry run: checking hook removal for {}...", kind.name());
        } else {
            println!("Uninstalling RCH hook from {}...", kind.name());
        }
    }

    let result = uninstall_hook(kind, dry_run)?;

    if ctx.is_json() {
        let _ = ctx.json(&serde_json::json!({
            "agent": kind.id(),
            "action": "uninstall",
            "result": format!("{:?}", result),
            "dry_run": dry_run
        }));
    } else {
        match result {
            IdempotentResult::Changed => {
                println!(
                    "{} RCH hook removed from {}",
                    StatusIndicator::Success.display(style),
                    kind.name()
                );
            }
            IdempotentResult::Unchanged => {
                println!(
                    "{} RCH hook was not installed for {}",
                    StatusIndicator::Info.display(style),
                    kind.name()
                );
            }
            IdempotentResult::WouldChange(msg) => {
                println!("{} {}", StatusIndicator::Info.display(style), msg);
            }
            IdempotentResult::NotApplicable(msg) => {
                println!("{} {}", StatusIndicator::Warning.display(style), msg);
            }
            // Other variants (Created, AlreadyExists, Updated, DryRun) not returned by hook uninstall
            other => {
                println!(
                    "{} Hook uninstallation result: {:?}",
                    StatusIndicator::Info.display(style),
                    other
                );
            }
        }
    }

    Ok(())
}
