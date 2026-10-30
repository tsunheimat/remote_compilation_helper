//! Exercise the actual build script and, on macOS, the actual linked CLI.
//! These checks do not substitute Linux execution for native loader timing.

#[path = "../build.rs"]
mod build_script;

#[cfg(target_os = "macos")]
mod native {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn cli_retains_native_tls_and_plain_hook_behavior_after_dylib_stripping() {
        let binary = env!("CARGO_BIN_EXE_rch");
        let output = Command::new("/usr/bin/otool")
            .args(["-L", binary])
            .output()
            .expect("inspect the actual native CLI load commands");
        assert!(output.status.success(), "otool failed: {output:?}");
        let libraries = String::from_utf8(output.stdout).unwrap();
        eprintln!("native CLI load commands:\n{libraries}");
        // The optimization must not disable the native trust store/verifier.
        // Other dylibs with genuinely live references are intentionally left
        // to the linker, not prohibited through a hard-coded allowlist.
        assert!(libraries.contains("/Security.framework/"), "{libraries}");
        assert!(
            libraries.contains("/CoreFoundation.framework/"),
            "{libraries}"
        );
        assert!(
            libraries.contains("/usr/lib/libSystem.B.dylib"),
            "{libraries}"
        );

        let root = tempfile::tempdir().unwrap().keep();
        let mut child = Command::new(binary)
            .env_clear()
            .env("HOME", &root)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"git status\"}}\n")
            .unwrap();
        let response = child.wait_with_output().unwrap();
        assert!(response.status.success(), "{response:?}");
        assert!(response.stdout.is_empty(), "{response:?}");
        assert!(response.stderr.is_empty(), "{response:?}");
    }
}
