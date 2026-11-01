//! One validated package/feature selection for both daemon fixture builders.
//!
//! An enclosing workspace test has a different unified dependency graph from
//! `cargo build -p rabsd`. The two preparation consumers must agree on which
//! selection may be reused without allowing target/profile/manifest redirection.

/// Only package/feature selection belongs to the caller. It cannot redirect
/// the manifest, profile, target or build directory, execute tests recursively,
/// or omit the daemon whose freshness this preparation must establish.
pub(crate) fn workspace_args(value: &str) -> Result<Vec<&str>, &'static str> {
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
