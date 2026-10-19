//! Fork workflow policy after explicit removal of automatic merging.
//!
//! Validate the workflows that actually run, not the retired upstream merge
//! automation. These are structural/aggregate tests, not a substitute for
//! actionlint or executing the native product and release suites.

use serde_yaml_ng::Value;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .expect("workspace root")
}

fn document(path: &Path) -> Value {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_yaml_ng::from_str(&content)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn validate_permissions(value: &Value) -> Result<(), String> {
    let permissions = value.as_mapping().ok_or("non-mapping permissions")?;
    for (permission, value) in permissions {
        match value.as_str() {
            Some("read" | "none") => {}
            // The existing audit writes check results, never source or reviews.
            Some("write") if permission.as_str() == Some("checks") => {}
            _ => return Err(format!("unexpected permission {permission:?}: {value:?}")),
        }
    }
    Ok(())
}

fn validate_read_only_workflow(workflow: &Value) -> Result<(), String> {
    if workflow["permissions"]["contents"].as_str() != Some("read") {
        return Err("validation workflow must declare read-only contents".into());
    }
    validate_permissions(&workflow["permissions"])?;
    let triggers = workflow["on"]
        .as_mapping()
        .ok_or("expected explicit triggers")?;
    if triggers.contains_key(Value::String("pull_request_target".into())) {
        return Err("validation must not use privileged pull_request_target".into());
    }
    let jobs = workflow["jobs"].as_mapping().ok_or("missing jobs")?;
    for (name, job) in jobs {
        if !job["permissions"].is_null() {
            validate_permissions(&job["permissions"])?;
        }
        let needs: Vec<&str> = match &job["needs"] {
            Value::Null => Vec::new(),
            Value::String(name) => vec![name],
            Value::Sequence(names) => names
                .iter()
                .map(|name| name.as_str().ok_or("invalid dependency"))
                .collect::<Result<_, _>>()?,
            _ => return Err("invalid needs declaration".into()),
        };
        for dependency in needs {
            if !jobs.contains_key(Value::String(dependency.into())) {
                return Err(format!("job {name:?} needs nonexistent job {dependency}"));
            }
        }
    }
    Ok(())
}

#[test]
fn validation_workflows_are_read_only_and_have_resolvable_job_dependencies() {
    let directory = repo_root().join(".github/workflows");
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) != Some("yml")
            || path.file_name().and_then(|value| value.to_str()) == Some("release.yml")
        {
            continue;
        }
        validate_read_only_workflow(&document(&path))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }
}

#[test]
fn policy_validator_rejects_privilege_and_dependency_regressions() {
    let original = document(&repo_root().join(".github/workflows/ci.yml"));
    assert!(validate_read_only_workflow(&original).is_ok());
    let mut changed = original.clone();
    changed["permissions"]["contents"] = Value::String("write".into());
    assert!(validate_read_only_workflow(&changed).is_err());
    let mut changed = original.clone();
    changed["jobs"]["check"]["permissions"] = serde_yaml_ng::from_str("contents: write").unwrap();
    assert!(validate_read_only_workflow(&changed).is_err());
    let mut changed = original.clone();
    changed["permissions"]["pull-requests"] = Value::String("write".into());
    assert!(validate_read_only_workflow(&changed).is_err());
    let mut changed = original.clone();
    changed["on"]["pull_request_target"] = Value::Null;
    assert!(validate_read_only_workflow(&changed).is_err());
    let mut changed = original;
    changed["jobs"]["ci-success"]["needs"] = Value::String("deleted-job".into());
    assert!(validate_read_only_workflow(&changed).is_err());
}

#[test]
fn e2e_and_shipping_have_one_reusable_pr_owner() {
    let root = repo_root();
    let ci = document(&root.join(".github/workflows/ci.yml"));
    for (job, file) in [("e2e", "e2e.yml"), ("build-release", "test-release.yml")] {
        assert_eq!(
            ci["jobs"][job]["uses"].as_str(),
            Some(format!("./.github/workflows/{file}").as_str())
        );
        let called = document(&root.join(".github/workflows").join(file));
        let on = called["on"].as_mapping().unwrap();
        assert!(on.contains_key(Value::String("workflow_call".into())));
        for duplicate in ["pull_request", "push"] {
            assert!(
                !on.contains_key(Value::String(duplicate.into())),
                "duplicate {file} trigger"
            );
        }
    }
}

#[test]
fn dependency_update_proposals_remain_without_automatic_merging() {
    let root = repo_root();
    assert!(
        !root
            .join(".github/workflows/dependabot-automerge.yml")
            .exists()
    );
    let config = document(&root.join(".github/dependabot.yml"));
    let updates = config["updates"].as_sequence().unwrap();
    for ecosystem in ["cargo", "github-actions"] {
        assert!(
            updates
                .iter()
                .any(|update| update["package-ecosystem"].as_str() == Some(ecosystem))
        );
    }
}

#[cfg(unix)]
#[test]
fn actual_ci_aggregate_rejects_failed_cancelled_skipped_and_missing_required_results() {
    let ci = document(&repo_root().join(".github/workflows/ci.yml"));
    let run = ci["jobs"]["ci-success"]["steps"][0]["run"]
        .as_str()
        .unwrap();
    let script = run
        .strip_prefix("python3 - <<'PY'\n")
        .and_then(|body| body.trim_end().strip_suffix("\nPY"))
        .expect("aggregate must expose the actual Python program for this regression");
    let output = std::process::Command::new("python3")
        .args(["-c", AGGREGATE_CASES, script])
        .env_clear()
        .stdin(std::process::Stdio::null())
        .output()
        .expect("Python 3 is required by the Unix CI workflow tests");
    assert!(output.status.success(), "aggregate regression: {output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "93 aggregate cases passed"
    );
}

#[cfg(unix)]
const AGGREGATE_CASES: &str = r#"
import json
import os
import subprocess
import sys

script = sys.argv[1]
core = {'check', 'manifest_sync', 'security', 'test', 'e2e', 'build-release'}
extra = {'docs', 'benchmark', 'coverage'}
all_jobs = core | extra
cases = []
for event in ('pull_request', 'push', 'workflow_dispatch'):
    normal = {j: {'result': 'success' if j in core else 'skipped'} for j in all_jobs}
    cases.append((event, 'false', normal, core, 0))
    for job in core:
        for result in ('failure', 'cancelled', 'skipped', 'missing'):
            bad = dict(normal)
            if result == 'missing':
                bad.pop(job)
            else:
                bad[job] = {'result': result}
            cases.append((event, 'false', bad, core, 1))
extended = {j: {'result': 'success'} for j in all_jobs}
cases.append(('workflow_dispatch', 'true', extended, all_jobs, 0))
for job in extra:
    for result in ('failure', 'cancelled', 'skipped', 'missing'):
        bad = dict(extended)
        if result == 'missing':
            bad.pop(job)
        else:
            bad[job] = {'result': result}
        cases.append(('workflow_dispatch', 'true', bad, all_jobs, 1))
for result in ('success', 'failure', 'cancelled', 'skipped', 'missing'):
    scheduled = {j: {'result': 'skipped'} for j in all_jobs}
    if result == 'missing':
        scheduled.pop('security')
    else:
        scheduled['security'] = {'result': result}
    cases.append(('schedule', 'false', scheduled, {'security'}, int(result != 'success')))
for event, flag, results, required, expected in cases:
    env = dict(os.environ, JOB_RESULTS=json.dumps(results), EVENT_NAME=event, EXTENDED=flag)
    run = subprocess.run([sys.executable, '-c', script], env=env, capture_output=True,
                         text=True, timeout=5)
    assert run.returncode == expected, (event, flag, results, run.stdout, run.stderr)
    verdict = json.loads(run.stdout)
    assert set(verdict['required']) == required, (event, verdict)
    assert verdict['failed'] == {j: results.get(j, {}).get('result', 'missing')
                                for j in required if results.get(j, {}).get('result') != 'success'}
print(f'{len(cases)} aggregate cases passed')
"#;
