//! Black-box ordinary remote-test replacement contracts through the real RCH CLI.
//! These are native local-fallback and refusal tests, not live worker evidence.
//! Each case owns its HOME/config/project/guard and retains files for inspection;
//! no real daemon, hooks, worker configuration or optional RABS cache is used.
#![cfg(target_os = "linux")]

use serde_json::Value;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    calls: AtomicUsize,
}

struct Run {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    elapsed: Duration,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("rch-invocation-parity-")
            .tempdir()
            .unwrap()
            .keep();
        let project = root.join("project with spaces");
        for dir in ["home", "config", "state", "cache", "guard"] {
            fs::create_dir(root.join(dir)).unwrap();
        }
        fs::create_dir(&project).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("guard"), fs::Permissions::from_mode(0o700)).unwrap();
        let fixture = Self {
            root,
            project,
            calls: AtomicUsize::new(0),
        };
        fixture.configure("hybrid", true);
        fixture
    }

    fn configure(&self, role: &str, allow_fallback: bool) {
        let guard = toml::Value::String(self.root.join("guard").display().to_string());
        fs::write(
            self.root.join("config/config.toml"),
            format!(
                "[general]\nenabled = true\nrole = {role:?}\n\
                 [compilation]\nallow_local_fallback = {allow_fallback}\n\
                 [jobs]\nlocal_guard_root = {guard}\nshared_workspaces = []\n"
            ),
        )
        .unwrap();
        fs::write(self.root.join("config/workers.toml"), "workers = []\n").unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rch"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("RCH_CONFIG_DIR", self.root.join("config"))
            .env("RCH_STATE_HOME", self.root.join("state"))
            .env("RCH_NO_SELF_HEALING", "1")
            .env("RCH_DAEMON_INSTALLS_HOOKS", "0")
            .env("RCH_NO_UPDATE_CHECK", "1")
            .env("RCH_LOG_LEVEL", "error")
            .env("NO_COLOR", "1")
            .current_dir(&self.project)
            .arg("--no-self-healing");
        command
    }

    fn exec(&self) -> Command {
        let mut command = self.command();
        command.args(["exec", "--verification"]);
        command
    }

    fn run(&self, command: &mut Command, input: &[u8]) -> Run {
        self.run_with_control(command, input, |_, _| {})
    }

    fn run_with_control(
        &self,
        command: &mut Command,
        input: &[u8],
        mut control: impl FnMut(&std::process::Child, Duration),
    ) -> Run {
        let id = self.calls.fetch_add(1, Ordering::Relaxed);
        let stdout = self.root.join(format!("{id}.stdout"));
        let stderr = self.root.join(format!("{id}.stderr"));
        // Use a real input file so a PTY backpressure defect cannot block the
        // harness itself in a synchronous pipe write before its deadline loop.
        let stdin = self.root.join(format!("{id}.stdin"));
        fs::write(&stdin, input).unwrap();
        command
            .stdin(File::open(stdin).unwrap())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap());
        let start = Instant::now();
        let mut child = command.spawn().expect("spawn real RCH CLI");
        let deadline = start + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            control(&child, start.elapsed());
            if Instant::now() >= deadline {
                // Signal only our unreaped CLI child. Its guard owns workload
                // teardown; do not kill by executable name or guessed group.
                let pid =
                    rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
                let stop_deadline = Instant::now() + Duration::from_secs(5);
                while child.try_wait().unwrap().is_none() && Instant::now() < stop_deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                if child.try_wait().unwrap().is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
                panic!(
                    "RCH invocation exceeded deadline; evidence: {}",
                    self.root.display()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        Run {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
            elapsed: start.elapsed(),
        }
    }

    fn result(&self, path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap_or_else(|error| {
            panic!(
                "missing result {}: {error}; evidence {}",
                path.display(),
                self.root.display()
            )
        }))
        .expect("one complete JSON result document")
    }

    fn assert_local_result(&self, path: &Path, outcome: &str, exit: i32) {
        let result = self.result(path);
        assert_eq!(result["api_version"], "1.0");
        assert_eq!(result["location"], "local");
        assert_eq!(result["outcome"], outcome);
        assert_eq!(result["remote_exit_code"], exit);
        assert!(result.get("worker_id").is_none_or(Value::is_null));
        assert!(
            result["fallback_reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
        );
    }

    fn assert_guard_reclaimed(&self) {
        let jobs = self.root.join("guard/jobs");
        assert_eq!(
            fs::read_dir(jobs).unwrap().count(),
            0,
            "completed local jobs leave no live scratch"
        );
    }
}

#[test]
fn rtk_hook_preserves_exact_literal_export_event_and_controls() {
    let fixture = Fixture::new();
    // Exact sanitized PreToolUse bytes from the live rejection. Only planning
    // runs here; neither this cwd nor its payload is executed or mutated.
    let event = br#"{"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": "rtk run -c 'export GOTOOLCHAIN=local GOSUMDB=off GOFLAGS=-mod=mod GOPROXY=file:///mnt/vibe-coding-share/scratch/rch-rtk-live-01a1387e-20261015T044808Z-00d4c2/module-proxy RCH_ACCEPT_LITERAL='\"'\"'literal value ; $HOME \u4e2d\u6587'\"'\"'; /usr/local/go/bin/go test -v -count=1 ./...'", "timeout": 120000, "description": "bounded RCH RTK deployment acceptance"}, "cwd": "/mnt/vibe-coding-share/scratch/rch-rtk-live-01a1387e-20261015T044808Z-00d4c2"}"#;
    let original: Value = serde_json::from_slice(event).unwrap();
    let exact = original["tool_input"]["command"].as_str().unwrap();
    for (index, command) in [
        exact.to_owned(),
        exact.replacen("rtk run -c ", "rtk run ", 1),
        exact.replacen("rtk run -c ", "rtk proxy sh -c ", 1),
        "rtk run 'export GOTOOLCHAIN=local; /usr/local/go/bin/go test -v -count=1 ./...'".into(),
        "rtk run -c '/usr/local/go/bin/go test -v -count=1 ./...'".into(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = original.clone();
        request["tool_input"]["command"] = command.clone().into();
        let input = if index == 0 {
            event.to_vec()
        } else {
            serde_json::to_vec(&request).unwrap()
        };
        let output = fixture.run(fixture.command().env("RCH_REQUIRE_REMOTE", "1"), &input);
        assert!(output.status.success(), "{:?}", output.stderr);
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        let mut expected = request["tool_input"].clone();
        expected["command"] = format!("rch exec --verification -- {command}").into();
        assert_eq!(response["hookSpecificOutput"]["updatedInput"], expected);
        assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    }
    assert!(!fixture.root.join("state/job-leases").exists());
}

#[test]
fn rtk_proxy_hook_preserves_exact_export_and_conditional_event() {
    let fixture = Fixture::new();
    // Exact 548-byte supplied event (SHA256 89ffcbea...e0a634). Planning only;
    // the historical cwd and file:// proxy operands must never be executed.
    let event = br#"{"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": "rtk proxy sh -c 'export GOTOOLCHAIN=local GOSUMDB=off GOFLAGS=-mod=mod GOPROXY=file:///mnt/vibe-coding-share/scratch/rch-rtk-live-01a1387e-20261015T044808Z-00d4c2/module-proxy RCH_ACCEPT_LITERAL='\"'\"'literal value ; $HOME \u4e2d\u6587'\"'\"'; go test -v -count=1 ./... && node --test probe.test.cjs'", "timeout": 120000, "description": "bounded RCH RTK deployment acceptance"}, "cwd": "/mnt/vibe-coding-share/scratch/rch-rtk-live-01a1387e-20261015T044808Z-00d4c2"}"#;
    assert_eq!(event.len(), 548);
    let original: Value = serde_json::from_slice(event).unwrap();
    let output = fixture.run(fixture.command().env("RCH_REQUIRE_REMOTE", "1"), event);
    assert!(output.status.success(), "{:?}", output.stderr);
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut expected = original["tool_input"].clone();
    expected["command"] = format!(
        "rch exec --verification -- {}",
        original["tool_input"]["command"].as_str().unwrap()
    )
    .into();
    assert_eq!(response["hookSpecificOutput"]["updatedInput"], expected);
    assert!(output.stderr.is_empty());
    assert!(!fixture.root.join("state/job-leases").exists());
}

#[test]
fn literal_argv_cwd_environment_and_nonzero_exit_are_not_replayed() {
    let fixture = Fixture::new();
    let result = fixture.root.join("result.json");
    let cwd = fixture.project.join("sub directory");
    fs::create_dir(&cwd).unwrap();
    let literal = "a b; $(touch should-not-exist)\n'\"";
    let code = "import os,sys; open('executions','ab').write(b'x'); sys.stdout.buffer.write((os.getcwd()+'\\0'+os.environ['VALUE']+'\\0'+os.environ['EMPTY']+'\\0'+sys.argv[1]).encode()); sys.exit(7)";
    let output = fixture.run(
        fixture
            .exec()
            .arg("--cwd")
            .arg(&cwd)
            .args(["--env", "VALUE=a=b c", "--env", "EMPTY="])
            .arg("--json-result")
            .arg(&result)
            .args(["--", "/usr/bin/python3", "-I", "-c", code, literal]),
        b"",
    );
    assert_eq!(
        output.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        format!("{}\0a=b c\0\0{literal}", cwd.display()).as_bytes()
    );
    assert_eq!(fs::read(cwd.join("executions")).unwrap(), b"x");
    assert!(!cwd.join("should-not-exist").exists());
    fixture.assert_local_result(&result, "completed", 7);
    fixture.assert_guard_reclaimed();
}

#[test]
fn native_streams_preserve_binary_bytes_and_stdin_eof() {
    let fixture = Fixture::new();
    let result = fixture.root.join("binary.json");
    let input = b"input\0\xff\xfe\n";
    let code = "import sys; data=sys.stdin.buffer.read(); sys.stdout.buffer.write(b'out\\x00\\xff'+data); sys.stderr.buffer.write(b'err\\x00\\xfe')";
    let output = fixture.run(
        fixture.exec().arg("--json-result").arg(&result).args([
            "--",
            "/usr/bin/python3",
            "-I",
            "-c",
            code,
        ]),
        input,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, [b"out\0\xff".as_slice(), input].concat());
    assert!(
        output.stderr.ends_with(b"err\0\xfe"),
        "stderr bytes must not move to stdout or be decoded"
    );
    fixture.assert_local_result(&result, "completed", 0);
}

#[test]
fn raw_shell_keeps_cd_pipeline_redirects_and_exit_status() {
    let fixture = Fixture::new();
    let result = fixture.root.join("shell.json");
    let output = fixture.run(
        fixture.exec().arg("--json-result").arg(&result).args([
            "--shell", "mkdir output && cd output && printf abc | tr a-z A-Z > value && cat value; printf shell-error >&2; exit 9",
        ]),
        b"",
    );
    assert_eq!(output.status.code(), Some(9));
    assert_eq!(output.stdout, b"ABC");
    assert!(output.stderr.ends_with(b"shell-error"));
    assert_eq!(
        fs::read(fixture.project.join("output/value")).unwrap(),
        b"ABC"
    );
    fixture.assert_local_result(&result, "completed", 9);
}

#[test]
fn explicit_raw_shell_supports_bash_arrays_and_process_substitution() {
    let fixture = Fixture::new();
    let output = fixture.run(
        fixture.exec().args([
            "--shell",
            "items=('one two' three); [[ ${#items[@]} == 2 ]] && printf '%s\\n' \"${items[@]}\"; cat <(printf process-substitution)",
        ]),
        b"",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"one two\nthree\nprocess-substitution");
}

#[test]
fn result_path_is_relative_to_invoker_not_overridden_cwd() {
    let fixture = Fixture::new();
    let cwd = fixture.project.join("sub");
    fs::create_dir(&cwd).unwrap();
    let output = fixture.run(
        fixture.exec().arg("--cwd").arg(&cwd).args([
            "--json-result",
            "outcome.json",
            "--",
            "/usr/bin/printf",
            "done",
        ]),
        b"",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"done");
    assert!(!cwd.join("outcome.json").exists());
    fixture.assert_local_result(&fixture.project.join("outcome.json"), "completed", 0);
}

#[test]
fn explicit_env_overrides_forwarded_value_without_shell_expansion() {
    let fixture = Fixture::new();
    let literal = "$(touch unwanted); value=two\nlines";
    let output = fixture.run(
        fixture
            .exec()
            .env("FORWARD_ME", "ambient")
            .args(["--forward-env", "FORWARD_ME", "--env"])
            .arg(format!("FORWARD_ME={literal}"))
            .args([
                "--",
                "/usr/bin/python3",
                "-I",
                "-c",
                "import os,sys; sys.stdout.write(os.environ['FORWARD_ME'])",
            ]),
        b"",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, literal.as_bytes());
    assert!(!fixture.project.join("unwanted").exists());
}

#[test]
fn timeout_terminates_owned_workload_once_and_publishes_distinct_outcome() {
    let fixture = Fixture::new();
    let result = fixture.root.join("timeout.json");
    let output = fixture.run(
        fixture.exec().arg("--json-result").arg(&result).args([
            "--timeout",
            "1s",
            "--shell",
            "printf x >> executions; exec sleep 30",
        ]),
        b"",
    );
    assert_eq!(
        output.status.code(),
        Some(124),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.elapsed < Duration::from_secs(10));
    assert_eq!(fs::read(fixture.project.join("executions")).unwrap(), b"x");
    fixture.assert_local_result(&result, "deadline_exceeded", 124);
    fixture.assert_guard_reclaimed();
}

#[test]
fn detached_descendant_is_reaped_before_scratch_is_reclaimed() {
    let fixture = Fixture::new();
    let result = fixture.root.join("descendants.json");
    let code = r#"import os, pathlib, subprocess, sys, time
root = pathlib.Path.cwd()
worker = '''import os,pathlib,time
root=pathlib.Path.cwd()
root.joinpath("descendant.pid").write_text(str(os.getpid()))
(root / "descendant.ready").write_bytes(b"ready")
end=time.monotonic()+12
while time.monotonic()<end:
    with (root / "heartbeat").open("ab") as stream: stream.write(b"x")
    time.sleep(0.05)
'''
subprocess.Popen([sys.executable, '-I', '-c', worker], start_new_session=True,
                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
end=time.monotonic()+5
while not root.joinpath('descendant.ready').exists():
    if time.monotonic()>end: raise RuntimeError('child never became ready')
    time.sleep(0.01)
"#;
    let output = fixture.run(
        fixture.exec().arg("--json-result").arg(&result).args([
            "--",
            "/usr/bin/python3",
            "-I",
            "-c",
            code,
        ]),
        b"",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.elapsed < Duration::from_secs(10),
        "cleanup must not wait for the descendant's 12-second natural exit"
    );
    let pid: u32 = fs::read_to_string(fixture.project.join("descendant.pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "adopted descendant must be reaped before success"
    );
    fixture.assert_local_result(&result, "completed", 0);
    fixture.assert_guard_reclaimed();
}

#[test]
fn interrupting_cli_stops_workload_and_publishes_interrupted_result() {
    let fixture = Fixture::new();
    let result = fixture.root.join("interrupted.json");
    let ready = fixture.project.join("workload.ready");
    let mut interrupted = false;
    let output = fixture.run_with_control(
        fixture.exec().arg("--json-result").arg(&result).args([
            "--shell",
            "printf x >> executions; printf ready > workload.ready; exec sleep 30",
        ]),
        b"",
        |child, _| {
            if !interrupted && ready.is_file() {
                let pid =
                    rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
                rustix::process::kill_process(pid, rustix::process::Signal::INT).unwrap();
                interrupted = true;
            }
        },
    );
    assert!(
        interrupted,
        "the workload must reach readiness before interruption"
    );
    assert_eq!(
        output.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(fixture.project.join("executions")).unwrap(), b"x");
    fixture.assert_local_result(&result, "interrupted", 130);
    fixture.assert_guard_reclaimed();
}

#[test]
fn signal_death_is_not_reported_as_a_normal_workload_exit() {
    let fixture = Fixture::new();
    let result = fixture.root.join("signal.json");
    let output = fixture.run(
        fixture
            .exec()
            .arg("--json-result")
            .arg(&result)
            .args(["--shell", "kill -TERM $$"]),
        b"",
    );
    assert_eq!(output.status.code(), Some(143));
    fixture.assert_local_result(&result, "signaled", 143);
}

#[test]
fn strict_remote_and_dispatcher_never_execute_the_local_fallback() {
    for (role, strict, allow) in [
        ("hybrid", true, true),
        ("dispatcher", false, true),
        ("hybrid", false, false),
    ] {
        let fixture = Fixture::new();
        fixture.configure(role, allow);
        let result = fixture.root.join("refusal.json");
        let mut command = fixture.exec();
        if strict {
            command.env("RCH_REQUIRE_REMOTE", "1");
        }
        // A falsy explicit RCH_REQUIRE_REMOTE overrides the dispatcher default;
        // leave it absent here to test that default, not the opt-out contract.
        let output = fixture.run(
            command
                .arg("--json-result")
                .arg(&result)
                .args(["--shell", "printf should-not-run > marker"]),
            b"",
        );
        assert!(
            !output.status.success(),
            "{role}, strict={strict}, allow={allow}"
        );
        assert!(output.stdout.is_empty());
        assert!(!fixture.project.join("marker").exists());
        let value = fixture.result(&result);
        assert_ne!(value["outcome"], "completed");
        assert_ne!(value["location"], "remote");
    }
}

#[test]
fn explicit_remote_opt_out_keeps_existing_dispatcher_override_semantics() {
    let fixture = Fixture::new();
    fixture.configure("dispatcher", true);
    let result = fixture.root.join("opt-out.json");
    let output = fixture.run(
        fixture
            .exec()
            .env("RCH_REQUIRE_REMOTE", "0")
            .arg("--json-result")
            .arg(&result)
            .args(["--", "/usr/bin/printf", "explicit-local"]),
        b"",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"explicit-local");
    fixture.assert_local_result(&result, "completed", 0);
}

#[test]
fn exact_worker_and_tool_requirements_never_gain_verification_fallback_permission() {
    for required in [["--worker", "exact-worker"], ["--require-tool", "pytest"]] {
        let fixture = Fixture::new();
        fixture.configure("hybrid", true);
        let result = fixture.root.join("constraint-refusal.json");
        let output = fixture.run(
            fixture
                .exec()
                .env("RCH_REQUIRE_REMOTE", "0")
                .args(required)
                .arg("--json-result")
                .arg(&result)
                .args(["--shell", "printf must-not-run > marker"]),
            b"",
        );
        assert!(!output.status.success(), "{required:?}");
        assert!(output.stdout.is_empty());
        assert!(!fixture.project.join("marker").exists());
        let value = fixture.result(&result);
        assert_ne!(value["outcome"], "completed");
        assert_ne!(value["location"], "remote");
    }
}

#[test]
fn explicit_job_does_not_gain_verification_fallback_permission() {
    let fixture = Fixture::new();
    let output = fixture.run(
        fixture
            .exec()
            .args(["--job", "--shell", "printf should-not-run > marker"]),
        b"",
    );
    assert!(!output.status.success());
    assert!(!fixture.project.join("marker").exists());
}

#[test]
fn invocation_cwd_selects_project_configuration_before_fallback() {
    let fixture = Fixture::new();
    let cwd = fixture.project.join("restricted project");
    fs::create_dir_all(cwd.join(".rch")).unwrap();
    fs::write(
        cwd.join(".rch/config.toml"),
        "[compilation]\nallow_local_fallback = false\n",
    )
    .unwrap();
    let output = fixture.run(
        fixture
            .exec()
            .arg("--cwd")
            .arg(&cwd)
            .args(["--shell", "printf should-not-run > marker"]),
        b"",
    );
    assert!(
        !output.status.success(),
        "must use the invocation project's policy"
    );
    assert!(!cwd.join("marker").exists());
    assert!(!fixture.project.join("marker").exists());
}

#[test]
fn hook_payload_cwd_controls_project_policy_without_changing_process_cwd() {
    let fixture = Fixture::new();
    let restricted = fixture.project.join("restricted");
    fs::create_dir_all(restricted.join(".rch")).unwrap();
    fs::write(
        restricted.join(".rch/config.toml"),
        "[general]\nforce_local = true\n",
    )
    .unwrap();
    let command = "python3 -m pytest tests/test_example.py";
    let event = serde_json::json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": command}, "cwd": restricted,
    });
    let output = fixture.run(&mut fixture.command(), &serde_json::to_vec(&event).unwrap());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.stdout.is_empty() {
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            value.pointer("/hookSpecificOutput/updatedInput").is_none(),
            "payload project forbids rewriting: {value}"
        );
    }
    let event = serde_json::json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": command}, "cwd": fixture.project,
    });
    let output = fixture.run(&mut fixture.command(), &serde_json::to_vec(&event).unwrap());
    assert!(output.status.success());
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("control case must actually plan a rewrite");
    let rewritten = value
        .pointer("/hookSpecificOutput/updatedInput/command")
        .and_then(Value::as_str)
        .unwrap();
    assert!(rewritten.contains("rch exec --verification"));
    assert!(
        value
            .pointer("/hookSpecificOutput/permissionDecision")
            .is_none(),
        "rewrite must not grant permission"
    );
}

#[test]
fn interactive_mode_really_provides_a_terminal_and_result_file() {
    let fixture = Fixture::new();
    let result = fixture.root.join("interactive.json");
    let output = fixture.run(
        fixture.exec().arg("--interactive").arg("--json-result").arg(&result).args([
            "--", "/usr/bin/python3", "-I", "-c",
            "import os,sys; sys.stdout.write('pty' if all(os.isatty(fd) for fd in (0,1,2)) else 'not-a-pty')",
        ]),
        b"",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"pty");
    fixture.assert_local_result(&result, "completed", 0);
}

#[test]
fn ordinary_invocation_can_read_its_foreground_terminal_without_interactive_flag() {
    let fixture = Fixture::new();
    let result = fixture.root.join("foreground.json");
    let driver = r#"import errno,os,select,signal,sys,time
pid,master=os.forkpty()
if pid==0:
    os.execv(sys.argv[1],sys.argv[1:])
data=bytearray(); sent=False; status=None
try:
    deadline=time.monotonic()+12
    while time.monotonic()<deadline:
        ready,_,_=select.select([master],[],[],0.05)
        if ready:
            try: chunk=os.read(master,65536)
            except OSError as error:
                if error.errno!=errno.EIO: raise
                chunk=b''
            data.extend(chunk)
            if b'STDIN_READY' in data and not sent:
                os.write(master,b'input-value\n'); sent=True
        observed,value=os.waitpid(pid,os.WNOHANG)
        if observed:
            status=value
            break
    if status is None: raise RuntimeError('foreground terminal read never completed: '+repr(data))
    if not sent or b'READ:input-value' not in data: raise RuntimeError('stdin did not reach workload: '+repr(data))
    if os.waitstatus_to_exitcode(status)!=0: raise RuntimeError('rch failed: '+repr(data))
finally:
    if status is None:
        try: os.kill(pid,signal.SIGTERM)
        except ProcessLookupError: pass
        cleanup=time.monotonic()+5
        while time.monotonic()<cleanup:
            observed,value=os.waitpid(pid,os.WNOHANG)
            if observed: status=value; break
            time.sleep(0.05)
        if status is None:
            os.kill(pid,signal.SIGKILL); os.waitpid(pid,0)
    os.close(master)
print('foreground-stdin-ok')
"#;
    let template = fixture.command();
    let mut command = Command::new("/usr/bin/python3");
    command.env_clear().current_dir(&fixture.project);
    for (key, value) in template.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    command.args(["-I", "-c", driver, env!("CARGO_BIN_EXE_rch"), "--no-self-healing", "exec", "--verification", "--json-result"])
        .arg(&result).args(["--", "/usr/bin/python3", "-I", "-c",
            "import sys; print('STDIN_READY',flush=True); print('READ:'+sys.stdin.readline().strip(),flush=True)"]);
    let output = fixture.run(&mut command, b"");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"foreground-stdin-ok\n");
    fixture.assert_local_result(&result, "completed", 0);
    fixture.assert_guard_reclaimed();
}

#[test]
fn interactive_canonical_input_without_newline_reaches_eof() {
    let fixture = Fixture::new();
    let output = fixture.run(
        fixture.exec().args([
            "--interactive",
            "--timeout",
            "5s",
            "--",
            "/usr/bin/python3",
            "-I",
            "-c",
            "import sys; data=sys.stdin.read(); print('READ:'+repr(data),flush=True)",
        ]),
        b"abc",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("READ:'abc'"));
    fixture.assert_guard_reclaimed();
}

#[test]
fn interactive_duplex_output_does_not_deadlock_pending_input() {
    let fixture = Fixture::new();
    let code = "import os,sys; left=262144; chunk=b'Y'*4096;\nwhile left: left-=os.write(1,chunk[:left])\ndata=sys.stdin.buffer.read(); print('RECEIVED='+str(len(data)),flush=True)";
    let input = "input-line\n".repeat(12_000);
    let output = fixture.run(
        fixture.exec().args([
            "--interactive",
            "--timeout",
            "10s",
            "--",
            "/usr/bin/python3",
            "-I",
            "-c",
            code,
        ]),
        input.as_bytes(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'Y').count(),
        262144
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("RECEIVED={}", input.len())));
    fixture.assert_guard_reclaimed();
}

#[test]
fn real_generic_shim_and_ci_execute_tool_once_with_literal_args() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    let native = fixture.root.join("native-tools");
    fs::create_dir(&native).unwrap();
    // The tool is a protocol witness, NOT a claim that pytest or a worker ran.
    let pytest = native.join("pytest");
    fs::write(&pytest, "#!/bin/sh\n[ \"${RCH_GUARDED_LOCAL:-}\" = 1 ] || exit 91\nprintf x >> executions\nprintf '%s\\0' \"$@\"\nprintf tool-error >&2\nexit 7\n").unwrap();
    fs::set_permissions(&pytest, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(env!("CARGO_BIN_EXE_rch"), native.join("rch")).unwrap();
    let installed = fixture.run(
        fixture
            .command()
            .args(["shim", "install", "--tools", "pytest"]),
        b"",
    );
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let path =
        std::env::join_paths([native.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    let output = fixture.run(
        fixture.command().env("PATH", &path).args([
            "ci",
            "--",
            "pytest",
            "-q",
            "literal space; value",
        ]),
        b"",
    );
    assert_eq!(
        output.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"-q\0literal space; value\0");
    assert!(output.stderr.ends_with(b"tool-error"));
    assert_eq!(fs::read(fixture.project.join("executions")).unwrap(), b"x");
    fixture.assert_guard_reclaimed();

    // An ordinary interpreter is not verification itself; its subprocess must
    // still enter the installed shim instead of inheriting a global bypass.
    let output = fixture.run(
        fixture.command().env("PATH", path).args([
            "ci",
            "--",
            "/usr/bin/python3",
            "-I",
            "-c",
            "import subprocess,sys; sys.exit(subprocess.call(['pytest','nested']))",
        ]),
        b"",
    );
    assert_eq!(
        output.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"nested\0");
    assert_eq!(fs::read(fixture.project.join("executions")).unwrap(), b"xx");
    fixture.assert_guard_reclaimed();
}

#[test]
fn rtk_build_refusal_never_executes_on_controller_in_remote_required_policy() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let rtk = bin.join("rtk");
    fs::write(
        &rtk,
        "#!/bin/sh\nprintf x >> controller-executions\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&rtk, fs::Permissions::from_mode(0o700)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    for args in [
        vec![
            "exec",
            "--",
            "rtk",
            "run",
            "-c",
            "/usr/local/go/bin/go test ./...",
        ],
        vec!["exec", "--verification", "--", "rtk", "cargo", "build"],
        vec!["exec", "--verification", "--", "rtk", "test", "bun", "test"],
        vec![
            "shim",
            "dispatch",
            "--tool",
            "rtk",
            "--",
            "proxy",
            "sh",
            "-c",
            "export X='literal value ; $HOME 中文'; go test ./... && node --test probe.test.cjs",
        ],
    ] {
        let output = fixture.run(
            fixture
                .command()
                .env("PATH", &path)
                .env("RCH_REQUIRE_REMOTE", "1")
                .env("RCH_SHIM_REAL_TOOL", &rtk)
                .args(args),
            b"",
        );
        assert!(!output.status.success());
        assert!(!fixture.project.join("controller-executions").exists());
    }
    let output = fixture.run(
        fixture.command().env("PATH", &path).args([
            "exec",
            "--",
            "rtk",
            "run",
            "-c",
            "printf harmless",
        ]),
        b"",
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(
        fs::read(fixture.project.join("controller-executions")).unwrap(),
        b"x"
    );
    let ordinary = fixture.run(
        fixture.command().args([
            "exec",
            "--",
            "env",
            "RTK_TEE_DIR=/literal",
            "/usr/bin/printf",
            "%s",
            "not-a-build",
        ]),
        b"",
    );
    assert!(ordinary.status.success(), "{:?}", ordinary.stderr);
    assert_eq!(ordinary.stdout, b"not-a-build");
}

#[test]
fn authored_rtk_retains_controller_accessible_raw_logs_on_failure() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let native = fixture.root.join("native-tools");
    fs::create_dir(&native).unwrap();
    let rtk = native.join("rtk");
    // This writer exercises the RTK tee environment contract only. It does not
    // stand in for RTK compression correctness or live remote execution.
    fs::write(&rtk, "#!/bin/sh\ntest -d \"$RTK_TEE_DIR\" || exit 91\nprintf 'raw\\000\\377' > \"$RTK_TEE_DIR/raw.log\"\nprintf filtered-output\nexit 7\n").unwrap();
    fs::set_permissions(&rtk, fs::Permissions::from_mode(0o700)).unwrap();
    let result = fixture.root.join("rtk.json");
    let path =
        std::env::join_paths([native.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    let output = fixture.run(
        fixture
            .exec()
            .env("PATH", path)
            .arg("--json-result")
            .arg(&result)
            .args(["--", "rtk", "pytest", "-q"]),
        b"",
    );
    assert_eq!(
        output.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"filtered-output");
    fixture.assert_local_result(&result, "completed", 7);
    let record = fixture.result(&result);
    let details = &record["execution_details"];
    assert_eq!(details["lease_retained"], true);
    let logs = details["logs"]
        .as_array()
        .expect("controller-readable log locations");
    let tee = logs
        .iter()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .find(|path| path.is_dir() && path.join("raw.log").is_file())
        .expect("returned tee location must exist after command failure");
    assert!(tee.starts_with(fixture.root.join("guard/jobs")));
    assert_eq!(
        fs::metadata(&tee).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(fs::read(tee.join("raw.log")).unwrap(), b"raw\0\xff");
}

#[test]
fn conflicting_or_invalid_cli_options_fail_before_any_workload() {
    let cases: &[&[&str]] = &[
        &["--shell", "printf shell", "--", "/usr/bin/touch", "marker"],
        &["--timeout", "0", "--", "/usr/bin/touch", "marker"],
        &["--env", "BROKEN", "--", "/usr/bin/touch", "marker"],
        &["--env", "1KEY=value", "--", "/usr/bin/touch", "marker"],
        &[
            "--cwd",
            "absent-directory",
            "--",
            "/usr/bin/touch",
            "marker",
        ],
        &["--workspace", "mirror", "--", "/usr/bin/touch", "marker"],
    ];
    for args in cases {
        let fixture = Fixture::new();
        let output = fixture.run(fixture.exec().args(*args), b"");
        assert!(!output.status.success(), "accepted invalid args {args:?}");
        assert!(
            !fixture.project.join("marker").exists(),
            "executed {args:?}"
        );
    }
}

#[test]
fn machine_stdout_mode_refuses_typed_workload_before_execution() {
    let fixture = Fixture::new();
    let output = fixture.run(
        fixture.command().args([
            "--json",
            "exec",
            "--verification",
            "--shell",
            "printf should-not-run > marker",
        ]),
        b"",
    );
    assert!(!output.status.success());
    assert!(!fixture.project.join("marker").exists());
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("one refusal envelope, not workload output");
    assert_eq!(value["outcome"], "refused");
}

#[test]
fn result_publication_failure_does_not_replay_completed_command() {
    let fixture = Fixture::new();
    let output = fixture.run(
        fixture
            .exec()
            .arg("--json-result")
            .arg(fixture.root.join("absent/result.json"))
            .args(["--shell", "printf x >> executions; printf done"]),
        b"",
    );
    assert!(
        !output.status.success(),
        "missing requested result is a control failure"
    );
    assert_eq!(output.stdout, b"done");
    assert_eq!(fs::read(fixture.project.join("executions")).unwrap(), b"x");
}

#[test]
fn lone_argv_with_spaces_is_an_executable_name_not_shell_source() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let tool = fixture.project.join("a tool with spaces");
    fs::write(&tool, "#!/bin/sh\nprintf literal-executable\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
    let output = fixture.run(fixture.exec().arg("--").arg(&tool), b"");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"literal-executable");
}
