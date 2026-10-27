//! Explicit command semantics shared by CLI, semantic hooks and tool shims.
//! Shell source is never inferred from the number or contents of argv entries.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
pub enum CommandSpec {
    Argv(Vec<String>),
    Shell(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    #[default]
    Mirror,
    Shared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationOptions {
    pub command: CommandSpec,
    /// Absolute controller cwd, resolved before configuration lookup or admission.
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub timeout_seconds: Option<u64>,
    pub interactive: bool,
    pub workspace: WorkspaceMode,
    pub verification: bool,
    /// Result file, independent of the workload's stdout/stderr.
    pub json_result: Option<PathBuf>,
}

impl InvocationOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !self.cwd.is_absolute() {
            return Err("invocation cwd must be absolute".into());
        }
        match &self.command {
            CommandSpec::Argv(args) if args.is_empty() || args[0].is_empty() => {
                return Err("invocation argv requires an executable".into());
            }
            CommandSpec::Argv(args) if args.iter().any(|arg| arg.contains('\0')) => {
                return Err("invocation argv contains NUL".into());
            }
            CommandSpec::Shell(source) if source.trim().is_empty() || source.contains('\0') => {
                return Err("invocation shell source must be nonempty and NUL-free".into());
            }
            _ => {}
        }
        if self.timeout_seconds == Some(0) {
            return Err("invocation timeout must be positive".into());
        }
        if self.env.len() > 128 {
            return Err("at most 128 explicit environment variables are supported".into());
        }
        for (key, value) in &self.env {
            validate_env_pair(key, value)?;
        }
        Ok(())
    }

    /// Render only at an explicit POSIX-shell transport boundary. Do not split
    /// a lone argv element: e.g. a filename with spaces remains one argument.
    pub fn shell_command(&self) -> String {
        match &self.command {
            CommandSpec::Shell(source) => source.clone(),
            CommandSpec::Argv(args) => args
                .iter()
                .map(|arg| shell_escape::escape(arg.as_str().into()).into_owned())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

pub fn parse_env_assignment(value: &str) -> Result<(String, String), String> {
    let (key, value) = value.split_once('=').ok_or("expected KEY=VALUE")?;
    validate_env_pair(key, value)?;
    Ok((key.into(), value.into()))
}

fn validate_env_pair(key: &str, value: &str) -> Result<(), String> {
    let mut bytes = key.bytes();
    if !bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || value.contains('\0')
    {
        return Err("invalid environment name or NUL in value".into());
    }
    Ok(())
}

/// Configured correspondence, not by itself proof that the backing tree is shared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SharedWorkspaceMapping {
    pub controller_root: PathBuf,
    pub worker_root: PathBuf,
    /// Worker id; mappings never silently authorize another worker.
    pub worker: String,
}

impl SharedWorkspaceMapping {
    pub fn map_cwd(&self, cwd: &Path, worker: &str) -> Result<PathBuf, String> {
        if worker != self.worker || self.worker.is_empty() {
            return Err("shared workspace mapping does not authorize this worker".into());
        }
        for path in [&self.controller_root, &self.worker_root, cwd] {
            if !path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(
                    "shared workspace paths must be absolute without parent traversal".into(),
                );
            }
        }
        let relative = cwd
            .strip_prefix(&self.controller_root)
            .map_err(|_| "cwd is outside the configured shared workspace")?;
        Ok(self.worker_root.join(relative))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_argv_is_not_inferred_shell_source() {
        let request = InvocationOptions {
            command: CommandSpec::Argv(vec![
                "printf".into(),
                "%s".into(),
                "a b; $(printf expanded)".into(),
            ]),
            cwd: PathBuf::from("/project"),
            env: BTreeMap::new(),
            timeout_seconds: None,
            interactive: false,
            workspace: WorkspaceMode::Shared,
            verification: true,
            json_result: None,
        };
        assert!(request.validate().is_ok());
        let rendered = request.shell_command();
        assert_eq!(rendered, "printf '%s' 'a b; $(printf expanded)'");
        #[cfg(unix)]
        {
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", &rendered])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, b"a b; $(printf expanded)");
            assert!(output.stderr.is_empty());
        }
        assert_eq!(
            serde_json::from_str::<InvocationOptions>(&serde_json::to_string(&request).unwrap())
                .unwrap(),
            request
        );
    }

    #[test]
    fn invocation_validation_rejects_invalid_boundaries() {
        let request = InvocationOptions {
            command: CommandSpec::Argv(vec!["true".into()]),
            cwd: PathBuf::from("/project"),
            env: BTreeMap::new(),
            timeout_seconds: Some(1),
            interactive: false,
            workspace: WorkspaceMode::Shared,
            verification: true,
            json_result: None,
        };
        assert!(request.validate().is_ok());
        for command in [
            CommandSpec::Argv(vec![]),
            CommandSpec::Argv(vec![String::new()]),
            CommandSpec::Argv(vec!["true".into(), "bad\0argument".into()]),
            CommandSpec::Shell(" \n\t".into()),
            CommandSpec::Shell("true\0false".into()),
        ] {
            let mut invalid = request.clone();
            invalid.command = command;
            assert!(invalid.validate().is_err());
        }
        let mut invalid = request.clone();
        invalid.cwd = PathBuf::from("relative");
        assert!(invalid.validate().is_err());
        invalid = request.clone();
        invalid.timeout_seconds = Some(0);
        assert!(invalid.validate().is_err());
        invalid = request.clone();
        invalid.env = (0..128)
            .map(|index| (format!("KEY_{index}"), String::new()))
            .collect();
        assert!(invalid.validate().is_ok());
        invalid.env.insert("OVER_LIMIT".into(), String::new());
        assert!(invalid.validate().is_err());
        invalid = request;
        invalid.env.insert("BAD-NAME".into(), String::new());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn environment_values_preserve_equals_and_empty_values() {
        assert_eq!(
            parse_env_assignment("A=a=b").unwrap(),
            ("A".into(), "a=b".into())
        );
        assert_eq!(parse_env_assignment("A=").unwrap(), ("A".into(), "".into()));
        for value in ["A", "=x", "1A=x", "A-B=x", "A=x\0"] {
            assert!(parse_env_assignment(value).is_err(), "{value:?}");
        }
    }

    #[test]
    fn mapping_rejects_other_workers_and_sibling_paths() {
        let mapping = SharedWorkspaceMapping {
            controller_root: PathBuf::from("/share/project"),
            worker_root: PathBuf::from("/worker/project"),
            worker: "builder".into(),
        };
        assert_eq!(
            mapping
                .map_cwd(Path::new("/share/project/sub"), "builder")
                .unwrap(),
            PathBuf::from("/worker/project/sub")
        );
        assert!(
            mapping
                .map_cwd(Path::new("/share/project2"), "builder")
                .is_err()
        );
        assert!(
            mapping
                .map_cwd(Path::new("/share/project/../other"), "builder")
                .is_err()
        );
        assert!(
            mapping
                .map_cwd(Path::new("/share/project"), "other")
                .is_err()
        );
    }
}
