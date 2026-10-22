//! Shared filesystem execution: admission and completion only, never mirror transfer.
use super::super::ssh::acquire_remote_source_authority_lock;
use super::*;
use rch_common::invocation::SharedWorkspaceMapping;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

/// Resolve the actual controller path before matching an explicitly authorized worker.
fn shared_mapping<'a>(
    mappings: &'a [SharedWorkspaceMapping],
    cwd: &Path,
    worker: &str,
) -> anyhow::Result<(&'a SharedWorkspaceMapping, PathBuf, PathBuf)> {
    let cwd = std::fs::canonicalize(cwd).context("resolve shared invocation cwd")?;
    let mut matches = mappings
        .iter()
        .filter_map(|mapping| {
            let remote = mapping.map_cwd(&cwd, worker).ok()?;
            Some((mapping, remote))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(mapping, _)| {
        std::cmp::Reverse(mapping.controller_root.components().count())
    });
    let (mapping, remote) = matches
        .into_iter()
        .next()
        .context("no explicit shared workspace mapping authorizes this cwd and selected worker")?;
    anyhow::ensure!(
        std::fs::canonicalize(&mapping.controller_root)? == mapping.controller_root,
        "shared controller root must use its canonical spelling"
    );
    Ok((mapping, cwd, remote))
}

fn source_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|root| root.join(".git").exists())
        .unwrap_or(cwd)
        .to_owned()
}

fn control_root(mapping: &SharedWorkspaceMapping, source: &Path) -> anyhow::Result<PathBuf> {
    let root = mapping.controller_root.join(".rch-control");
    anyhow::ensure!(
        !root.starts_with(source),
        "shared mapping must include a control location outside source inputs; configure its enclosing shared volume"
    );
    Ok(root)
}

fn quote_path(path: &Path) -> anyhow::Result<String> {
    let value = path.to_str().context("shared paths must be UTF-8")?;
    anyhow::ensure!(!value.contains('\0'), "shared path contains NUL");
    Ok(shell_escape::escape(value.into()).into_owned())
}

fn private_directory(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("private directory requires a parent")?;
    anyhow::ensure!(
        path.is_absolute() && std::fs::canonicalize(parent)? == parent,
        "shared control parent must already exist without symlinks"
    );
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => validate_private_directory(path, &metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new().mode(0o700).create(path)?;
        }
        Err(error) => return Err(error.into()),
    }
    validate_private_directory(path, &std::fs::symlink_metadata(path)?)
}

fn validate_private_directory(path: &Path, metadata: &std::fs::Metadata) -> anyhow::Result<()> {
    anyhow::ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.mode() & 0o077 == 0,
        "shared control/tee directory must be owned, non-symlink and mode 0700: {}",
        path.display()
    );
    Ok(())
}

pub(in crate::hook) fn uses_rtk(invocation: &InvocationOptions) -> anyhow::Result<bool> {
    uses_rtk_depth(invocation, 0)
}

const RTK_CAPTURE: &str = include_str!("rtk_capture.py");

/// Captured output belongs to this job, not to a shared source mapping. The
/// remote copy is retained for collection-only recovery after transport loss.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct RtkOutput {
    remote: PathBuf,
    local: PathBuf,
    identity: String,
    collected: bool,
    #[serde(default)]
    remote_directory: Option<(u64, u64)>,
}

impl RtkOutput {
    pub(crate) fn validate(
        &self,
        source: &Path,
        sources: &[String],
        identity: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.identity == identity && uuid::Uuid::parse_str(identity).is_ok(),
            "RTK output identity differs from its job"
        );
        for path in [&self.remote, &self.local] {
            anyhow::ensure!(
                path.is_absolute()
                    && !path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir)),
                "invalid RTK output path"
            );
            anyhow::ensure!(
                path.file_name().and_then(|p| p.to_str()) == Some(identity),
                "RTK output is not a job directory"
            );
        }
        anyhow::ensure!(
            !self.local.starts_with(source)
                && sources.iter().all(|root| !self.remote.starts_with(root)),
            "RTK output must be outside source inputs"
        );
        Ok(())
    }

    fn command(&self, operation: &str, args: &[String]) -> anyhow::Result<String> {
        Ok(shell_words::join(
            std::iter::once("python3".to_owned())
                .chain([
                    "-I".into(),
                    "-c".into(),
                    RTK_CAPTURE.into(),
                    operation.into(),
                    self.remote
                        .to_str()
                        .context("RTK output path must be UTF-8")?
                        .into(),
                    self.identity.clone(),
                ])
                .chain(args.iter().cloned()),
        ))
    }

    pub(crate) async fn prepare(&mut self, worker: &WorkerConfig) -> anyhow::Result<()> {
        let output = super::super::ssh::run_offload_ssh_command(
            worker,
            &self.command("prepare", &[RTK_CAPTURE.into()])?,
            Duration::from_secs(15),
        )
        .await?;
        anyhow::ensure!(
            output.status.success(),
            "RTK output preparation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.remote_directory = Some(serde_json::from_slice(&output.stdout)?);
        Ok(())
    }

    fn wrap(&self, options: &mut InvocationOptions) -> anyhow::Result<()> {
        let command = serde_json::to_string(&options.command)?;
        options.command = rch_common::invocation::CommandSpec::Argv(vec![
            "python3".into(),
            "-I".into(),
            self.remote
                .join(".capture.py")
                .to_str()
                .context("RTK output path must be UTF-8")?
                .into(),
            "run".into(),
            self.remote
                .to_str()
                .context("RTK output path must be UTF-8")?
                .into(),
            self.identity.clone(),
            command,
        ]);
        Ok(())
    }

    pub(crate) async fn collect(
        &mut self,
        worker: &WorkerConfig,
    ) -> anyhow::Result<ExecResultDirStat> {
        if self.collected {
            let mut stat = result_stat(&self.local, Path::new("."))?;
            stat.path = self.local.display().to_string();
            stat.status = "retrieved".into();
            eprintln!("[RCH] full RTK output: {}", self.local.display());
            return Ok(stat);
        }
        use sha2::{Digest, Sha256};
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Entry {
            name: String,
            size: u64,
            sha256: String,
        }
        let parent = self.local.parent().context("RTK output parent")?;
        // Reject symlink ancestors before creating any controller output.
        let mut checked = PathBuf::from("/");
        for component in parent.components().skip(1) {
            checked.push(component);
            match std::fs::symlink_metadata(&checked) {
                Ok(metadata) => anyhow::ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "unsafe RTK output ancestor"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::DirBuilder::new().mode(0o700).create(&checked)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        private_directory(&self.local)?;
        let output = super::super::ssh::run_offload_ssh_command(
            worker,
            &self.command(
                "manifest",
                &[serde_json::to_string(
                    &self
                        .remote_directory
                        .context("missing RTK directory identity")?,
                )?],
            )?,
            Duration::from_secs(30),
        )
        .await?;
        anyhow::ensure!(
            output.status.success(),
            "RTK output manifest failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let entries: Vec<Entry> = serde_json::from_slice(&output.stdout)?;
        anyhow::ensure!(entries.len() <= 256, "RTK output file limit exceeded");
        let mut total = 0_u64;
        let mut names = std::collections::HashSet::new();
        for entry in &entries {
            anyhow::ensure!(
                !entry.name.contains('/')
                    && !entry.name.starts_with('.')
                    && names.insert(entry.name.clone()),
                "unsafe RTK output filename"
            );
            total = total
                .checked_add(entry.size)
                .context("RTK output size overflow")?;
            anyhow::ensure!(total <= 65 * 1024 * 1024, "RTK output byte limit exceeded");
            let path = self.local.join(&entry.name);
            let mut bytes = Vec::new();
            while bytes.len() < entry.size as usize {
                let output = super::super::ssh::run_offload_ssh_command(
                    worker,
                    &self.command(
                        "read",
                        &[
                            entry.name.clone(),
                            bytes.len().to_string(),
                            serde_json::to_string(
                                &self
                                    .remote_directory
                                    .context("missing RTK directory identity")?,
                            )?,
                        ],
                    )?,
                    Duration::from_secs(30),
                )
                .await?;
                anyhow::ensure!(
                    output.status.success(),
                    "RTK output read failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let hex = std::str::from_utf8(output.stdout.trim_ascii())?;
                anyhow::ensure!(hex.len() % 2 == 0, "invalid RTK output encoding");
                let chunk = hex
                    .as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| Ok(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                anyhow::ensure!(
                    !chunk.is_empty()
                        && chunk.len() <= 1024 * 1024
                        && bytes.len() + chunk.len() <= entry.size as usize,
                    "RTK output length changed"
                );
                bytes.extend(chunk);
            }
            let hash = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            anyhow::ensure!(hash == entry.sha256, "RTK output changed during collection");
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    anyhow::ensure!(
                        metadata.is_file()
                            && !metadata.file_type().is_symlink()
                            && metadata.nlink() == 1
                            && metadata.len() == entry.size
                            && std::fs::read(&path)? == bytes,
                        "retained RTK output changed; refusing overwrite"
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let temporary = self
                        .local
                        .join(format!(".{}.pending", uuid::Uuid::new_v4()));
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&temporary)?;
                    file.write_all(&bytes)?;
                    file.sync_all()?;
                    std::fs::rename(temporary, &path)?;
                    File::open(&self.local)?.sync_all()?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        self.collected = true;
        eprintln!("[RCH] full RTK output: {}", self.local.display());
        Ok(ExecResultDirStat {
            path: self.local.display().to_string(),
            status: "retrieved".into(),
            files: entries.len() as u64,
            bytes: total,
        })
    }
}

fn shell_segments(source: &str) -> anyhow::Result<Vec<String>> {
    let mut segments = Vec::new();
    let mut segment = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in source.chars() {
        if escaped {
            segment.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            segment.push(ch);
            escaped = true;
            continue;
        }
        if let Some(open) = quote {
            segment.push(ch);
            if ch == open {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            segment.push(ch);
        } else if matches!(ch, ';' | '|' | '&' | '\n') {
            if !segment.trim().is_empty() {
                segments.push(std::mem::take(&mut segment));
            }
        } else {
            segment.push(ch);
        }
    }
    anyhow::ensure!(
        quote.is_none() && !escaped,
        "unterminated shell input in shared invocation"
    );
    if !segment.trim().is_empty() {
        segments.push(segment);
    }
    Ok(segments)
}

fn uses_rtk_depth(invocation: &InvocationOptions, depth: usize) -> anyhow::Result<bool> {
    use rch_common::invocation::CommandSpec;
    anyhow::ensure!(depth < 8, "RTK wrapper nesting exceeds inspection limit");
    anyhow::ensure!(
        !invocation.env.contains_key("RTK_TEE_DIR"),
        "RTK_TEE_DIR is managed; configure jobs.rtk_tee_root instead"
    );
    let shell_words;
    let is_shell = matches!(&invocation.command, CommandSpec::Shell(_));
    let args = match &invocation.command {
        CommandSpec::Shell(source) => {
            let segments = shell_segments(source)?;
            if segments.len() > 1 {
                let mut uses = false;
                for segment in segments {
                    let mut nested = invocation.clone();
                    nested.command = CommandSpec::Shell(segment);
                    uses |= uses_rtk_depth(&nested, depth + 1)?;
                }
                return Ok(uses);
            }
            // Inspect command positions only; quoted argument text stays data.
            shell_words = match shell_words::split(source) {
                Ok(words) => words,
                Err(_) => return Ok(false),
            };
            &shell_words
        }
        CommandSpec::Argv(args) => args,
    };
    let basename = |arg: &str| {
        Path::new(arg)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_owned()
    };
    let mut index = 0;
    let mut resets_environment = false;
    while let Some(command) = args.get(index) {
        index += 1;
        if is_shell && command.contains('=') {
            let (key, _) = rch_common::invocation::parse_env_assignment(command)
                .map_err(anyhow::Error::msg)?;
            anyhow::ensure!(
                key != "RTK_TEE_DIR",
                "authored RTK_TEE_DIR override is forbidden"
            );
            continue;
        }
        match basename(command).as_str() {
            "rtk" => {
                anyhow::ensure!(
                    !resets_environment,
                    "RTK wrapper resets its managed environment"
                );
                // Shell source nested under RTK run/proxy still must not
                // replace the RCH-owned output location or clear its policy.
                let mut nested = invocation.clone();
                let mut payload = index;
                while args.get(payload).is_some_and(|arg| {
                    matches!(
                        arg.as_str(),
                        "-v" | "-vv" | "-vvv" | "--verbose" | "--ultra-compact" | "--skip-env"
                    )
                }) {
                    payload += 1;
                }
                let mode = args.get(payload).map(String::as_str);
                payload += 1;
                while args
                    .get(payload)
                    .is_some_and(|arg| matches!(arg.as_str(), "--ultra-compact" | "--skip-env"))
                {
                    payload += 1;
                }
                if args.get(payload).is_some_and(|arg| arg == "--") {
                    payload += 1;
                }
                match mode {
                    Some("run") => {
                        let source = if args
                            .get(payload)
                            .is_some_and(|arg| matches!(arg.as_str(), "-c" | "--command"))
                        {
                            args.get(payload + 1)
                                .cloned()
                                .context("RTK run requires command source")?
                        } else if let Some(value) = args
                            .get(payload)
                            .and_then(|arg| arg.strip_prefix("--command="))
                        {
                            value.to_owned()
                        } else {
                            args[payload..].join(" ")
                        };
                        nested.command = CommandSpec::Shell(source);
                        uses_rtk_depth(&nested, depth + 1)?;
                    }
                    Some("proxy" | "test" | "err" | "summary") => {
                        nested.command = CommandSpec::Argv(args[payload..].to_vec());
                        uses_rtk_depth(&nested, depth + 1)?;
                    }
                    _ => {}
                }
                return Ok(true);
            }
            "command" | "exec" | "builtin" => {
                while let Some(option) = args.get(index).filter(|arg| arg.starts_with('-')) {
                    index += 1;
                    match option.as_str() {
                        "--" => break,
                        "-p" if command == "command" => {}
                        "-v" | "-V" if command == "command" => return Ok(false),
                        "-c" if command == "exec" => resets_environment = true,
                        "-l" if command == "exec" => {}
                        "-a" if command == "exec" => {
                            anyhow::ensure!(args.get(index).is_some(), "exec -a requires argv0");
                            index += 1;
                        }
                        _ => return Ok(false),
                    }
                }
            }
            "export" | "declare" | "typeset" | "local" | "readonly" | "unset" => {
                anyhow::ensure!(
                    !args[index..].iter().any(|arg| arg
                        .split_once('=')
                        .map_or(arg.as_str(), |(key, _)| key)
                        == "RTK_TEE_DIR"),
                    "shell builtin mutates managed RTK_TEE_DIR; use jobs.rtk_tee_root"
                );
                return Ok(false);
            }
            "bash" | "sh" => {
                let mut shell_index = index;
                while args
                    .get(shell_index)
                    .is_some_and(|arg| matches!(arg.as_str(), "--noprofile" | "--norc"))
                {
                    shell_index += 1;
                }
                if args
                    .get(shell_index)
                    .is_some_and(|arg| matches!(arg.as_str(), "-c" | "-lc"))
                {
                    let source = args
                        .get(shell_index + 1)
                        .context("shell wrapper requires command source")?;
                    let mut nested = invocation.clone();
                    nested.command = CommandSpec::Shell(source.clone());
                    let uses = uses_rtk_depth(&nested, depth + 1)?;
                    anyhow::ensure!(
                        !uses || !resets_environment,
                        "RTK shell wrapper resets its managed environment"
                    );
                    return Ok(uses);
                }
                return Ok(false);
            }
            "env" => {
                while let Some(arg) = args.get(index) {
                    if arg == "--" {
                        index += 1;
                        break;
                    }
                    if arg.contains('=') && !arg.starts_with('-') {
                        let (key, _) = rch_common::invocation::parse_env_assignment(arg)
                            .map_err(anyhow::Error::msg)?;
                        anyhow::ensure!(
                            key != "RTK_TEE_DIR",
                            "authored RTK_TEE_DIR override is forbidden"
                        );
                        index += 1;
                    } else if arg == "-u" || arg == "--unset" {
                        let name = args.get(index + 1).context("env unset requires a name")?;
                        anyhow::ensure!(
                            name != "RTK_TEE_DIR",
                            "authored RTK_TEE_DIR removal is forbidden"
                        );
                        index += 2;
                    } else if let Some(name) = arg.strip_prefix("--unset=") {
                        anyhow::ensure!(
                            name != "RTK_TEE_DIR",
                            "authored RTK_TEE_DIR removal is forbidden"
                        );
                        index += 1;
                    } else if matches!(arg.as_str(), "-i" | "--ignore-environment" | "-") {
                        resets_environment = true;
                        index += 1;
                    } else if matches!(arg.as_str(), "-C" | "--chdir" | "-a" | "--argv0") {
                        anyhow::ensure!(
                            args.get(index + 1).is_some(),
                            "env option requires a value"
                        );
                        index += 2;
                    } else if arg.starts_with("--chdir=")
                        || arg.starts_with("--argv0=")
                        || (arg.starts_with("-C") && arg.len() > 2)
                        || matches!(arg.as_str(), "-v" | "--debug")
                    {
                        index += 1;
                    } else if matches!(arg.as_str(), "-S" | "--split-string") {
                        let source = args
                            .get(index + 1)
                            .context("env split-string requires source")?;
                        let mut words =
                            shell_words::split(source).context("invalid env split-string")?;
                        words.extend_from_slice(&args[index + 2..]);
                        let mut nested = invocation.clone();
                        nested.command = CommandSpec::Argv(words);
                        let uses = uses_rtk_depth(&nested, depth + 1)?;
                        anyhow::ensure!(
                            !uses || !resets_environment,
                            "env split-string resets RTK environment"
                        );
                        return Ok(uses);
                    } else if arg.starts_with('-') {
                        // Unknown options do not make ordinary argument data a
                        // managed RTK invocation. Semantic admission owns them.
                        return Ok(false);
                    } else {
                        break;
                    }
                }
            }
            "nice" => {
                if let Some(arg) = args.get(index) {
                    if arg == "-n" || arg == "--adjustment" {
                        index += 2;
                    } else if arg.starts_with("--adjustment=")
                        || arg.strip_prefix('-').is_some_and(|value| {
                            !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                        })
                    {
                        index += 1;
                    }
                }
                if args.get(index).is_some_and(|arg| arg == "--") {
                    index += 1;
                }
            }
            "timeout" => {
                while let Some(arg) = args.get(index).filter(|arg| arg.starts_with('-')) {
                    index += 1;
                    match arg.as_str() {
                        "--" => break,
                        "-s" | "--signal" | "-k" | "--kill-after" => index += 1,
                        "--preserve-status" | "--foreground" | "-v" | "--verbose" => {}
                        value
                            if value.starts_with("--signal=")
                                || value.starts_with("--kill-after=") => {}
                        _ => {
                            anyhow::bail!("unsupported timeout wrapper option in shared invocation")
                        }
                    }
                }
                anyhow::ensure!(args.get(index).is_some(), "timeout requires a duration");
                index += 1;
            }
            _ => return Ok(false),
        }
    }
    Ok(false)
}

/// A nonce in the volume root does not prove a nested bind mount. Refuse every
/// descendant mount in the admitted mapping, including same-device bind mounts.
fn verify_mount_topology(mountinfo: &str, root: &Path) -> anyhow::Result<()> {
    fn decode(value: &str) -> anyhow::Result<PathBuf> {
        let mut bytes = Vec::new();
        let mut input = value.as_bytes().iter().copied();
        while let Some(byte) = input.next() {
            if byte == b'\\' {
                let digits = [input.next(), input.next(), input.next()];
                let mut decoded = 0_u16;
                for digit in digits {
                    let digit = digit.context("truncated mountinfo escape")?;
                    anyhow::ensure!((b'0'..=b'7').contains(&digit), "invalid mountinfo escape");
                    decoded = decoded * 8 + u16::from(digit - b'0');
                }
                bytes.push(u8::try_from(decoded)?);
            } else {
                bytes.push(byte);
            }
        }
        Ok(PathBuf::from(String::from_utf8(bytes)?))
    }
    let mut covering = false;
    for line in mountinfo.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        anyhow::ensure!(
            fields.len() >= 10 && fields.contains(&"-"),
            "malformed mountinfo"
        );
        let mounted = decode(fields[4])?;
        anyhow::ensure!(mounted.is_absolute(), "nonabsolute mountinfo entry");
        if root.starts_with(&mounted) {
            covering = true;
        }
        anyhow::ensure!(
            mounted == root || !mounted.starts_with(root),
            "shared mapping contains a nested mount at {}; authorize a single backing mount",
            mounted.display()
        );
    }
    anyhow::ensure!(covering, "shared mapping has no mount identity evidence");
    Ok(())
}

/// A fresh read in each direction proves this invocation's mapping, not a stale
/// marker from another mount. Evidence lives outside source inputs and is retained.
async fn verify_mapping(
    worker: &WorkerConfig,
    mapping: &SharedWorkspaceMapping,
    cwd: &Path,
    remote_cwd: &Path,
    control: &Path,
) -> anyhow::Result<()> {
    verify_mount_topology(
        &std::fs::read_to_string("/proc/self/mountinfo")
            .context("shared mapping verification requires Linux mountinfo")?,
        &mapping.controller_root,
    )?;
    let worker_mounts = super::super::ssh::run_offload_ssh_command(
        worker,
        "cat /proc/self/mountinfo",
        Duration::from_secs(15),
    )
    .await?;
    anyhow::ensure!(
        worker_mounts.status.success(),
        "worker mount identity unavailable: {}",
        String::from_utf8_lossy(&worker_mounts.stderr)
    );
    verify_mount_topology(
        std::str::from_utf8(&worker_mounts.stdout)?,
        &mapping.worker_root,
    )?;
    private_directory(control)?;
    anyhow::ensure!(
        std::fs::metadata(cwd)?.dev() == std::fs::metadata(control)?.dev(),
        "shared cwd and nonce control differ in filesystem identity"
    );
    let nonce = uuid::Uuid::new_v4().to_string();
    let evidence = control.join(&nonce);
    private_directory(&evidence)?;
    let probe = evidence.join("controller");
    let reply = evidence.join("worker");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&probe)?;
    file.write_all(nonce.as_bytes())?;
    file.sync_all()?;
    let remote_probe = mapping
        .map_cwd(&probe, worker.id.as_str())
        .map_err(anyhow::Error::msg)?;
    let remote_reply = mapping
        .map_cwd(&reply, worker.id.as_str())
        .map_err(anyhow::Error::msg)?;
    let root = quote_path(&mapping.worker_root)?;
    let remote = quote_path(remote_cwd)?;
    let command = format!(
        "set -eu; test \"$(cd -P -- {root} && pwd -P)\" = {root}; test \"$(cd -P -- {remote} && pwd -P)\" = {remote}; test \"$(stat -c %d -- {remote})\" = \"$(stat -c %d -- {probe})\"; test \"$(cat -- {probe})\" = '{nonce}'; (set -C; printf '%s' '{nonce}:worker' > {reply})",
        probe = quote_path(&remote_probe)?,
        reply = quote_path(&remote_reply)?,
    );
    let output =
        super::super::ssh::run_offload_ssh_command(worker, &command, Duration::from_secs(15))
            .await?;
    anyhow::ensure!(
        output.status.success(),
        "shared mapping nonce verification failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    anyhow::ensure!(
        std::fs::read(&reply)? == format!("{nonce}:worker").as_bytes(),
        "worker reply is not controller-visible in the configured shared mapping"
    );
    anyhow::ensure!(
        std::fs::canonicalize(cwd)? == cwd,
        "shared cwd changed during mapping verification"
    );
    Ok(())
}

pub(super) fn result_stat(
    root: &Path,
    declared: &Path,
) -> anyhow::Result<super::super::remote_result::ExecResultDirStat> {
    fn count(root: File) -> anyhow::Result<(u64, u64)> {
        use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, openat, statat};
        let mut files = 0_u64;
        let mut bytes = 0_u64;
        let mut entries = 0_usize;
        let started = std::time::Instant::now();
        let mut pending = vec![(root, 0_usize)];
        while let Some((directory, depth)) = pending.pop() {
            anyhow::ensure!(depth <= 64, "shared result exceeds directory depth limit");
            for entry in Dir::read_from(&directory)? {
                let entry = entry?;
                let name = entry.file_name();
                if matches!(name.to_bytes(), b"." | b"..") {
                    continue;
                }
                entries += 1;
                anyhow::ensure!(
                    entries <= 100_000 && started.elapsed() < Duration::from_secs(10),
                    "shared result exceeds bounded inspection budget"
                );
                let stat = statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
                match FileType::from_raw_mode(stat.st_mode) {
                    FileType::Directory => {
                        let child = openat(
                            &directory,
                            name,
                            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                            Mode::empty(),
                        )?;
                        pending.push((File::from(child), depth + 1));
                    }
                    FileType::RegularFile => {
                        files = files.saturating_add(1);
                        bytes = bytes.saturating_add(u64::try_from(stat.st_size)?);
                    }
                    _ => anyhow::bail!("shared result contains a symlink or non-regular entry"),
                }
            }
        }
        Ok((files, bytes))
    }
    anyhow::ensure!(
        !declared.is_absolute()
            && !declared
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir)),
        "shared result must be repository-relative"
    );
    use rustix::fs::{Mode, OFlags, open, openat};
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = File::from(open(root, flags, Mode::empty())?);
    for component in declared.components() {
        if let std::path::Component::Normal(name) = component {
            directory = File::from(openat(&directory, name, flags, Mode::empty())?);
        }
    }
    let (files, bytes) = count(directory)?;
    Ok(super::super::remote_result::ExecResultDirStat {
        path: declared.display().to_string(),
        files,
        bytes,
        status: "ok".into(),
    })
}

struct ObservedOutput<W> {
    sink: W,
    state: Arc<Mutex<BuildHeartbeatSnapshot>>,
    capture: Option<Vec<u8>>,
}

impl<W: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for ObservedOutput<W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let count = std::task::ready!(std::pin::Pin::new(&mut self.sink).poll_write(cx, bytes))?;
        if count > 0 {
            mark_heartbeat_progress(&self.state);
        }
        if let Some(capture) = self.capture.as_mut() {
            let remaining = (64 * 1024_usize).saturating_sub(capture.len());
            capture.extend_from_slice(&bytes[..count.min(remaining)]);
        }
        std::task::Poll::Ready(Ok(count))
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.sink).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.sink).poll_shutdown(cx)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_shared_invocation(
    worker: &SelectedWorker,
    invocation: &InvocationOptions,
    jobs: &rch_common::JobsConfig,
    transfer: TransferConfig,
    environment: &rch_common::EnvironmentConfig,
    execution_storage: &rch_common::execution_storage::ExecutionStorageConfig,
    reporter: &HookReporter,
    socket_path: &str,
    build_id: Option<u64>,
    wrapper_id: Option<&str>,
    lease: Option<&DurableLeaseWriter>,
    result_dirs: &[PathBuf],
) -> anyhow::Result<RemoteExecutionResult> {
    invocation.validate().map_err(anyhow::Error::msg)?;
    let worker = selected_worker_to_config(worker);
    anyhow::ensure!(
        !WorkerPlatform::from_worker(&worker).is_windows()
            && !super::super::ssh::should_skip_remote_preflight(&worker),
        "shared workspaces require a live POSIX worker"
    );
    let lease = lease.context("shared execution requires an admitted durable lease")?;
    let build_id = build_id.context("shared execution requires an admitted build identity")?;
    let heartbeat =
        BuildHeartbeatLoop::start(socket_path, build_id, &worker.id, wrapper_id, Some(lease));
    heartbeat.update_phase(
        BuildHeartbeatPhase::SyncUp,
        Some("shared_mapping_validation".into()),
    );
    heartbeat.flush().await;
    let (mapping, cwd, remote_cwd) =
        shared_mapping(&jobs.shared_workspaces, &invocation.cwd, worker.id.as_str())?;
    let source = source_root(&cwd);
    let control = control_root(mapping, &source)?;
    verify_mapping(&worker, mapping, &cwd, &remote_cwd, &control).await?;
    let mut options = invocation.clone();
    options.cwd = cwd.clone();
    anyhow::ensure!(
        !options.env.contains_key("RTK_TEE_DIR"),
        "explicit RTK_TEE_DIR is not allowed; configure jobs.rtk_tee_root"
    );
    let identity = uuid::Uuid::new_v4().to_string();
    let remote = remote_cwd
        .to_str()
        .context("shared worker cwd must be UTF-8")?
        .to_owned();
    let remote_source = mapping
        .map_cwd(&source, worker.id.as_str())
        .map_err(anyhow::Error::msg)?;
    let sources = vec![
        remote_source
            .to_str()
            .context("shared worker source root must be UTF-8")?
            .to_owned(),
    ];
    let tee = if uses_rtk(invocation)? {
        // This is worker storage, never evidence that the controller and
        // worker share the same output mount. Collection uses bounded SSH.
        let base = execution_storage
            .tmp_root()
            .unwrap_or_else(|| transfer.remote_base.clone());
        let local = jobs.rtk_tee_root.clone().unwrap_or_else(|| {
            rch_common::job_identity::default_job_lease_directory().join("rtk-output")
        });
        let output = RtkOutput {
            remote: PathBuf::from(base).join("rch-output").join(&identity),
            local: local.join(&identity),
            identity: identity.clone(),
            collected: false,
            remote_directory: None,
        };
        output.validate(&source, &sources, &identity)?;
        output.wrap(&mut options)?;
        Some(output)
    } else {
        None
    };
    recovery::RecoverySession::begin(
        lease,
        &worker,
        sources.clone(),
        None,
        None,
        None,
        transfer.clone(),
        cwd.clone(),
        identity.clone(),
    )?;
    let lock_timeout = Duration::from_secs(invocation.timeout_seconds.unwrap_or(3600));
    let mut sources_lock =
        acquire_remote_source_authority_lock(&worker, &sources, None, &identity, lock_timeout)
            .await?;
    let mut session = recovery::RecoverySession::prepare_shared(lease)?;
    session.declare_shared_results(result_dirs)?;
    session.declare_rtk_output(tee)?;
    session.prepare_rtk_output(&worker).await?;
    let pipeline =
        TransferPipeline::new(cwd.clone(), "shared-job".into(), identity.clone(), transfer)
            .with_remote_path_override(remote.clone())
            .with_build_id(Some(build_id))
            .with_env_allowlist(environment.allowlist.clone())
            .with_execution_environment(execution_storage.clone(), environment.remote.clone())?
            .with_invocation_options(options.clone())?
            .with_source_authority(identity)?;
    let pipeline = session.completion_pipeline(pipeline);
    heartbeat.set_remote_pgid_file(Some(TransferPipeline::remote_pgid_file_path_for_root(
        &remote, build_id,
    )));
    heartbeat.update_phase(
        BuildHeartbeatPhase::Execute,
        Some("shared_exec_start".into()),
    );
    heartbeat.flush().await;
    sources_lock.ensure_held()?;
    let workload_stderr = super::super::remote_result::workload_streams_to_stderr();
    let mut stdout = ObservedOutput {
        sink: if workload_stderr {
            crate::transfer::RawOutputSink::stderr()?
        } else {
            crate::transfer::RawOutputSink::stdout()?
        },
        state: heartbeat.shared_state(),
        capture: None,
    };
    let mut stderr = ObservedOutput {
        sink: crate::transfer::RawOutputSink::stderr()?,
        state: heartbeat.shared_state(),
        capture: Some(Vec::new()),
    };
    let transport = pipeline
        .prepare_raw_transport(&worker, &options.shell_command())
        .await?;
    session.starting_execution()?;
    let result = match pipeline
        .execute_remote_bytes(&worker, transport, &mut stdout, &mut stderr)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            if let Some(proof) = error.downcast_ref::<crate::transfer::RawTransportNotStarted>() {
                session.transport_not_started(proof)?;
                // The outer orchestrator drains preparation after these local
                // source holders drop, then releases this reservation as failed.
                return Err(error);
            }
            return Err(error.context(crate::transfer::RemoteExecutionUnconfirmed));
        }
    };
    let captured_stderr = stderr.capture.take().unwrap_or_default();
    // Restore stdio flags before diagnostics and final machine publication.
    drop(stderr);
    drop(stdout);
    let completed = read_completion_with_retry(&pipeline, &worker)
        .await
        .context(crate::transfer::RemoteExecutionUnconfirmed)?
        .context("shared execution has no durable completion; recover this attempt, never replay")
        .context(crate::transfer::RemoteExecutionUnconfirmed)?;
    anyhow::ensure!(
        completed == result.exit_code,
        crate::transfer::RemoteExecutionUnconfirmed
    );
    sources_lock
        .ensure_held()
        .context(crate::transfer::RemoteExecutionUnconfirmed)?;
    session.completed(completed)?;
    reporter.verbose(&format!(
        "[RCH] shared output: stdout={} bytes stderr={} bytes",
        result.stdout_bytes, result.stderr_bytes
    ));
    // No rsync, output-tree preparation, target reaper, or artifact publication.
    let mut stats = Vec::new();
    let mut delivery_error = None;
    for declared in result_dirs {
        match result_stat(&cwd, declared) {
            Ok(stat) => stats.push(stat),
            Err(error) => {
                delivery_error.get_or_insert(error);
            }
        }
    }
    match session.collect_rtk_output(&worker).await {
        Ok(Some(stat)) => stats.push(stat),
        Ok(None) => {}
        Err(error) => {
            delivery_error.get_or_insert(error.context(
                "RTK output retained remotely; use rch jobs recover for this wrapper; never replay",
            ));
        }
    }
    let delivery_failed = delivery_error.is_some();
    let exit_code = if delivery_failed {
        EXIT_ARTIFACT_TRANSFER_FAILED
    } else {
        result
            .termination
            .map_or(completed, crate::transfer::RawTermination::exit_code)
    };
    session.returned(exit_code)?;
    // Mark non-applicable retirement, then release only the authority grant.
    session.tree_retired()?;
    sources_lock.release().await?;
    session.sources_released()?;
    session.pair_released()?;
    session.retired()?;
    heartbeat
        .finish(
            BuildHeartbeatPhase::Finalize,
            Some(format!("shared_exit_{exit_code}")),
        )
        .await;
    if let Some(error) = delivery_error {
        reporter.summary_critical(&format!("[RCH] shared result unavailable: {error:#}"));
    }
    Ok(RemoteExecutionResult {
        deadline_triggered: result.termination == Some(crate::transfer::RawTermination::Deadline),
        interrupted: result.termination == Some(crate::transfer::RawTermination::Interrupted),
        exit_code,
        stderr: String::from_utf8_lossy(&captured_stderr).into_owned(),
        duration_ms: result.duration_ms,
        timing: CommandTimingBreakdown {
            exec: Some(Duration::from_millis(result.duration_ms)),
            ..Default::default()
        },
        result_dirs: stats,
        disk_roots: vec![remote],
        cargo_stdout: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtk_capture_rejects_replaced_directories_and_linked_output() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap().keep().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let job = root.join("job");
        let identity = uuid::Uuid::new_v4().to_string();
        let invoke = |operation: &str, args: &[String]| {
            std::process::Command::new("/usr/bin/python3")
                .args([
                    "-I",
                    "-c",
                    RTK_CAPTURE,
                    operation,
                    job.to_str().unwrap(),
                    &identity,
                ])
                .args(args)
                .output()
                .unwrap()
        };
        let prepared = invoke("prepare", &[RTK_CAPTURE.into()]);
        assert!(prepared.status.success(), "{prepared:?}");
        let directory_identity = String::from_utf8(prepared.stdout).unwrap();
        std::fs::write(job.join("outer.stdout"), b"retained").unwrap();
        std::fs::write(job.join("outer.stderr"), b"").unwrap();
        assert!(
            invoke("manifest", std::slice::from_ref(&directory_identity))
                .status
                .success()
        );
        assert!(
            invoke(
                "read",
                &[
                    "outer.stdout".into(),
                    "0".into(),
                    directory_identity.clone()
                ]
            )
            .status
            .success()
        );
        assert!(
            !invoke("read", &["outer.stdout".into(), "0".into(), "[0,0]".into()])
                .status
                .success()
        );
        assert!(!invoke("manifest", &["[0,0]".into()]).status.success());
        let foreign = root.join("foreign");
        std::fs::write(&foreign, b"must not read").unwrap();
        symlink(&foreign, job.join("outer.json")).unwrap();
        assert!(!invoke("manifest", &[directory_identity]).status.success());
        assert!(
            !invoke("read", &["../foreign".into(), "0".into(), "[0,0]".into()])
                .status
                .success()
        );
        assert_eq!(std::fs::read(foreign).unwrap(), b"must not read");
        // Preparing the same name again cannot replace an earlier job.
        assert!(!invoke("prepare", &[RTK_CAPTURE.into()]).status.success());
        assert_eq!(
            std::fs::read(job.join("outer.stdout")).unwrap(),
            b"retained"
        );
    }

    #[test]
    fn rtk_capture_budget_refuses_incomplete_logs_and_excess_commands() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap().keep().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new("/usr/bin/python3")
            .args([
                "-I",
                "-c",
                r#"
import json,pathlib,subprocess,sys,uuid
script,root=sys.argv[1:];root=pathlib.Path(root)
for kind in ['bytes','files']:
    job=root/kind;identity=str(uuid.uuid4())
    base=[sys.executable,'-I','-c',script]
    prepare=subprocess.run(base+['prepare',str(job),identity,script],capture_output=True,check=True)
    if kind=='bytes':
        (job/'.budget').write_text(str(64*1024*1024-4))
    else:
        for _ in range(254): (job/(str(uuid.uuid4())+'.stdout')).write_bytes(b'')
    marker=root/(kind+'-executed')
    request=json.dumps({'mode':'argv','value':[sys.executable,'-c',f'from pathlib import Path;Path({str(marker)!r}).write_text("x");print("overflow")']})
    run=subprocess.run(base+['run',str(job),identity,request],capture_output=True,start_new_session=True,timeout=10)
    assert run.returncode!=0,(kind,run)
    assert marker.exists()==(kind=='bytes')
    assert b'budget' in (job/'capture.error').read_bytes()
    manifest=subprocess.run(base+['manifest',str(job),identity,prepare.stdout.decode()],capture_output=True)
    assert manifest.returncode!=0,(kind,manifest)
"#,
                RTK_CAPTURE,
                root.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn rtk_policy_handles_wrappers_and_rejects_authored_tee_overrides() {
        let request = |words: &[&str]| InvocationOptions {
            command: rch_common::invocation::CommandSpec::Argv(
                words.iter().map(|word| (*word).into()).collect(),
            ),
            cwd: "/shared/project".into(),
            env: Default::default(),
            timeout_seconds: None,
            interactive: false,
            workspace: WorkspaceMode::Shared,
            verification: true,
            json_result: None,
        };
        assert!(uses_rtk(&request(&["rtk", "pytest", "-q"])).unwrap());
        assert!(
            uses_rtk(&request(&[
                "env",
                "A=value",
                "nice",
                "-n",
                "5",
                "timeout",
                "--signal=TERM",
                "10s",
                "rtk",
                "pytest"
            ]))
            .unwrap()
        );
        assert!(
            uses_rtk(&request(&[
                "env",
                "RTK_TEE_DIR=/tmp/unowned",
                "rtk",
                "pytest",
                "-q"
            ]))
            .is_err()
        );
        assert!(uses_rtk(&request(&["env", "-u", "RTK_TEE_DIR", "rtk", "pytest"])).is_err());
        assert!(uses_rtk(&request(&["env", "-i", "rtk", "pytest"])).is_err());
        for words in [
            vec![
                "rtk",
                "run",
                "--skip-env",
                "-c",
                "export RTK_TEE_DIR=/tmp/foreign; go test",
            ],
            vec![
                "rtk",
                "-vvv",
                "proxy",
                "--",
                "env",
                "RTK_TEE_DIR=/tmp/foreign",
                "go",
                "test",
            ],
        ] {
            assert!(uses_rtk(&request(&words)).is_err());
        }
        assert!(!uses_rtk(&request(&["custom-wrapper", "rtk", "pytest"])).unwrap());
        assert!(!uses_rtk(&request(&["pytest", "-q"])).unwrap());
        assert!(!uses_rtk(&request(&["printf", "%s\\n", "rtk"])).unwrap());
        assert!(!uses_rtk(&request(&["printf", "%s\\n", "RTK_TEE_DIR=/tmp/data"])).unwrap());
        assert!(uses_rtk(&request(&["bash", "-c", "rtk pytest -q"])).unwrap());
        assert!(uses_rtk(&request(&["bash", "-c", "command -- rtk pytest -q"])).unwrap());
        assert!(uses_rtk(&request(&["bash", "-c", "exec -- rtk pytest -q"])).unwrap());
        assert!(!uses_rtk(&request(&["env", "-C", ".", "pytest", "-q"])).unwrap());
        assert!(uses_rtk(&request(&["env", "--chdir=.", "rtk", "pytest"])).unwrap());
        for source in [
            "export RTK_TEE_DIR=/tmp/unowned; rtk pytest -q",
            "rtk pytest -q; export RTK_TEE_DIR=/tmp/unowned",
            "unset RTK_TEE_DIR; command rtk pytest -q",
            "declare -x RTK_TEE_DIR=/tmp/unowned; exec rtk pytest -q",
            "builtin export RTK_TEE_DIR=/tmp/unowned; rtk pytest",
            "command export RTK_TEE_DIR=/tmp/unowned; rtk pytest",
        ] {
            assert!(
                uses_rtk(&request(&["bash", "-c", source])).is_err(),
                "{source}"
            );
        }
        assert!(
            !uses_rtk(&request(&[
                "bash",
                "-c",
                "printf '%s' 'export RTK_TEE_DIR=x; rtk'"
            ]))
            .unwrap()
        );
        assert!(
            uses_rtk(&request(&[
                "bash",
                "-lc",
                "env RTK_TEE_DIR=/tmp/unowned rtk pytest"
            ]))
            .is_err()
        );
        let mut shell = request(&["ignored"]);
        shell.command = rch_common::invocation::CommandSpec::Shell("env A=x rtk pytest".into());
        assert!(uses_rtk(&shell).unwrap());
        shell.command = rch_common::invocation::CommandSpec::Shell("printf '%s\\n' rtk".into());
        assert!(!uses_rtk(&shell).unwrap());
        shell.command =
            rch_common::invocation::CommandSpec::Shell("printf '%s' 'rtk; RTK_TEE_DIR=x'".into());
        assert!(!uses_rtk(&shell).unwrap());
        shell.command =
            rch_common::invocation::CommandSpec::Shell("printf '%s' rtk; rtk pytest".into());
        assert!(uses_rtk(&shell).unwrap());
    }

    #[test]
    fn shared_mapping_rejects_nested_and_same_device_bind_mounts() {
        let base = "24 1 0:40 / /share rw - nfs server:/export rw\n";
        assert!(verify_mount_topology(base, Path::new("/share")).is_ok());
        let bind = format!("{base}25 24 0:40 /other /share/project rw - nfs server:/export rw\n");
        assert!(verify_mount_topology(&bind, Path::new("/share")).is_err());
        let escaped =
            format!("{base}25 24 0:40 /other /share/project\\040one rw - nfs server:/export rw\n");
        assert!(verify_mount_topology(&escaped, Path::new("/share")).is_err());
        assert!(verify_mount_topology("", Path::new("/share")).is_err());
        assert!(verify_mount_topology("bad", Path::new("/share")).is_err());
    }

    #[test]
    fn private_control_rejects_permissive_and_symlink_directories() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let private = root.join("private");
        private_directory(&private).unwrap();
        assert_eq!(std::fs::metadata(&private).unwrap().mode() & 0o777, 0o700);
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(private_directory(&private).is_err());
        std::os::unix::fs::symlink(&private, root.join("alias")).unwrap();
        assert!(private_directory(&root.join("alias")).is_err());
        assert!(private_directory(&root.join("alias/new")).is_err());
        assert!(!private.join("new").exists());
    }

    #[test]
    fn shared_result_inspection_rejects_symlink_descent() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("results")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("results/loop")).unwrap();
        assert!(result_stat(&root, Path::new("results")).is_err());
        assert!(result_stat(&root, Path::new("results/loop")).is_err());
    }

    #[test]
    fn shared_control_never_lives_inside_source_inputs() {
        let mapping = SharedWorkspaceMapping {
            controller_root: "/shared".into(),
            worker_root: "/worker".into(),
            worker: "a".into(),
        };
        assert_eq!(
            control_root(&mapping, Path::new("/shared/project")).unwrap(),
            PathBuf::from("/shared/.rch-control")
        );
        assert!(control_root(&mapping, Path::new("/shared")).is_err());
    }

    #[test]
    fn shared_mapping_requires_authorized_worker_and_canonical_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let cwd = root.join("source");
        std::fs::create_dir(&cwd).unwrap();
        let mappings = vec![SharedWorkspaceMapping {
            controller_root: root,
            worker_root: "/worker".into(),
            worker: "builder".into(),
        }];
        let (_, canonical, remote) = shared_mapping(&mappings, &cwd, "builder").unwrap();
        assert_eq!(canonical, cwd);
        assert_eq!(remote, PathBuf::from("/worker/source"));
        assert!(shared_mapping(&mappings, &cwd, "other").is_err());
    }

    #[test]
    fn shared_results_count_existing_files_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("results")).unwrap();
        std::fs::write(root.join("results/output"), b"\0\xffbytes").unwrap();
        let stat = result_stat(&root, Path::new("results")).unwrap();
        assert_eq!((stat.files, stat.bytes), (1, 7));
        assert_eq!(
            std::fs::read(root.join("results/output")).unwrap(),
            b"\0\xffbytes"
        );
        assert!(result_stat(&root, Path::new("../escape")).is_err());
        assert!(result_stat(&root, Path::new("missing")).is_err());
    }
}
