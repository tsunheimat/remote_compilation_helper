//! Real subprocess checks for the terminal fallback gate. No compiler or worker
//! is launched: the workload is a short owned shell that records execution.

use super::*;
use std::process::{Command as ProcessCommand, Output as ProcessOutput};

const CASE: &str = "RCH_FALLBACK_TEST_CASE";
const REASON: &str = "RCH_FALLBACK_TEST_REASON";

#[test]
fn subprocess_entry() {
    let Ok(case) = std::env::var(CASE) else {
        return;
    };
    remote_result::set_machine_output(std::env::var("RCH_TEST_MACHINE").as_deref() != Ok("0"));
    let command = if case == "signal" {
        "printf run >> \"$RCH_FALLBACK_PROBE\"; kill -TERM $$".to_owned()
    } else {
        format!(
            "printf run >> \"$RCH_FALLBACK_PROBE\"; test \"${RCH_CARGO_WRAPPER_BYPASS_ENV}\" = 1 || exit 91; printf 'workload-out\\n'; printf 'workload-err\\n' >&2; exit 23"
        )
    };
    let reason = std::env::var(REASON).expect("fixture reason");
    exit_with_gated_local_fallback(
        &command,
        &HookReporter::new(OutputVisibility::None),
        &reason,
        std::env::var("RCH_TEST_REQUIRED").as_deref() == Ok("1"),
        std::env::var("RCH_TEST_ALLOWED").as_deref() == Ok("1"),
        Some("fixture remote failure"),
    );
}

fn run_case(
    case: &str,
    policy: (bool, bool),
    reason: &str,
    options: &[(&str, &str)],
) -> (tempfile::TempDir, ProcessOutput) {
    let root = tempfile::tempdir().expect("private fixture");
    let state = root.path().join("state");
    if case == "ledger-unavailable" {
        std::fs::write(&state, b"not a directory").unwrap();
    }
    let empty_path = root.path().join("empty-bin");
    std::fs::create_dir(&empty_path).unwrap();
    let mut child = ProcessCommand::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "hook::fallback_policy_tests::subprocess_entry",
            "--nocapture",
            "--quiet",
        ])
        .env_clear()
        .env(CASE, case)
        .env(REASON, reason)
        .env("RCH_TEST_REQUIRED", if policy.0 { "1" } else { "0" })
        .env("RCH_TEST_ALLOWED", if policy.1 { "1" } else { "0" })
        .env("RCH_STATE_HOME", &state)
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_CACHE_HOME", root.path().join("cache"))
        .env("RCH_FALLBACK_PROBE", root.path().join("executed"))
        .env(
            "PATH",
            if case == "spawn-failure" {
                empty_path.as_os_str()
            } else {
                std::ffi::OsStr::new("/usr/bin:/bin")
            },
        )
        .envs(options.iter().copied())
        .current_dir(root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch the exact fixture test");
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("fallback fixture timed out: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    (root, output)
}

fn envelopes(output: &ProcessOutput) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value.get("api_version").is_some())
        .collect()
}

#[test]
fn disabled_or_required_fallback_never_launches_and_emits_one_refusal() {
    for reason in [
        "daemon unavailable",
        "dependency preflight failed",
        "transfer skipped",
        "unrequested worker",
    ] {
        for policy in [(false, false), (true, false), (true, true)] {
            let (root, output) = run_case("refused", policy, reason, &[]);
            let expected = if policy.0 {
                EXIT_REMOTE_REQUIRED_REFUSED
            } else {
                EXIT_BUILD_ERROR
            };
            assert_eq!(output.status.code(), Some(expected), "{reason}: {output:?}");
            assert!(
                !root.path().join("executed").exists(),
                "refusal ran a workload"
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("refusing local fallback"));
            let result = envelopes(&output);
            assert_eq!(result.len(), 1, "{output:?}");
            assert_eq!(result[0]["outcome"], "refused");
            assert_eq!(result[0]["fallback_reason"], reason);
            assert!(result[0].get("remote_exit_code").is_none());
            let events =
                rch_common::IncidentLedger::with_path(root.path().join("state/incidents.jsonl"))
                    .read_all();
            assert_eq!(events.len(), 1, "refusal must retain its reason");
            assert!(!events[0].local_fallback_allowed);
            assert_eq!(
                events[0].details.get("reason").map(String::as_str),
                Some(reason)
            );
        }
    }
}

#[test]
fn permitted_fallback_runs_once_and_preserves_streams_and_exit_status() {
    let (root, output) = run_case("allowed", (false, true), "daemon unavailable", &[]);
    assert_eq!(output.status.code(), Some(23), "{output:?}");
    assert_eq!(std::fs::read(root.path().join("executed")).unwrap(), b"run");
    assert!(String::from_utf8_lossy(&output.stdout).contains("workload-out\n"));
    assert_eq!(output.stderr, b"workload-err\n");
    let result = envelopes(&output);
    assert_eq!(result.len(), 1, "{output:?}");
    assert_eq!(result[0]["outcome"], "completed");
    assert_eq!(result[0]["remote_exit_code"], 23);
}

#[test]
fn fallback_signal_and_spawn_failure_are_not_successful_completion() {
    for (case, code, outcome) in [
        ("signal", 143, "completed"),
        ("spawn-failure", 1, "transport_error"),
    ] {
        let (root, output) = run_case(case, (false, true), "daemon unavailable", &[]);
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        assert_eq!(root.path().join("executed").exists(), case == "signal");
        let result = envelopes(&output);
        assert_eq!(result.len(), 1, "{output:?}");
        assert_eq!(result[0]["outcome"], outcome);
        if case == "signal" {
            assert_eq!(result[0]["remote_exit_code"], 143);
        } else {
            assert!(result[0].get("remote_exit_code").is_none());
        }
    }
}

#[test]
fn ledger_failure_does_not_change_execution_or_refusal() {
    for (policy, code) in [
        ((false, false), 1),
        ((true, true), EXIT_REMOTE_REQUIRED_REFUSED),
        ((false, true), 23),
    ] {
        let (root, output) = run_case("ledger-unavailable", policy, "daemon unavailable", &[]);
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        assert_eq!(root.path().join("executed").exists(), code == 23);
        assert_eq!(envelopes(&output).len(), 1, "{output:?}");
        assert_eq!(
            std::fs::read(root.path().join("state")).unwrap(),
            b"not a directory"
        );
    }
}

#[test]
fn non_machine_fallback_keeps_protocol_stdout_free_of_envelopes() {
    let (_root, output) = run_case(
        "allowed",
        (false, true),
        "daemon unavailable",
        &[("RCH_TEST_MACHINE", "0")],
    );
    assert_eq!(output.status.code(), Some(23), "{output:?}");
    assert!(envelopes(&output).is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("workload-out\n"));
}

#[test]
fn uncertain_remote_completion_cannot_authorize_local_replay() {
    let error =
        release_unconfirmed_error(&WorkerId::new("worker-a"), 7).context("outer transfer failure");
    assert_eq!(
        classify_remote_pipeline_failure(&error),
        RemotePipelineFailurePolicy::FailClosedNoLocalFallback
    );
    assert_eq!(
        classify_remote_pipeline_failure(&anyhow::anyhow!("definite pre-start failure")),
        RemotePipelineFailurePolicy::AllowLocalFallback
    );
}
