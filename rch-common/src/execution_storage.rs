//! Optional worker storage placement, independent of source mirrors and target pools.

use serde::{Deserialize, Serialize};

/// How a managed job exposes its scratch directory.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TmpMode {
    /// Set TMPDIR, TMP and TEMP. Programs may still explicitly write elsewhere.
    #[default]
    Env,
    /// Linux mount namespace with job scratch bound over /tmp. Requires mount privilege.
    PrivateMount,
}

/// Opt-in physical placement for remote execution. Unset paths preserve native RCH placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionStorageConfig {
    /// Derives `cache` and `tmp/jobs` below this absolute worker path.
    pub root: Option<String>,
    /// Override the derived package-cache root.
    pub cache_root: Option<String>,
    /// Override the parent of private job directories.
    pub tmp_root: Option<String>,
    /// Optional HOME; never derived automatically because it changes tool/config discovery.
    pub home_root: Option<String>,
    pub tmp_mode: TmpMode,
    /// Sweep inactive abandoned jobs older than this; zero disables the sweep.
    pub tmp_retention_hours: u32,
}

impl Default for ExecutionStorageConfig {
    fn default() -> Self {
        Self {
            root: None,
            cache_root: None,
            tmp_root: None,
            home_root: None,
            tmp_mode: TmpMode::Env,
            tmp_retention_hours: 24,
        }
    }
}

impl ExecutionStorageConfig {
    pub fn cache_root(&self) -> Option<String> {
        self.cache_root.clone().or_else(|| {
            self.root
                .as_ref()
                .map(|r| format!("{}/cache", r.trim_end_matches('/')))
        })
    }

    pub fn tmp_root(&self) -> Option<String> {
        self.tmp_root.clone().or_else(|| {
            self.root
                .as_ref()
                .map(|r| format!("{}/tmp/jobs", r.trim_end_matches('/')))
        })
    }

    pub fn enabled(&self) -> bool {
        self.root.is_some()
            || self.cache_root.is_some()
            || self.tmp_root.is_some()
            || self.home_root.is_some()
    }

    /// Validate without touching the controller filesystem: these paths belong to workers.
    pub fn validate(&self) -> Result<(), String> {
        for (key, path) in [
            ("root", &self.root),
            ("cache_root", &self.cache_root),
            ("tmp_root", &self.tmp_root),
            ("home_root", &self.home_root),
        ] {
            if let Some(path) = path
                && (!path.starts_with('/')
                    || path.contains("//")
                    || path.trim_matches('/').is_empty()
                    || path.chars().any(char::is_control)
                    || path.split('/').any(|part| matches!(part, "." | "..")))
            {
                return Err(format!(
                    "execution.storage.{key} must be an absolute non-root POSIX path without dot components or control characters"
                ));
            }
        }
        if self.tmp_mode == TmpMode::PrivateMount && self.tmp_root().is_none() {
            return Err(
                "execution.storage.tmp_mode=private_mount requires root or tmp_root".into(),
            );
        }
        if self.tmp_mode == TmpMode::PrivateMount {
            for path in [self.tmp_root(), self.cache_root(), self.home_root.clone()]
                .into_iter()
                .flatten()
            {
                if path.trim_end_matches('/') == "/tmp" || path.starts_with("/tmp/") {
                    return Err("private_mount storage must be outside /tmp, which is hidden by the private mount".into());
                }
            }
        }
        Ok(())
    }

    /// Package/runtime caches only. CARGO_TARGET_DIR remains owned by the target pool.
    pub fn cache_env(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        if let Some(root) = self.cache_root() {
            for (key, leaf) in [
                ("XDG_CACHE_HOME", "xdg"),
                ("CARGO_HOME", "cargo-home"),
                ("GOCACHE", "go-build"),
                ("GOMODCACHE", "go-mod"),
                ("GOPATH", "go-path"),
                ("NPM_CONFIG_CACHE", "npm"),
                ("BUN_INSTALL_CACHE_DIR", "bun"),
                ("UV_CACHE_DIR", "uv"),
                ("PIP_CACHE_DIR", "pip"),
                ("PLAYWRIGHT_BROWSERS_PATH", "playwright"),
            ] {
                pairs.push((key.into(), format!("{}/{leaf}", root.trim_end_matches('/'))));
            }
        }
        if let Some(home) = &self.home_root {
            pairs.push(("HOME".into(), home.clone()));
        }
        pairs
    }
}

/// Validate persistent remote defaults before assembling any shell command.
pub fn validate_remote_environment(
    environment: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    for (key, value) in environment {
        if !crate::ssh_utils::is_valid_env_key(key) || value.chars().any(char::is_control) {
            return Err(format!(
                "environment.remote contains an invalid key or control character: {key:?}"
            ));
        }
        if matches!(
            key.as_str(),
            "RCH_CH_BASE" | "RCH_CARGO_WRAPPER_BYPASS" | "CARGO_TARGET_DIR"
        ) {
            return Err(format!(
                "environment.remote.{key} is managed internally by RCH"
            ));
        }
    }
    Ok(())
}

/// Worker-side scratch lifetime. The lease FD is inherited by the existing
/// watchdog and workload, including their children. Cleanup never signals a
/// process or deletes a directory whose lease is still held. A killed supervisor
/// leaves an orphan for the next age-and-lock sweep, not a guessed completion.
/// Args: tmp parent, unique job token, mode, retention minutes, execution script.
pub const JOB_TMP_SCRIPT: &str = r#"
set -u
base=$1; token=$2; mode=$3; minutes=$4; workload=$5
case "$token" in ''|*[!a-zA-Z0-9-]*) exit 125;; esac
command -v flock >/dev/null 2>&1 || { printf '%s\n' 'RCH managed tmp requires flock on the worker' >&2; exit 125; }
if [ "$mode" = private_mount ]; then
    [ "$(uname -s)" = Linux ] && command -v unshare >/dev/null 2>&1 && command -v mount >/dev/null 2>&1 || {
        printf '%s\n' 'RCH private_mount requires Linux, unshare and mount' >&2; exit 125;
    }
fi
(umask 077; mkdir -p -- "$base") || exit 125
base=$(CDPATH= cd -- "$base" && pwd -P) || exit 125
if [ "$mode" = private_mount ]; then
    case "$base" in /tmp|/tmp/*) printf '%s\n' 'RCH private tmp root resolves inside /tmp' >&2; exit 125;; esac
fi
cleanup='d=$1; [ ! -L "$d" ] && [ -d "$d" ] && [ -f "$d/.rch-lease" ] && [ ! -L "$d/.rch-lease" ] || exit 1
    [ "$(cat "$d/.rch-owner")" = rch-execution-storage-v1 ] || exit 1
    flock -xn "$d/.rch-lease" rm -rf -- "$d"'
if [ "$minutes" -gt 0 ]; then
    find "$base" -mindepth 1 -maxdepth 1 -type d -name 'rch-job-*' -mmin +"$minutes" \
        -exec sh -c "$cleanup" rch-tmp-gc '{}' \; || exit 125
fi
job=$base/rch-job-$token
(umask 077; mkdir -- "$job" && mkdir -- "$job/tmp" && : > "$job/.rch-lease") || exit 125
# Do not overwrite inherited source-authority descriptors. 3 and 4 are used
# by RCH's watchdog/timeout wrappers. Only the validated digit enters eval;
# the path remains a quoted variable expansion, including spaces/metacharacters.
rch_tmp_fd=
for candidate in 9 8 7 6 5; do
    if ! (eval ": >&$candidate") 2>/dev/null; then rch_tmp_fd=$candidate; break; fi
done
[ -n "$rch_tmp_fd" ] || { printf '%s\n' 'RCH managed tmp needs one free descriptor in 5..9' >&2; exit 125; }
eval 'exec '"$rch_tmp_fd"'<>"$job/.rch-lease"' || exit 125
flock -s "$rch_tmp_fd" || exit 125
# Publish the GC marker only AFTER acquiring the lease. Even a suspended
# launcher may not look like an unlocked abandoned job before it starts.
printf '%s\n' rch-execution-storage-v1 > "$job/.rch-owner" || exit 125
(
    if [ "$mode" = private_mount ]; then
        unshare --mount --propagation private sh -c 'mount --bind "$1" /tmp || exit 125; exec sh -c "$2"' rch-private-tmp "$job/tmp" "$workload"
    else
        sh -c "$workload"
    fi
)
status=$?
eval "exec $rch_tmp_fd>&-"
sh -c "$cleanup" rch-tmp-cleanup "$job" || printf '%s\n' "RCH retained busy job tmp: $job" >&2
exit "$status"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_paths_and_package_caches() {
        assert!(!ExecutionStorageConfig::default().enabled());
        let config = ExecutionStorageConfig {
            root: Some("/srv/rch/".into()),
            ..Default::default()
        };
        assert_eq!(config.tmp_root().as_deref(), Some("/srv/rch/tmp/jobs"));
        assert_eq!(config.cache_root().as_deref(), Some("/srv/rch/cache"));
        assert!(
            config
                .cache_env()
                .iter()
                .any(|(k, v)| k == "GOCACHE" && v == "/srv/rch/cache/go-build")
        );
        assert!(
            !config
                .cache_env()
                .iter()
                .any(|(k, _)| k == "HOME" || k == "CARGO_TARGET_DIR")
        );
        for root in [
            "",
            "/",
            "relative",
            "/srv/../tmp",
            "/srv/./rch",
            "/srv/rch\n",
        ] {
            assert!(
                ExecutionStorageConfig {
                    root: Some(root.into()),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            ExecutionStorageConfig {
                tmp_mode: TmpMode::PrivateMount,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    fn run_tmp(root: &std::path::Path, token: &str, command: &str) -> std::process::Output {
        std::process::Command::new("sh")
            .args(["-c", JOB_TMP_SCRIPT, "rch-test"])
            .arg(root)
            .args([token, "env", "1", command])
            .output()
            .unwrap()
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn job_tmp_preserves_status_and_cleans_only_its_own_directory() {
        let root = tempfile::tempdir().unwrap();
        let sentinel = root.path().join("keep");
        std::fs::write(&sentinel, "unrelated").unwrap();
        let output = run_tmp(root.path(), "failed", "printf out; printf err >&2; exit 42");
        assert_eq!(output.status.code(), Some(42));
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
        assert!(!root.path().join("rch-job-failed").exists());
        assert!(sentinel.exists());
        assert!(!run_tmp(root.path(), "../escape", "exit 0").status.success());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn tmp_sweep_keeps_live_jobs_and_reaps_abandoned_jobs() {
        use std::io::BufRead;
        use std::process::Stdio;
        let root = tempfile::tempdir().unwrap();
        let job = root.path().join("rch-job-active");
        std::fs::create_dir(&job).unwrap();
        std::fs::write(job.join(".rch-owner"), "rch-execution-storage-v1\n").unwrap();
        let mut holder = std::process::Command::new("sh")
            .args([
                "-c",
                "exec 9<>\"$1/.rch-lease\"; flock -s 9; echo ready; read -r done",
                "lease",
            ])
            .arg(&job)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = String::new();
        std::io::BufReader::new(holder.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "ready");
        assert!(
            std::process::Command::new("touch")
                .args(["-d", "2 hours ago"])
                .arg(&job)
                .status()
                .unwrap()
                .success()
        );
        assert!(run_tmp(root.path(), "first", "exit 0").status.success());
        assert!(job.exists(), "an old but locked job must not be reclaimed");
        drop(holder.stdin.take());
        holder.wait().unwrap();
        assert!(run_tmp(root.path(), "second", "exit 0").status.success());
        assert!(!job.exists(), "unlocked abandoned job should be reclaimed");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn job_tmp_retains_live_descendants_after_parent_exit() {
        let root = tempfile::tempdir().unwrap();
        let output = run_tmp(
            root.path(),
            "descendant",
            "sleep 30 </dev/null >/dev/null 2>&1 & printf '%s' \"$!\"",
        );
        assert!(output.status.success());
        let pid = String::from_utf8(output.stdout).unwrap();
        assert!(root.path().join("rch-job-descendant/tmp").exists());
        assert!(String::from_utf8_lossy(&output.stderr).contains("retained busy job tmp"));
        assert!(
            std::process::Command::new("kill")
                .arg(&pid)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn job_tmp_does_not_replace_inherited_descriptors() {
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new("sh")
            .args(["-c", "exec 9>\"$1/preserved\"; sh -c \"$2\" rch-tmp \"$1\" descriptors env 0 'printf preserved >&9'", "fixture"])
            .arg(root.path()).arg(JOB_TMP_SCRIPT).output().unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert_eq!(
            std::fs::read(root.path().join("preserved")).unwrap(),
            b"preserved"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn private_mount_runs_in_namespace_or_refuses_before_workload() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let root = directory.path().canonicalize().unwrap();
        let available = std::process::Command::new("unshare")
            .args(["--mount", "--propagation", "private", "true"])
            .output()
            .unwrap()
            .status
            .success();
        let script = "test /tmp -ef \"$1/rch-job-private/tmp\" || exit 99; printf mounted";
        let workload = format!(
            "sh -c {} check {}",
            shell_escape::escape(script.into()),
            shell_escape::escape(root.to_str().unwrap().into())
        );
        let output = std::process::Command::new("sh")
            .args(["-c", JOB_TMP_SCRIPT, "rch-test"])
            .arg(&root)
            .args(["private", "private_mount", "0", &workload])
            .output()
            .unwrap();
        if available {
            assert!(output.status.success(), "{:?}", output);
            assert_eq!(output.stdout, b"mounted");
        } else {
            assert!(!output.status.success());
            assert!(
                output.stdout.is_empty(),
                "workload must not run without mount privilege"
            );
        }
        assert!(!root.join("rch-job-private").exists());
    }
}
