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
/// helper. Native compiler routing should run before this slower general path.
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
    if !is_candidate(command) || command.contains('\0') {
        return Ok(None);
    }
    let request = serde_json::to_vec(&serde_json::json!({ "command": command }))?;
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
        apply_plan(command, &output)
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
        let sentinel = fixture.join("python3");
        std::fs::write(
            &sentinel,
            "#!/bin/sh\n\
             test \"$RCH_SHIM_BYPASS\" = 1 || exit 81\n\
             test \"$RCH_CARGO_WRAPPER_BYPASS\" = 1 || exit 82\n\
             test \"$1\" = -I || exit 83\n\
             test \"$2\" = -c || exit 84\n\
             exec /usr/bin/python3 \"$@\"\n",
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
