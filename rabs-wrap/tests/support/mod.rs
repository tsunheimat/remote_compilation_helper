//! Daemon preparation shared by the real-wrapper integration fixtures.
//!
//! A workspace test run has a different unified dependency graph from a
//! single-package production build. The runner can pass its package/feature
//! selection in `RABS_TEST_WORKSPACE_ARGS` so Cargo verifies the already-built
//! test graph instead of rebuilding a second graph inside the first fixture.
//! Cargo still runs once per fixture process; an existing file is never proof
//! of freshness. Standalone fixtures retain the production daemon build.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, MutexGuard};

/// A fork can retain another test's CLOEXEC writer until its own exec. Pair
/// executable-file preparation with this short spawn guard to avoid ETXTBSY.
/// Child execution and waits must happen after the guard has been released.
pub fn process_setup_guard() -> MutexGuard<'static, ()> {
    static PREPARATION: Mutex<()> = Mutex::new(());
    PREPARATION
        .lock()
        .expect("process preparation lock poisoned")
}

pub fn spawn(command: &mut Command) -> std::io::Result<Child> {
    let _preparation = process_setup_guard();
    command.spawn()
}

/// Only package/feature selection belongs to the caller. It cannot redirect
/// the manifest, profile, target or build directory, execute tests recursively,
/// or omit the daemon whose freshness this preparation must establish.
pub(super) fn workspace_args(value: &str) -> Result<Vec<&str>, &'static str> {
    let args: Vec<_> = value.split_whitespace().collect();
    if args.first() != Some(&"--workspace") {
        return Err("workspace preparation requires --workspace");
    }
    let mut index = 1;
    while index < args.len() {
        match args[index] {
            "--exclude" => {
                index += 1;
                let name = args.get(index).ok_or("--exclude needs a package name")?;
                if *name == "rabsd"
                    || name.starts_with('-')
                    || !name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                {
                    return Err("excluded package must be a literal name other than rabsd");
                }
            }
            "--features" => {
                index += 1;
                if args
                    .get(index)
                    .is_none_or(|features| features.starts_with('-'))
                {
                    return Err("--features needs a feature list");
                }
            }
            "--all-features" | "--no-default-features" => {}
            _ => return Err("unsupported workspace preparation argument"),
        }
        index += 1;
    }
    Ok(args)
}

pub fn rabsd_bin() -> PathBuf {
    static BUILT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let mut build = Command::new(env!("CARGO"));
        match std::env::var("RABS_TEST_WORKSPACE_ARGS") {
            Ok(value) => {
                let args = workspace_args(&value).expect("invalid RABS_TEST_WORKSPACE_ARGS");
                build.args(["test", "--no-run"]).args(args);
            }
            Err(std::env::VarError::NotPresent) => {
                build.args(["build", "-p", "rabsd", "--bin", "rabsd"]);
            }
            Err(error) => panic!("invalid RABS_TEST_WORKSPACE_ARGS: {error}"),
        }
        build.arg("--locked");
        if !cfg!(debug_assertions) {
            build.arg("--release");
        }
        // Parent-package metadata must not invalidate dependency build scripts.
        // Preserve compiler/instrumentation flags and the target directory.
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| {
                name.starts_with("CARGO_PKG_")
                    || name.starts_with("CARGO_BIN_EXE_")
                    || matches!(name, "CARGO_MANIFEST_DIR" | "CARGO_MANIFEST_PATH")
            }) {
                build.env_remove(name);
            }
        }
        let status = spawn(&mut build)
            .expect("spawn daemon preparation")
            .wait()
            .expect("wait for daemon preparation");
        assert!(status.success(), "rabsd preparation failed");
    });
    Path::new(env!("CARGO_BIN_EXE_rabs-wrap")).with_file_name("rabsd")
}
