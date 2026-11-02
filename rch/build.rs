//! Avoid eagerly loading unused transitive frameworks for every agent hook.
//!
//! The published macOS release linked IOKit, OpenDirectory, Foundation and
//! SystemConfiguration despite importing no symbols from them. Apple ld can
//! omit unused dylib load commands while retaining libraries that supply live
//! symbols (including native TLS verification). RCH does not intentionally
//! depend on initializer-only libraries. Keep this scoped to the CLI binary;
//! changing TLS features, certificates or daemon/worker link policy is not a
//! startup optimization.

fn cli_link_instructions(target_os: &str, target_arch: &str) -> &'static [&'static str] {
    match (target_os, target_arch) {
        // ARM macOS already requires 11.0, the minimum supporting this format
        // (Xcode 13.3 release notes / LLVM MachO shouldEmitChainedFixups).
        // The observed ARM release still carried eager classic rebase tables.
        // Chained fixups allow dyld's page-in fixups without changing imports,
        // TLS, panic policy or the deployment target. Intel can target pre-11
        // systems and deliberately retains its existing link format.
        ("macos", "aarch64") => &[
            "cargo:rustc-link-arg-bin=rch=-Wl,-dead_strip_dylibs",
            "cargo:rustc-link-arg-bin=rch=-Wl,-fixup_chains",
        ],
        ("macos", _) => &["cargo:rustc-link-arg-bin=rch=-Wl,-dead_strip_dylibs"],
        _ => &[],
    }
}

#[cfg(not(test))]
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Cargo's target is authoritative; cfg!(target_os) here describes the host
    // running this build script and is wrong when cross-compiling.
    for instruction in cli_link_instructions(
        &std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo must supply the target OS"),
        &std::env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo must supply the target architecture"),
    ) {
        println!("{instruction}");
    }
}

#[cfg(test)]
mod tests {
    use super::cli_link_instructions;

    #[test]
    fn target_os_not_build_host_controls_the_cli_only_link_option() {
        assert_eq!(
            cli_link_instructions("macos", "aarch64"),
            [
                "cargo:rustc-link-arg-bin=rch=-Wl,-dead_strip_dylibs",
                "cargo:rustc-link-arg-bin=rch=-Wl,-fixup_chains"
            ]
        );
        for arch in ["x86_64", "i686", "", "arm"] {
            assert_eq!(
                cli_link_instructions("macos", arch),
                ["cargo:rustc-link-arg-bin=rch=-Wl,-dead_strip_dylibs"]
            );
        }
        for other in ["linux", "windows", "freebsd", "ios", "", "darwin"] {
            for arch in ["aarch64", "x86_64"] {
                assert!(
                    cli_link_instructions(other, arch).is_empty(),
                    "{other}/{arch}"
                );
            }
        }
    }
}
