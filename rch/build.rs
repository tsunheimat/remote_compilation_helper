//! Avoid eagerly loading unused transitive frameworks for every agent hook.
//!
//! The published macOS release linked IOKit, OpenDirectory, Foundation and
//! SystemConfiguration despite importing no symbols from them. Apple ld can
//! omit unused dylib load commands while retaining libraries that supply live
//! symbols (including native TLS verification). RCH does not intentionally
//! depend on initializer-only libraries. Keep this scoped to the CLI binary;
//! changing TLS features, certificates or daemon/worker link policy is not a
//! startup optimization.

fn cli_link_instruction(target_os: &str) -> Option<&'static str> {
    (target_os == "macos").then_some("cargo:rustc-link-arg-bin=rch=-Wl,-dead_strip_dylibs")
}

#[cfg(not(test))]
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Cargo's target is authoritative; cfg!(target_os) here describes the host
    // running this build script and is wrong when cross-compiling.
    if let Some(instruction) = cli_link_instruction(
        &std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo must supply the target OS"),
    ) {
        println!("{instruction}");
    }
}

#[cfg(test)]
mod tests {
    use super::cli_link_instruction;

    #[test]
    fn target_os_not_build_host_controls_the_cli_only_link_option() {
        assert_eq!(
            cli_link_instruction("macos"),
            Some("cargo:rustc-link-arg-bin=rch=-Wl,-dead_strip_dylibs")
        );
        for other in ["linux", "windows", "freebsd", "ios", "", "darwin"] {
            assert_eq!(cli_link_instruction(other), None, "{other}");
        }
    }
}
