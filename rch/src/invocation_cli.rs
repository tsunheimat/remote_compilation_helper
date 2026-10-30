//! CLI adaptation for the common invocation contract and generic tool dispatch.

use anyhow::{Context, Result};
use rch_common::invocation::{CommandSpec, InvocationOptions, WorkspaceMode, parse_env_assignment};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

pub fn parse_timeout(text: &str) -> Result<u64, String> {
    let duration = if text.bytes().all(|byte| byte.is_ascii_digit()) {
        std::time::Duration::from_secs(text.parse().map_err(|_| "invalid timeout")?)
    } else {
        humantime::parse_duration(text).map_err(|error| error.to_string())?
    };
    let seconds = duration
        .as_secs()
        .checked_add(u64::from(duration.subsec_nanos() > 0))
        .ok_or("timeout overflow")?;
    if seconds == 0 {
        return Err("timeout must be positive".into());
    }
    Ok(seconds)
}

#[allow(clippy::too_many_arguments)]
pub fn options(
    verification: bool,
    cwd: Option<PathBuf>,
    assignments: Vec<String>,
    forward: Vec<String>,
    shell: Option<String>,
    timeout_seconds: Option<u64>,
    interactive: bool,
    json_result: Option<PathBuf>,
    workspace: Option<String>,
    command: &[String],
) -> Result<Option<InvocationOptions>> {
    let explicit = verification
        || cwd.is_some()
        || !assignments.is_empty()
        || !forward.is_empty()
        || shell.is_some()
        || timeout_seconds.is_some()
        || interactive
        || json_result.is_some()
        || workspace.is_some();
    if !explicit {
        return Ok(None);
    }
    let ambient = std::env::current_dir()?;
    let cwd = cwd.unwrap_or_else(|| ambient.clone());
    let cwd = if cwd.is_absolute() {
        cwd
    } else {
        ambient.join(cwd)
    };
    let cwd = cwd
        .canonicalize()
        .context("invocation cwd is unavailable")?;
    anyhow::ensure!(cwd.is_dir(), "invocation cwd must be a directory");
    let mut env = BTreeMap::new();
    for name in forward {
        parse_env_assignment(&format!("{name}=")).map_err(anyhow::Error::msg)?;
        if let Some(value) = std::env::var_os(&name) {
            env.insert(
                name,
                value
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("non-UTF-8 forwarded environment"))?,
            );
        }
    }
    for assignment in assignments {
        let (key, value) = parse_env_assignment(&assignment).map_err(anyhow::Error::msg)?;
        env.insert(key, value);
    }
    let command = match shell {
        Some(source) => {
            anyhow::ensure!(command.is_empty(), "--shell conflicts with argv");
            CommandSpec::Shell(source)
        }
        None => CommandSpec::Argv(command.to_vec()),
    };
    let workspace = match workspace.as_deref() {
        Some("mirror") => WorkspaceMode::Mirror,
        Some("shared") => WorkspaceMode::Shared,
        None => WorkspaceMode::Shared,
        _ => anyhow::bail!("unsupported workspace mode"),
    };
    let request = InvocationOptions {
        command,
        cwd,
        env,
        timeout_seconds,
        interactive,
        workspace,
        verification,
        json_result: json_result.map(|path| {
            if path.is_absolute() {
                path
            } else {
                ambient.join(path)
            }
        }),
    };
    request.validate().map_err(anyhow::Error::msg)?;
    Ok(Some(request))
}

pub fn run_ci(command: Vec<OsString>) -> Result<()> {
    let (program, args) = command.split_first().context("CI command is empty")?;
    let home = dirs::home_dir().context("cannot resolve shim home")?;
    let mut paths = vec![home.join(".rch/shims")];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut child = std::process::Command::new(program);
    child.args(args).env("PATH", std::env::join_paths(paths)?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(child.exec()).context("cannot launch CI command")
    }
    #[cfg(not(unix))]
    {
        let status = child.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

pub async fn dispatch_tool(tool: String, args: Vec<OsString>) -> Result<()> {
    anyhow::ensure!(
        crate::commands::shim::GENERIC_TOOLS.contains(&tool.as_str()),
        "unsupported shim tool"
    );
    let real = PathBuf::from(
        std::env::var_os("RCH_SHIM_REAL_TOOL").context("shim did not provide resolved tool")?,
    );
    anyhow::ensure!(
        real.is_absolute() && real.is_file(),
        "shim real executable is unavailable"
    );
    // Non-UTF-8 arguments cannot be sent through the JSON planning protocol.
    // They remain literal local argv instead of lossy replacement characters.
    let utf8_args = args
        .iter()
        .map(|arg| arg.to_str())
        .collect::<Option<Vec<_>>>();
    let remote = if let Some(arguments) = &utf8_args {
        let source =
            shell_words::join(std::iter::once(tool.as_str()).chain(arguments.iter().copied()));
        #[cfg(unix)]
        {
            match crate::hook::semantic_plan(&source).await {
                Ok(plan) => plan.is_some(),
                Err(error) => {
                    // Failure to classify is not a successful ordinary-tool
                    // decision. Resolve the same policy used by the agent hook
                    // before allowing an unguarded interpreter/tool launch.
                    let config = crate::config::load_config().context(
                        "RCH shim cannot resolve fallback policy after planner failure; refusing local execution",
                    )?;
                    if crate::hook::planner_failure_requires_refusal(&config) {
                        return Err(error).context(
                            "RCH shim planner failed; refusing local execution under remote-only or no-local-fallback policy",
                        );
                    }
                    eprintln!(
                        "[RCH] semantic planning unavailable; preserving local argv under non-strict fallback policy: {error:#}"
                    );
                    false
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = source;
            false
        }
    } else {
        false
    };
    if remote {
        let mut process = std::process::Command::new(std::env::current_exe()?);
        // Worker resolves its own native tool, not the controller's absolute path.
        process
            .args(["exec", "--verification", "--", &tool])
            .args(&args)
            .env("RCH_SHIM_REAL_TOOL", &real);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            return Err(process.exec()).context("cannot launch verification dispatch");
        }
        #[cfg(not(unix))]
        {
            std::process::exit(process.status()?.code().unwrap_or(1));
        }
    }
    // Informational/interpreter launch is not itself remote verification. Do
    // not export a global bypass: its nested test commands still need shims.
    let mut process = std::process::Command::new(real);
    process.args(args).env_remove("RCH_SHIM_REAL_TOOL");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(process.exec()).context("cannot launch ordinary tool command")
    }
    #[cfg(not(unix))]
    {
        std::process::exit(process.status()?.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeout_units_round_up_not_down() {
        assert_eq!(parse_timeout("1ms").unwrap(), 1);
        assert_eq!(parse_timeout("2m").unwrap(), 120);
        assert_eq!(parse_timeout("7").unwrap(), 7);
        for value in ["0", "0s", "-1", "forever"] {
            assert!(parse_timeout(value).is_err());
        }
    }
    #[test]
    fn explicit_single_argv_stays_single_argv() {
        let args = vec!["program with spaces".to_string()];
        let request = options(
            true,
            None,
            vec!["A=a=b".into()],
            vec![],
            None,
            None,
            false,
            None,
            None,
            &args,
        )
        .unwrap()
        .unwrap();
        assert_eq!(request.command, CommandSpec::Argv(args));
        assert_eq!(request.workspace, WorkspaceMode::Shared);
        assert_eq!(request.env["A"], "a=b");
    }
}
