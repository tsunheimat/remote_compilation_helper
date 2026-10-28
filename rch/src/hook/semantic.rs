//! Bounded, isolated semantic shell planning for shared-workspace verification.
//!
//! The embedded stdlib helper only discovers executable-word spans. Rust owns
//! every inserted byte; neither side executes shell input or grants permission.

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

const HELPER: &str = include_str!("semantic.py");
const MAX_COMMAND_BYTES: usize = 65_536;
const MAX_OUTPUT_BYTES: usize = 524_288;
const MAX_STDERR_BYTES: usize = 8_192;
const PLAN_TIMEOUT: Duration = Duration::from_millis(750);

/// Cheap prefilter only. False positives are harmless; semantics live in the
/// helper. Bare native compiler commands retain their fast path; compound
/// commands must be planned as a whole before selecting each stage's backend.
pub(super) fn is_candidate(command: &str) -> bool {
    command.len() <= MAX_COMMAND_BYTES
        && [
            "go",
            "cargo",
            "swift",
            "pytest",
            "python",
            "uv",
            "npm",
            "pnpm",
            "bun",
            "node",
            "make",
            "npx",
            "vitest",
            "jest",
            "tsc",
            "eslint",
            "prettier",
            "playwright",
            "vite",
            "run_tests.sh",
        ]
        .iter()
        .any(|word| command.contains(word))
}

/// A conservative lexical trigger, not a shell parser. Quoted metacharacters
/// may request planning too; only the bounded structural helper decides spans.
pub(super) fn needs_structure(command: &str) -> bool {
    command.bytes().any(|byte| b";&|()<>\n\r".contains(&byte))
        && (is_candidate(command)
            || [
                "rustc", "gcc", "g++", "clang", "cmake", "ninja", "meson", "nix",
            ]
            .iter()
            .any(|word| command.contains(word)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookPlan {
    version: u8,
    insertions: Vec<HookInsertion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookInsertion {
    start: usize,
    end: usize,
    command_end: usize,
    word: String,
    argv: Vec<ArgumentSpan>,
    env: Vec<String>,
    verification: bool,
    native_eligible: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArgumentSpan {
    start: usize,
    end: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u8,
    insertions: Vec<Insertion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Insertion {
    start: usize,
    end: usize,
    word: String,
    env: Vec<String>,
}

fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn apply_plan(command: &str, output: &[u8]) -> Result<Option<String>> {
    let plan: Plan = serde_json::from_slice(output).context("invalid semantic planner response")?;
    ensure!(plan.version == 1, "unknown semantic planner version");
    ensure!(plan.insertions.len() <= 4096, "too many semantic spans");
    if plan.insertions.is_empty() {
        return Ok(None);
    }
    let mut previous_end = 0;
    for edit in &plan.insertions {
        ensure!(
            edit.start >= previous_end && edit.start < edit.end,
            "unordered or overlapping semantic spans"
        );
        ensure!(
            command.get(edit.start..edit.end) == Some(edit.word.as_str()),
            "semantic span does not match authored executable"
        );
        ensure!(
            edit.env.len() <= 4096 && edit.env.iter().all(|name| valid_env_name(name)),
            "invalid forwarded environment name"
        );
        previous_end = edit.end;
    }
    let mut rewritten = String::with_capacity(command.len() + plan.insertions.len() * 32);
    let mut cursor = 0;
    for edit in plan.insertions {
        rewritten.push_str(&command[cursor..edit.start]);
        rewritten.push_str("rch exec --verification");
        for name in edit.env {
            rewritten.push_str(" --forward-env ");
            rewritten.push_str(&name);
        }
        rewritten.push_str(" -- ");
        // Zero-width insertion preserves all authored quoting, redirections,
        // expansion order and operators, including redirects before the tool.
        cursor = edit.start;
    }
    rewritten.push_str(&command[cursor..]);
    Ok(Some(rewritten))
}

fn apply_hook_plan(
    command: &str,
    output: &[u8],
    config: &rch_common::RchConfig,
) -> Result<Option<String>> {
    let plan: HookPlan =
        serde_json::from_slice(output).context("invalid structural hook response")?;
    ensure!(plan.version == 2, "unknown structural hook version");
    ensure!(
        plan.insertions.len() <= 4096,
        "too many structural hook spans"
    );
    // Validate the entire response before accepting any partial rewrite.
    let mut previous_end = 0;
    for edit in &plan.insertions {
        ensure!(
            edit.start >= previous_end && edit.start < edit.end && edit.end <= edit.command_end,
            "unordered or overlapping hook components"
        );
        ensure!(
            command.get(edit.start..edit.end) == Some(edit.word.as_str())
                && command.get(edit.start..edit.command_end).is_some(),
            "hook component does not match authored command"
        );
        ensure!(
            edit.env.len() <= 4096 && edit.env.iter().all(|name| valid_env_name(name)),
            "invalid hook environment name"
        );
        ensure!(
            !edit.native_eligible || edit.env.is_empty(),
            "native hook component cannot discard forwarded environment"
        );
        ensure!(
            !edit.argv.is_empty()
                && edit.argv.len() <= 4096
                && edit.argv[0].start == edit.start
                && edit.argv[0].end == edit.end,
            "hook argv must start at the authored executable"
        );
        let mut argument_end = edit.start;
        for argument in &edit.argv {
            ensure!(
                argument.start >= argument_end
                    && argument.start < argument.end
                    && argument.end <= edit.command_end
                    && command.get(argument.start..argument.end).is_some(),
                "invalid hook argument span"
            );
            argument_end = argument.end;
        }
        ensure!(
            argument_end == edit.command_end,
            "incomplete hook argument spans"
        );
        previous_end = edit.command_end;
    }
    if !config.general.enabled || config.general.force_local {
        return Ok(None);
    }
    let threshold = if config.general.force_remote {
        0.0
    } else {
        config.compilation.confidence_threshold
    };
    let mut rewritten = String::with_capacity(command.len());
    let mut cursor = 0;
    let mut changed = false;
    for edit in plan.insertions {
        // Redirections can occur BETWEEN words (`cargo >log test`). Classify
        // only the validated authored argv spans so their position cannot
        // bypass native allowlist/confidence policy. This joined string is
        // NEVER executed: the rewrite below still preserves all original bytes.
        let component = edit
            .argv
            .iter()
            .map(|argument| &command[argument.start..argument.end])
            .collect::<Vec<_>>()
            .join(" ");
        let native = super::classify_command(&component);
        // A generic fallback must not turn an explicitly excluded native
        // compilation into a shared-workspace bypass of the same policy.
        if native.is_compilation
            && (native.confidence < threshold
                || native
                    .kind
                    .is_some_and(|kind| !config.execution.is_allowed(kind.command_base())))
        {
            continue;
        }
        let use_native = edit.native_eligible
            && native.is_compilation
            && native.command_prefix.is_none()
            && native.extracted_command.is_none();
        if !use_native && !edit.verification && !native.is_compilation {
            continue;
        }
        rewritten.push_str(&command[cursor..edit.start]);
        rewritten.push_str(if use_native {
            "rch exec"
        } else {
            "rch exec --verification"
        });
        for name in edit.env {
            rewritten.push_str(" --forward-env ");
            rewritten.push_str(&name);
        }
        rewritten.push_str(" -- ");
        cursor = edit.start;
        changed = true;
    }
    if !changed {
        return Ok(None);
    }
    rewritten.push_str(&command[cursor..]);
    Ok(Some(rewritten))
}

fn helper_process() -> Command {
    let mut process = Command::new("python3");
    process
        .args(["-I", "-c", HELPER])
        // PATH may resolve python3 to RCH's generic tool shim. Only this
        // parser subprocess bypasses interception; workload children retain
        // normal shim behavior and the parent environment is never mutated.
        .env("RCH_SHIM_BYPASS", "1")
        .env("RCH_CARGO_WRAPPER_BYPASS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    process
}

/// Return an authored-shell rewrite, or `None` on a semantic/grammar miss.
/// Operational failures are errors for the caller to log and fail open.
pub(super) async fn plan(command: &str) -> Result<Option<String>> {
    request_plan(command, None).await
}

/// Select a backend per structurally parsed component. Used only for agent
/// hook compounds; generic shim classification keeps its existing contract.
pub(super) async fn plan_for_hook(
    command: &str,
    config: &rch_common::RchConfig,
) -> Result<Option<String>> {
    request_plan(command, Some(config)).await
}

async fn request_plan(
    command: &str,
    hook_config: Option<&rch_common::RchConfig>,
) -> Result<Option<String>> {
    if command.len() > MAX_COMMAND_BYTES
        || command.contains('\0')
        || (hook_config.is_none() && !is_candidate(command))
    {
        return Ok(None);
    }
    let request = if hook_config.is_some() {
        serde_json::json!({ "command": command, "mode": "hook" })
    } else {
        serde_json::json!({ "command": command })
    };
    let request = serde_json::to_vec(&request)?;
    let mut child = helper_process()
        .spawn()
        .context("start isolated semantic planner")?;
    let mut stdin = child.stdin.take().context("planner stdin unavailable")?;
    let stdout = child.stdout.take().context("planner stdout unavailable")?;
    let stderr = child.stderr.take().context("planner stderr unavailable")?;
    let result = tokio::time::timeout(PLAN_TIMEOUT, async {
        let write = async {
            stdin.write_all(&request).await?;
            // EOF terminates the helper's bounded read. Explicitly close the
            // pipe rather than relying on AsyncWrite shutdown semantics.
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let read_stdout = async {
            let mut bytes = Vec::new();
            stdout
                .take((MAX_OUTPUT_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(
                bytes.len() <= MAX_OUTPUT_BYTES,
                "semantic planner stdout limit"
            );
            Ok::<_, anyhow::Error>(bytes)
        };
        let read_stderr = async {
            let mut bytes = Vec::new();
            stderr
                .take((MAX_STDERR_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(
                bytes.len() <= MAX_STDERR_BYTES,
                "semantic planner stderr limit"
            );
            Ok::<_, anyhow::Error>(bytes)
        };
        let ((), output, diagnostics) = tokio::try_join!(
            async { write.await.map_err(anyhow::Error::from) },
            read_stdout,
            read_stderr
        )?;
        let status = child.wait().await?;
        if !status.success() {
            bail!(
                "semantic planner exited {status}: {}",
                String::from_utf8_lossy(&diagnostics)
            );
        }
        match hook_config {
            Some(config) => apply_hook_plan(command, &output, config),
            None => apply_plan(command, &output),
        }
    })
    .await;
    match result {
        Ok(Ok(plan)) => Ok(plan),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            Err(error)
        }
        Err(error) => {
            let _ = child.kill().await;
            Err(error).context("semantic planner timed out")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn structural_hook_selects_each_backend_independently_of_stage_order() {
        let config = rch_common::RchConfig::default();
        for (source, expected) in [
            (
                "pytest -q && cargo test",
                "rch exec --verification -- pytest -q && rch exec -- cargo test",
            ),
            (
                "cargo test && pytest -q",
                "rch exec -- cargo test && rch exec --verification -- pytest -q",
            ),
            (
                "cd 'web dir' && pytest -q && cargo check",
                "cd 'web dir' && rch exec --verification -- pytest -q && rch exec -- cargo check",
            ),
            (
                "cargo check && cargo test",
                "rch exec -- cargo check && rch exec -- cargo test",
            ),
            (
                "pytest || node --import setup.mjs --test t.cjs",
                "rch exec --verification -- pytest || rch exec -- node --import setup.mjs --test t.cjs",
            ),
            (
                "rtk run 'pytest && cargo test' && cargo check",
                "rch exec --verification -- rtk run 'pytest && cargo test' && rch exec -- cargo check",
            ),
            (
                "cargo test | tee log",
                "rch exec --verification -- cargo test | tee log",
            ),
            (
                "X='a b' cargo test && pytest",
                "X='a b' rch exec --verification --forward-env X -- cargo test && rch exec --verification -- pytest",
            ),
        ] {
            assert_eq!(
                plan_for_hook(source, &config).await.unwrap().as_deref(),
                Some(expected),
                "{source}"
            );
        }
        for source in [
            "eval 'pytest' && cargo test",
            "if true; then pytest; fi && cargo test",
            "export X=1 && pytest; cargo test",
        ] {
            assert!(
                plan_for_hook(source, &config).await.unwrap().is_none(),
                "{source}"
            );
        }
        assert!(!needs_structure("cargo test"));
        assert!(!needs_structure("git status && git diff"));
        assert!(needs_structure("pytest && cargo test"));
    }

    #[tokio::test]
    async fn structural_hook_preserves_native_policy_without_shared_bypass() {
        let mut config = rch_common::RchConfig::default();
        config.execution.allowlist.retain(|tool| tool != "cargo");
        assert_eq!(
            plan_for_hook("pytest && cargo test", &config)
                .await
                .unwrap()
                .as_deref(),
            Some("rch exec --verification -- pytest && cargo test")
        );
        assert!(
            plan_for_hook("cargo test | tee log", &config)
                .await
                .unwrap()
                .is_none()
        );
        for source in ["cargo >log test && pytest", "cargo 2>errors test && pytest"] {
            assert_eq!(
                plan_for_hook(source, &config).await.unwrap().as_deref(),
                Some(
                    source
                        .replace("&& pytest", "&& rch exec --verification -- pytest")
                        .as_str()
                ),
                "{source}"
            );
        }
        config.general.force_local = true;
        assert!(
            plan_for_hook("pytest && cargo test", &config)
                .await
                .unwrap()
                .is_none()
        );
        config.general.force_local = false;
        config.general.enabled = false;
        assert!(
            plan_for_hook("pytest && cargo test", &config)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn structural_hook_rejects_invalid_full_plan_before_any_rewrite() {
        let source = "pytest && cargo test";
        let config = rch_common::RchConfig::default();
        let valid = serde_json::json!({"version":2,"insertions":[
            {"start":0,"end":6,"command_end":6,"word":"pytest","argv":[{"start":0,"end":6}],"env":[],"verification":true,"native_eligible":true},
            {"start":10,"end":15,"command_end":20,"word":"cargo","argv":[{"start":10,"end":15},{"start":16,"end":20}],"env":[],"verification":true,"native_eligible":true}
        ]});
        assert_eq!(
            apply_hook_plan(source, &serde_json::to_vec(&valid).unwrap(), &config)
                .unwrap()
                .as_deref(),
            Some("rch exec --verification -- pytest && rch exec -- cargo test")
        );
        let mut wrong_version = valid.clone();
        wrong_version["version"] = 1.into();
        let mut out_of_bounds = valid.clone();
        out_of_bounds["insertions"][1]["command_end"] = 999.into();
        let mut overlap = valid.clone();
        overlap["insertions"][1]["start"] = 1.into();
        let mut unsafe_env = valid.clone();
        unsafe_env["insertions"][1]["env"] = serde_json::json!(["X;id"]);
        let mut dropped_env = valid.clone();
        dropped_env["insertions"][1]["env"] = serde_json::json!(["CI"]);
        let mut overlapping_argv = valid.clone();
        overlapping_argv["insertions"][1]["argv"][1]["start"] = 14.into();
        let mut missing_argv = valid.clone();
        missing_argv["insertions"][1]["argv"] = serde_json::json!([]);
        let mut shortened_argv = valid.clone();
        shortened_argv["insertions"][1]["argv"][1]["end"] = 19.into();
        let mut forged_word = valid;
        forged_word["insertions"][1]["word"] = "pytest".into();
        for invalid in [
            wrong_version,
            out_of_bounds,
            overlap,
            unsafe_env,
            dropped_env,
            overlapping_argv,
            missing_argv,
            shortened_argv,
            forged_word,
        ] {
            assert!(
                apply_hook_plan(source, &serde_json::to_vec(&invalid).unwrap(), &config).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn validates_schema_and_utf8_spans() {
        let command = "printf é; pytest -q";
        let output =
            br#"{"version":1,"insertions":[{"start":11,"end":17,"word":"pytest","env":[]}]}"#;
        assert_eq!(
            apply_plan(command, output).unwrap().unwrap(),
            "printf é; rch exec --verification -- pytest -q"
        );
        for output in [
            r#"{"version":2,"insertions":[]}"#,
            r#"{"version":1,"insertions":[],"grant":true}"#,
            r#"{"version":1,"insertions":[{"start":8,"end":17,"word":"pytest","env":[]}]}"#,
            r#"{"version":1,"insertions":[{"start":11,"end":17,"word":"pytest","env":["X;id"]}]}"#,
            r#"{"version":1,"insertions":[{"start":11,"end":17,"word":"pytest","env":[]},{"start":11,"end":17,"word":"pytest","env":[]}]}"#,
        ] {
            assert!(apply_plan(command, output.as_bytes()).is_err(), "{output}");
        }
    }

    #[test]
    fn keeps_original_bytes_and_forwards_prefix_environment() {
        let output = br#"{"version":1,"insertions":[{"start":6,"end":12,"word":"pytest","env":["X"]},{"start":16,"end":18,"word":"go","env":[]}]}"#;
        assert_eq!(
            apply_plan("X='a' pytest && go test", output)
                .unwrap()
                .unwrap(),
            "X='a' rch exec --verification --forward-env X -- pytest && rch exec --verification -- go test"
        );
        assert!(!is_candidate("git status"));
        assert!(!is_candidate(&"x".repeat(MAX_COMMAND_BYTES + 1)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn planner_helper_bypasses_python_shim_only() {
        use std::os::unix::fs::PermissionsExt;

        // Retain the fixture: repository instructions forbid deleting files.
        let fixture = tempfile::Builder::new()
            .prefix("rch-semantic-shim-")
            .tempdir()
            .unwrap()
            .keep();
        // Resolve the actual PATH-selected interpreter before replacing PATH.
        // /usr/bin/python3 may be a platform developer-tool launcher rather
        // than that interpreter. Resolution is setup, not semantic planning;
        // the real helper below still has the unchanged PLAN_TIMEOUT budget.
        let interpreter = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new("python3")
                .args(["-I", "-S", "-c", "import sys; print(sys.executable)"])
                .env("RCH_SHIM_BYPASS", "1")
                .env("RCH_CARGO_WRAPPER_BYPASS", "1")
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(interpreter.status.success(), "{interpreter:?}");
        let interpreter = String::from_utf8(interpreter.stdout).unwrap();
        let interpreter = std::path::Path::new(interpreter.trim());
        assert!(interpreter.is_absolute() && interpreter.is_file());
        let interpreter = shell_escape::escape(interpreter.to_string_lossy());
        let sentinel = fixture.join("python3");
        std::fs::write(
            &sentinel,
            format!(
                "#!/bin/sh\n\
                 test \"$RCH_SHIM_BYPASS\" = 1 || exit 81\n\
                 test \"$RCH_CARGO_WRAPPER_BYPASS\" = 1 || exit 82\n\
                 test \"$1\" = -I || exit 83\n\
                 test \"$2\" = -c || exit 84\n\
                 exec {interpreter} \"$@\"\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o755)).unwrap();
        let shim_before = std::env::var_os("RCH_SHIM_BYPASS");
        let cargo_before = std::env::var_os("RCH_CARGO_WRAPPER_BYPASS");
        let mut process = helper_process();
        process.env("PATH", &fixture);
        let mut child = process.spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        input
            .write_all(br#"{"command":"go test ./..."}"#)
            .await
            .unwrap();
        drop(input);
        let output = tokio::time::timeout(PLAN_TIMEOUT, child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "shim sentinel failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            apply_plan("go test ./...", &output.stdout)
                .unwrap()
                .unwrap(),
            "rch exec --verification -- go test ./..."
        );
        assert_eq!(std::env::var_os("RCH_SHIM_BYPASS"), shim_before);
        assert_eq!(std::env::var_os("RCH_CARGO_WRAPPER_BYPASS"), cargo_before);
        // A separately constructed workload receives no bypass overrides.
        assert_eq!(Command::new("go").as_std().get_envs().count(), 0);
    }

    #[tokio::test]
    async fn embedded_helper_plans_without_sibling_imports() {
        assert_eq!(
            plan("cd pkg && python3 -m pytest -q 2>&1 | tee out")
                .await
                .unwrap()
                .unwrap(),
            "cd pkg && rch exec --verification -- python3 -m pytest -q 2>&1 | tee out"
        );
        assert!(plan("printf 'pytest'").await.unwrap().is_none());
    }
}
