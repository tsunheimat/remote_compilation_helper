//! Explicit local execution with a dedicated Linux child-subreaper process.
//! The controller's process settings, cwd and environment are never changed.
//! Cleanup requires the isolated supervisor's kernel ECHILD observation, not
//! an empty process group, an inherited lock, or a successful leader exit.
//! This is a cooperative cache guard, not a filesystem/security sandbox: jobs
//! can write outside their lease, and an unresponsive supervisor retains scratch.

use anyhow::{Context, Result, bail};
use rch_common::invocation::{CommandSpec, InvocationOptions};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MONITOR_INTERVAL: Duration = Duration::from_millis(250);
const TERMINATION_GRACE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct LocalGuardConfig {
    pub root: PathBuf,
    pub max_bytes: u64,
    pub min_free_bytes: u64,
    /// Nonblocking admission for active guard invocations, not escaped processes.
    /// No implicit unlimited fallback when all slots fill.
    pub max_concurrent: Option<usize>,
    /// Retain scratch even after the dedicated subreaper proves quiescence.
    pub keep: bool,
    /// Tee raw bytes to owned on-disk logs. Retains the lease for their consumer.
    pub capture_logs: bool,
    /// Preserve RTK's raw tee output under this job's owned lease.
    pub managed_rtk_tee: bool,
}

impl LocalGuardConfig {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            max_bytes: 2 * 1024 * 1024 * 1024,
            min_free_bytes: 4 * 1024 * 1024 * 1024,
            max_concurrent: None,
            keep: false,
            capture_logs: false,
            managed_rtk_tee: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionCause {
    Exited,
    Signaled,
    Timeout,
    JobHighWater,
    FreeSpaceFloor,
    Interrupted,
    ControlError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessControl {
    DedicatedChildSubreaper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseRetentionReason {
    Reclaimed,
    Requested,
    CapturedLogs,
    DescendantQuiescenceUnproven,
    CleanupFailed,
}

#[derive(Debug, Serialize)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub cause: ExecutionCause,
    pub local: bool,
    /// Streams are inherited (PTY mode merges output), never interpreted as UTF-8.
    /// No log file is invented when output was not captured.
    pub logs: Vec<PathBuf>,
    pub lease_path: PathBuf,
    pub lease_retained: bool,
    pub lease_retention_reason: LeaseRetentionReason,
    pub process_control: ProcessControl,
    /// False means unproven, not that an escaped descendant was observed.
    pub descendants_quiesced: bool,
    pub duration_ms: u64,
    pub detail: Option<String>,
}

/// A sibling temporary file plus fsync/rename keeps readers from seeing partial
/// JSON. The result path is independent of workload stdout/stderr. Existing
/// result files are replaced only at this explicitly requested destination.
pub fn write_json_result(path: &Path, result: &impl Serialize) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .context("create atomic result file beside destination")?;
    serde_json::to_writer(&mut temporary, result).context("serialize execution result")?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| e.error)
        .context("publish atomic execution result")?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub async fn run_guarded_local(
    invocation: &InvocationOptions,
    config: &LocalGuardConfig,
) -> Result<ExecutionResult> {
    invocation.validate().map_err(anyhow::Error::msg)?;
    #[cfg(target_os = "linux")]
    {
        linux::run(invocation, config).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        bail!("guarded local execution requires Linux process ownership and disk controls")
    }
}

/// Called before runtime construction. The hidden reexec process alone becomes
/// a subreaper; ordinary CLI invocations do not change process-global settings.
pub fn subreaper_entrypoint() -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        let mut args = std::env::args_os().skip(1);
        if args.next().as_deref() == Some(std::ffi::OsStr::new("--rch-local-subreaper-v1")) {
            let outcome = args
                .next()
                .and_then(|address| address.into_string().ok())
                .filter(|_| args.next().is_none())
                .context("invalid local supervisor arguments")
                .and_then(|address| linux::supervise(&address));
            return Some(match outcome {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("rch local supervisor: {error:#}");
                    125
                }
            });
        }
    }
    None
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, WaitOptions};
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const SETUP_BUDGET: Duration = Duration::from_secs(5);
    const REAP_BUDGET: Duration = Duration::from_secs(3);
    const POLL_INTERVAL: Duration = Duration::from_millis(10);
    const MAX_REQUEST: usize = 1024 * 1024;
    const MAX_RECEIPT: usize = 16 * 1024;
    const MAX_CHILDREN: usize = 65_536;
    // Linux __WALL includes clone children with a non-SIGCHLD exit signal.
    // Rustix exposes externally-defined wait flags through from_bits_retain.
    const WALL: u32 = 0x4000_0000;

    #[derive(Serialize, Deserialize)]
    struct SupervisorRequest {
        invocation: InvocationOptions,
        lease: PathBuf,
        max_bytes: u64,
        min_free_bytes: u64,
        capture_logs: bool,
        managed_rtk_tee: bool,
        workload_pgid: i32,
    }

    #[derive(Serialize, Deserialize)]
    struct Receipt {
        version: u32,
        exit_code: i32,
        signal: Option<i32>,
        cause: ExecutionCause,
        descendants_quiesced: bool,
        detail: Option<String>,
    }

    const CACHE_PATHS: &[(&str, &str)] = &[
        ("HOME", "home"),
        ("TMPDIR", "tmp"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
        ("GOTMPDIR", "tmp/go"),
        ("GOCACHE", "go-build"),
        ("GOMODCACHE", "go-mod"),
        ("XDG_CACHE_HOME", "xdg"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("NPM_CONFIG_CACHE", "npm"),
        ("npm_config_cache", "npm"),
        ("BUN_INSTALL_CACHE_DIR", "bun"),
        ("UV_CACHE_DIR", "uv"),
        ("PIP_CACHE_DIR", "pip"),
        ("CARGO_HOME", "cargo"),
        ("CARGO_TARGET_DIR", "cargo-target"),
    ];

    struct OwnedSupervisor {
        child: Option<Child>,
        disconnect: Option<UnixStream>,
    }

    impl Drop for OwnedSupervisor {
        fn drop(&mut self) {
            // Closing the controller connection is a cancellation request. No
            // supervisor signal can accidentally orphan its owned descendants.
            if let Some(stream) = &self.disconnect {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            if let Some(mut child) = self.child.take() {
                let _ = std::thread::Builder::new()
                    .name("rch-local-reap".into())
                    .spawn(move || {
                        let deadline = Instant::now() + REAP_BUDGET + SETUP_BUDGET;
                        loop {
                            match child.try_wait() {
                                Ok(Some(_)) | Err(_) => return,
                                Ok(None) => {}
                            }
                            if Instant::now() >= deadline {
                                // A stuck supervisor is not cleanup proof. Its
                                // job was persisted before spawn and is retained.
                                let _ = child.kill();
                                let _ = child.try_wait();
                                return;
                            }
                            std::thread::sleep(POLL_INTERVAL);
                        }
                    });
            }
        }
    }

    fn write_frame(stream: &mut UnixStream, value: &impl Serialize, max: usize) -> Result<()> {
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > max {
            bail!("local supervisor message exceeds control budget");
        }
        stream.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
        stream.write_all(&bytes)?;
        Ok(())
    }

    fn read_request(stream: &mut UnixStream) -> Result<SupervisorRequest> {
        let mut length = [0; 4];
        stream.read_exact(&mut length)?;
        let length = usize::try_from(u32::from_be_bytes(length))?;
        if length > MAX_REQUEST {
            bail!("local supervisor request exceeds control budget");
        }
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Only the dedicated, single spawning/reaping thread calls these wait
    /// functions. Log threads neither spawn nor reap. A child which exits stays
    /// a zombie until this thread reaps it, pinning its PID against reuse.
    fn signal_children(signal: Signal) -> Result<()> {
        let mut bytes = Vec::new();
        // Adopted orphans attach to the thread-group leader; a test harness can
        // invoke the very same supervisor on its test thread. Include all tasks,
        // not only the original spawning task. No other thread here spawns jobs.
        for (index, task) in std::fs::read_dir("/proc/self/task")?.enumerate() {
            if index >= 256 {
                bail!("local supervisor task enumeration exceeded budget");
            }
            let path = task?.path().join("children");
            match File::open(path) {
                Ok(file) => {
                    let remaining = (MAX_CHILDREN * 12 + 1).saturating_sub(bytes.len());
                    if remaining == 0 {
                        bail!("local supervisor child enumeration exceeded budget");
                    }
                    file.take(u64::try_from(remaining)?)
                        .read_to_end(&mut bytes)?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            }
            if bytes.len() > MAX_CHILDREN * 12 {
                bail!("local supervisor child enumeration exceeded budget");
            }
            bytes.push(b' ');
        }
        for (index, token) in std::str::from_utf8(&bytes)?.split_whitespace().enumerate() {
            if index >= MAX_CHILDREN {
                bail!("local supervisor child count exceeded budget");
            }
            let pid = Pid::from_raw(token.parse()?)
                .filter(|pid| pid.as_raw_nonzero().get() > 1)
                .context("invalid child PID in procfs")?;
            // Check actual wait ownership rather than trusting a PID snapshot.
            // No reaping occurs between this check and kill, so even an exit
            // leaves the same child identity pinned. Never signal a guessed PGID.
            match rustix::process::waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED
                    | WaitIdOptions::NOHANG
                    | WaitIdOptions::NOWAIT
                    | WaitIdOptions::from_bits_retain(WALL),
            ) {
                Ok(_) => match rustix::process::kill_process(pid, signal) {
                    Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                    Err(error) => return Err(error.into()),
                },
                Err(rustix::io::Errno::CHILD) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn reap_available(
        leader: Pid,
        status: &mut Option<rustix::process::WaitStatus>,
    ) -> Result<bool> {
        for _ in 0..MAX_CHILDREN {
            match rustix::process::wait(WaitOptions::NOHANG | WaitOptions::from_bits_retain(WALL)) {
                Ok(Some((pid, child_status))) => {
                    if pid == leader && status.is_none() {
                        // Preserve the original outcome even if extreme PID
                        // churn later reuses its reaped numeric identity.
                        *status = Some(child_status);
                    }
                }
                Ok(None) => return Ok(false),
                // This is the authorization boundary. Orphans are reparented
                // by the kernel before their previous parent becomes waitable.
                // No children means no still-live descendant can later fork.
                Err(rustix::io::Errno::CHILD) => return Ok(true),
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(false)
    }

    fn stop_descendants(
        leader: Pid,
        status: &mut Option<rustix::process::WaitStatus>,
    ) -> (bool, Option<String>) {
        let start = Instant::now();
        let mut detail = None;
        loop {
            match reap_available(leader, status) {
                Ok(true) => return (true, detail),
                Ok(false) => {}
                Err(error) => detail = Some(format!("reap descendants: {error:#}")),
            }
            if start.elapsed() >= REAP_BUDGET {
                return (
                    false,
                    Some(detail.unwrap_or_else(|| {
                        "descendant reaping exceeded three seconds; lease retained".into()
                    })),
                );
            }
            let signal = if start.elapsed() < TERMINATION_GRACE {
                Signal::TERM
            } else {
                Signal::KILL
            };
            if let Err(error) = signal_children(signal) {
                detail = Some(format!("signal owned descendants: {error:#}"));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    struct TerminalRestore(Option<rustix::termios::Termios>);
    impl TerminalRestore {
        fn enter(interactive: bool) -> Result<Self> {
            if !interactive || !std::io::IsTerminal::is_terminal(&io::stdin()) {
                return Ok(Self(None));
            }
            let old =
                rustix::termios::tcgetattr(io::stdin()).context("read input terminal settings")?;
            let mut raw = old.clone();
            raw.make_raw();
            rustix::termios::tcsetattr(io::stdin(), rustix::termios::OptionalActions::Now, &raw)
                .context("configure interactive input terminal")?;
            Ok(Self(Some(old)))
        }
    }
    impl Drop for TerminalRestore {
        fn drop(&mut self) {
            if let Some(old) = &self.0 {
                let _ = rustix::termios::tcsetattr(
                    io::stdin(),
                    rustix::termios::OptionalActions::Now,
                    old,
                );
            }
        }
    }

    fn private_directory(path: &Path) -> Result<()> {
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(path)?;
                if !metadata.is_dir()
                    || metadata.uid() != rustix::process::getuid().as_raw()
                    || metadata.permissions().mode() & 0o022 != 0
                {
                    bail!(
                        "guard directory must be owned, non-symlink and not writable by others: {}",
                        path.display()
                    );
                }
                Ok(())
            }
            Err(error) => {
                Err(error).with_context(|| format!("create guard directory {}", path.display()))
            }
        }
    }

    fn prepare_root(config: &LocalGuardConfig) -> Result<PathBuf> {
        if !config.root.is_absolute()
            || config.root == Path::new("/")
            || config
                .root
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
            || config.max_bytes == 0
            || config.min_free_bytes == 0
            || config.max_concurrent == Some(0)
        {
            bail!("invalid guard root, byte limits, or concurrency limit");
        }
        // Parent directories are deployment configuration, not an implicit /tmp fallback.
        private_directory(&config.root)?;
        let root = std::fs::canonicalize(&config.root)?;
        private_directory(&root.join("jobs"))?;
        Ok(root)
    }

    fn acquire_slot(root: &Path, limit: Option<usize>) -> Result<Option<File>> {
        use rustix::fs::{FlockOperation, flock};
        let Some(limit) = limit else { return Ok(None) };
        if limit > 4096 {
            bail!("guard concurrency limit exceeds 4096");
        }
        private_directory(&root.join("slots"))?;
        for index in 0..limit {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                .open(root.join("slots").join(index.to_string()))?;
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.uid() != rustix::process::getuid().as_raw()
                || metadata.nlink() != 1
                || metadata.mode() & 0o022 != 0
            {
                bail!("invalid guard admission lock");
            }
            match flock(&file, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => return Ok(Some(file)),
                Err(rustix::io::Errno::WOULDBLOCK) => {}
                Err(error) => return Err(error).context("lock guarded execution slot"),
            }
        }
        bail!("guarded local concurrency limit reached; command was not executed")
    }

    /// Never descend through symlinks. Both apparent length (including sparse
    /// files) and allocated blocks count, so neither kind evades the limit.
    fn job_size(root: &File, limit: u64) -> io::Result<u64> {
        use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, openat, statat};
        let mut pending = vec![root.try_clone()?];
        let mut total = 0u64;
        let mut entries = 0usize;
        let start = Instant::now();
        while let Some(directory) = pending.pop() {
            for entry in Dir::read_from(&directory)? {
                let entry = entry?;
                let name = entry.file_name();
                if matches!(name.to_bytes(), b"." | b"..") {
                    continue;
                }
                let metadata = match statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(metadata) => metadata,
                    Err(rustix::io::Errno::NOENT) => continue,
                    Err(error) => return Err(error.into()),
                };
                entries += 1;
                if entries > 1_000_000 || start.elapsed() > MONITOR_INTERVAL {
                    return Err(io::Error::other(
                        "guard lease scan exceeded its monitoring budget",
                    ));
                }
                let size = u64::try_from(metadata.st_size).unwrap_or(u64::MAX);
                let allocated = u64::try_from(metadata.st_blocks)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(512);
                total = total.saturating_add(size.max(allocated));
                if total > limit {
                    return Ok(total);
                }
                if FileType::from_raw_mode(metadata.st_mode) == FileType::Directory {
                    // Open relative to the pinned parent. O_NOFOLLOW defeats a
                    // directory-to-symlink replacement between stat and open.
                    match openat(
                        &directory,
                        name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    ) {
                        Ok(fd) => pending.push(File::from(fd)),
                        Err(rustix::io::Errno::NOENT) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        Ok(total)
    }

    fn free_bytes(root: &File) -> Result<u64> {
        let stat = rustix::fs::fstatvfs(root).context("read guard filesystem free space")?;
        Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
    }

    fn preserve_rustup_home(command: &mut Command, invocation: &InvocationOptions) {
        if invocation.env.contains_key("RUSTUP_HOME") || std::env::var_os("RUSTUP_HOME").is_some() {
            return;
        }
        if let Some(home) = std::env::var_os("HOME") {
            let toolchains = PathBuf::from(home).join(".rustup");
            if toolchains.is_dir() {
                command.env("RUSTUP_HOME", toolchains);
            }
        }
    }

    fn tee_bytes(
        mut input: impl io::Read,
        mut output: impl io::Write,
        mut log: File,
    ) -> io::Result<()> {
        use io::Write;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = match input.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                break;
            }
            log.write_all(&buffer[..count])?;
            output.write_all(&buffer[..count])?;
            output.flush()?;
        }
        log.sync_all()
    }

    fn start_tee(
        input: impl io::Read + Send + 'static,
        output: impl io::Write + Send + 'static,
        log: File,
        errors: std::sync::mpsc::Sender<String>,
    ) -> io::Result<std::thread::JoinHandle<io::Result<()>>> {
        std::thread::Builder::new()
            .name("rch-local-log".into())
            .spawn(move || {
                let result = tee_bytes(input, output, log);
                if let Err(error) = &result {
                    let _ = errors.send(error.to_string());
                }
                result
            })
    }

    fn command_argv(invocation: &InvocationOptions) -> Vec<String> {
        match &invocation.command {
            CommandSpec::Argv(args) => args.clone(),
            CommandSpec::Shell(source) => vec![
                "/bin/bash".into(),
                "--noprofile".into(),
                "--norc".into(),
                "-c".into(),
                source.clone(),
            ],
        }
    }

    fn validate_environment(invocation: &InvocationOptions) -> Result<()> {
        for key in invocation.env.keys() {
            if CACHE_PATHS.iter().any(|(owned, _)| key == owned)
                || matches!(
                    key.as_str(),
                    "BASH_ENV" | "ENV" | "RCH_GUARDED_LOCAL" | "RCH_LOCAL_LEASE"
                )
            {
                bail!("guard owns environment variable {key}; command was not executed");
            }
        }
        Ok(())
    }

    pub(super) fn supervise(address: &str) -> Result<()> {
        // No runtime or process-global SIGCHLD handler exists in this reexec.
        let address = SocketAddr::from_abstract_name(address.as_bytes())?;
        let mut stream = UnixStream::connect_addr(&address)?;
        stream.set_read_timeout(Some(SETUP_BUDGET))?;
        stream.set_write_timeout(Some(TERMINATION_GRACE))?;
        let request = read_request(&mut stream)?;
        request.invocation.validate().map_err(anyhow::Error::msg)?;
        validate_environment(&request.invocation)?;
        if !request.lease.is_absolute() || std::fs::canonicalize(&request.lease)? != request.lease {
            bail!("noncanonical supervisor lease");
        }
        private_directory(&request.lease)?;
        rustix::process::set_child_subreaper(Some(rustix::process::getpid()))
            .context("enable dedicated child subreaper")?;
        if rustix::process::child_subreaper()?.is_none() {
            bail!("kernel did not enable child subreaper");
        }
        stream.set_nonblocking(true)?;
        let mut receipt = supervise_workload(&request, &mut stream)?;
        // The receipt is a single length-delimited transaction, never inferred
        // from stdout or workload-writable files. Partial frames do not count.
        if let Some(detail) = &mut receipt.detail {
            detail.truncate(detail.floor_char_boundary(4096));
        }
        stream.set_nonblocking(false)?;
        write_frame(&mut stream, &receipt, MAX_RECEIPT)
    }

    fn supervise_workload(request: &SupervisorRequest, stream: &mut UnixStream) -> Result<Receipt> {
        let invocation = &request.invocation;
        let lease_path = &request.lease;
        let lease_fd = File::open(lease_path)?;
        let mut command = if invocation.interactive {
            // A trusted stdlib broker, not a shell parser or a second supervisor.
            let mut command = Command::new("/usr/bin/python3");
            command.args(["-I", "-c", PTY_BROKER]);
            command.arg(lease_path.join("pty-ready"));
            command.args(command_argv(invocation));
            command
        } else {
            let argv = command_argv(invocation);
            let mut command = Command::new(&argv[0]);
            command.args(&argv[1..]);
            // Inherit the controller's foreground group for native tty stdin.
            // The isolated supervisor itself remains in a different group, so
            // terminal-generated signals cannot kill the reaper. Ownership and
            // cleanup never depend on this workload group remaining intact.
            command.process_group(request.workload_pgid);
            command
        };
        command
            .current_dir(&invocation.cwd)
            .envs(&invocation.env)
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .env_remove("RCH_TEST_SUBREAPER_ADDRESS")
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        preserve_rustup_home(&mut command, invocation);
        let mut log_files = Vec::new();
        if request.capture_logs {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
            for name in ["stdout.log", "stderr.log"] {
                log_files.push(
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(lease_path.join(name))?,
                );
            }
        }
        for (key, subdir) in CACHE_PATHS {
            let path = lease_path.join(subdir);
            std::fs::create_dir_all(&path)?;
            command.env(key, path);
        }
        if request.managed_rtk_tee {
            let tee = lease_path.join("rtk-tee");
            private_directory(&tee)?;
            command.env("RTK_TEE_DIR", tee);
        }
        command
            .env("RCH_GUARDED_LOCAL", "1")
            .env("RCH_LOCAL_LEASE", lease_path);
        let start = Instant::now();
        let mut child = command
            .spawn()
            .context("spawn guarded local command (no fallback)")?;
        let leader = Pid::from_raw(i32::try_from(child.id())?).context("invalid workload PID")?;
        // From this point, every ordinary error passes through stop_descendants.
        // std::process::Child is never waited separately from our kernel reaper.
        let (log_errors_tx, log_errors_rx) = std::sync::mpsc::channel();
        let mut log_threads = Vec::new();
        let setup = (|| -> Result<()> {
            if request.capture_logs {
                let stderr = log_files.pop().context("stderr log missing")?;
                let stdout = log_files.pop().context("stdout log missing")?;
                log_threads.push(start_tee(
                    child.stdout.take().context("stdout pipe missing")?,
                    io::stdout(),
                    stdout,
                    log_errors_tx.clone(),
                )?);
                log_threads.push(start_tee(
                    child.stderr.take().context("stderr pipe missing")?,
                    io::stderr(),
                    stderr,
                    log_errors_tx,
                )?);
            }
            Ok(())
        })();
        let mut status = None;
        let mut interrupted_signal = None;
        let mut pty_started = !invocation.interactive;
        let mut detail = None;
        let mut next_scan = start;
        let monitored = setup.and_then(|()| -> Result<ExecutionCause> {
            loop {
                // Reaping all owned children also notices detached descendants
                // which exit before their leader. Only the leader sets outcome.
                let _ = reap_available(leader, &mut status)?;
                if status.is_some() {
                    return Ok(ExecutionCause::Exited);
                }
                let mut controls = [0u8; 64];
                match stream.read(&mut controls) {
                    Ok(0) => {
                        detail = Some("controller disconnected; stopping owned descendants".into());
                        return Ok(ExecutionCause::ControlError);
                    }
                    Ok(count) => {
                        for control in &controls[..count] {
                            match control {
                                1 | 2 | 15 => {
                                    interrupted_signal = Some(i32::from(*control));
                                    return Ok(ExecutionCause::Interrupted);
                                }
                                28 if invocation.interactive && pty_started => {
                                    signal_children(Signal::WINCH)?
                                }
                                28 => {}
                                124 => return Ok(ExecutionCause::Timeout),
                                _ => bail!("invalid supervisor control byte"),
                            }
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) => return Err(error.into()),
                }
                if invocation
                    .timeout_seconds
                    .is_some_and(|seconds| start.elapsed() >= Duration::from_secs(seconds))
                {
                    return Ok(ExecutionCause::Timeout);
                }
                if let Ok(error) = log_errors_rx.try_recv() {
                    bail!("binary log relay failed: {error}");
                }
                if !pty_started {
                    if lease_path.join("pty-ready").is_file() {
                        std::fs::write(lease_path.join("pty-go"), b"go\n")?;
                        pty_started = true;
                    } else if start.elapsed() >= SETUP_BUDGET {
                        bail!("PTY setup did not complete within five seconds");
                    }
                }
                if Instant::now() >= next_scan {
                    if job_size(&lease_fd, request.max_bytes)? > request.max_bytes {
                        return Ok(ExecutionCause::JobHighWater);
                    }
                    if free_bytes(&lease_fd)? < request.min_free_bytes {
                        return Ok(ExecutionCause::FreeSpaceFloor);
                    }
                    next_scan = Instant::now() + MONITOR_INTERVAL;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        });
        let mut cause = match monitored {
            Ok(cause) => cause,
            Err(error) => {
                detail = Some(format!("{error:#}"));
                ExecutionCause::ControlError
            }
        };
        let (descendants_quiesced, stop_detail) = stop_descendants(leader, &mut status);
        if let Some(stop_detail) = stop_detail {
            detail = Some(stop_detail);
        }
        if !descendants_quiesced
            && matches!(cause, ExecutionCause::Exited | ExecutionCause::Signaled)
        {
            cause = ExecutionCause::ControlError;
        }
        let log_deadline = Instant::now() + TERMINATION_GRACE;
        for thread in log_threads {
            while !thread.is_finished() && Instant::now() < log_deadline {
                std::thread::sleep(POLL_INTERVAL);
            }
            if !thread.is_finished() {
                cause = ExecutionCause::ControlError;
                detail = Some("binary log relay did not drain within one second".into());
            } else {
                match thread.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        cause = ExecutionCause::ControlError;
                        detail = Some(error.to_string());
                    }
                    Err(_) => {
                        cause = ExecutionCause::ControlError;
                        detail = Some("binary log relay panicked".into());
                    }
                }
            }
        }
        let signal = status.and_then(|status| status.terminating_signal());
        if cause == ExecutionCause::Exited && signal.is_some() {
            cause = ExecutionCause::Signaled;
        }
        if invocation.interactive && !pty_started {
            cause = ExecutionCause::ControlError;
            detail = Some("PTY broker failed before controlling-terminal setup".into());
        }
        let exit_code = match cause {
            ExecutionCause::Timeout => 124,
            ExecutionCause::JobHighWater
            | ExecutionCause::FreeSpaceFloor
            | ExecutionCause::ControlError => 125,
            ExecutionCause::Interrupted => 128 + interrupted_signal.unwrap_or(2),
            _ => status
                .and_then(|status| status.exit_status())
                .unwrap_or_else(|| 128 + signal.unwrap_or(1)),
        };
        Ok(Receipt {
            version: 1,
            exit_code,
            signal,
            cause,
            descendants_quiesced,
            detail,
        })
    }

    async fn read_receipt(reader: &mut tokio::net::unix::OwnedReadHalf) -> Result<Receipt> {
        let length = usize::try_from(reader.read_u32().await?)?;
        if length > MAX_RECEIPT {
            bail!("local supervisor receipt exceeds control budget");
        }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).await?;
        let receipt: Receipt = serde_json::from_slice(&bytes)?;
        if receipt.version != 1 {
            bail!("unknown local supervisor receipt version");
        }
        Ok(receipt)
    }

    pub(super) async fn run(
        invocation: &InvocationOptions,
        config: &LocalGuardConfig,
    ) -> Result<ExecutionResult> {
        validate_environment(invocation)?;
        let root = prepare_root(config)?;
        let _slot = acquire_slot(&root, config.max_concurrent)?;
        let root_fd = File::open(&root)?;
        if free_bytes(&root_fd)? < config.min_free_bytes {
            bail!("guard free-space floor not met; command was not executed");
        }
        let lease = tempfile::Builder::new()
            .prefix("job-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(root.join("jobs"))?;
        let lease_path = lease.path().to_path_buf();
        let lease_metadata = std::fs::symlink_metadata(&lease_path)?;
        let request = SupervisorRequest {
            invocation: invocation.clone(),
            lease: lease_path.clone(),
            max_bytes: config.max_bytes,
            min_free_bytes: config.min_free_bytes,
            capture_logs: config.capture_logs,
            managed_rtk_tee: config.managed_rtk_tee,
            workload_pgid: rustix::process::getpgrp().as_raw_nonzero().get(),
        };
        let request_bytes = serde_json::to_vec(&request)?;
        if request_bytes.len() > MAX_REQUEST {
            bail!("local supervisor request exceeds control budget; command was not executed");
        }
        let address = format!("rch-local-{}", uuid::Uuid::new_v4());
        let listener =
            UnixListener::bind_addr(&SocketAddr::from_abstract_name(address.as_bytes())?)?;
        listener.set_nonblocking(true)?;
        let listener = tokio::net::UnixListener::from_std(listener)?;
        let mut command = Command::new(std::env::current_exe()?);
        #[cfg(not(test))]
        command.args(["--rch-local-subreaper-v1", &address]);
        #[cfg(test)]
        {
            // Reexec the identical Rust supervisor inside one exact unit-test
            // helper. Harness banners are discarded, not mixed into log files.
            command
                .args([
                    "--exact",
                    "guarded_exec::tests::subreaper_test_entry",
                    "--nocapture",
                ])
                .env("RCH_TEST_SUBREAPER_ADDRESS", &address)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        command.process_group(0).stdin(Stdio::inherit());
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
        let mut resize =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
        let _terminal = TerminalRestore::enter(invocation.interactive)?;
        // Keep before spawn: cancellation/partial receipt can never drop live scratch.
        let _retained_lease = lease.keep();
        let start = Instant::now();
        let child = command
            .spawn()
            .context("spawn dedicated local supervisor (no fallback)")?;
        let pid = i32::try_from(child.id())?;
        let mut supervisor = OwnedSupervisor {
            child: Some(child),
            disconnect: None,
        };
        let connection = tokio::time::timeout(SETUP_BUDGET, async {
            loop {
                let (stream, _) = listener.accept().await?;
                // SO_PEERCRED identifies the process which connected. Descendants
                // cannot forge a receipt by reconnecting to the public address.
                if stream.peer_cred()?.pid() == Some(pid) {
                    return Ok::<_, io::Error>(stream);
                }
            }
        })
        .await
        .context("local supervisor connection timed out")??;
        let connection = connection.into_std()?;
        supervisor.disconnect = Some(connection.try_clone()?);
        let mut connection = tokio::net::UnixStream::from_std(connection)?;
        tokio::time::timeout(SETUP_BUDGET, async {
            connection
                .write_u32(u32::try_from(request_bytes.len())?)
                .await?;
            connection.write_all(&request_bytes).await?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("local supervisor request timed out")??;
        let (mut reader, mut writer) = connection.into_split();
        let receipt_future = read_receipt(&mut reader);
        tokio::pin!(receipt_future);
        let mut tick = tokio::time::interval(MONITOR_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut stop_deadline = None;
        let receipt = loop {
            let control = tokio::select! {
                receipt = &mut receipt_future => break receipt,
                _ = interrupt.recv() => 2,
                _ = terminate.recv() => 15,
                _ = hangup.recv() => 1,
                _ = resize.recv(), if invocation.interactive => 28,
                _ = tick.tick() => {
                    if stop_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                        break Err(anyhow::anyhow!("local supervisor did not finish bounded cleanup; lease retained"));
                    }
                    if stop_deadline.is_none() && invocation.timeout_seconds.is_some_and(|seconds| start.elapsed() >= Duration::from_secs(seconds).saturating_add(SETUP_BUDGET)) {
                        124
                    } else {
                        continue;
                    }
                }
            };
            if stop_deadline.is_some() {
                continue; // Never flood a stopping supervisor with repeated signals.
            }
            if control != 28 {
                stop_deadline =
                    Some(Instant::now() + REAP_BUDGET + TERMINATION_GRACE + TERMINATION_GRACE);
            }
            match tokio::time::timeout(TERMINATION_GRACE, writer.write_all(&[control])).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => break Err(error.into()),
                Err(error) => break Err(error.into()),
            }
        };
        let mut receipt = receipt.unwrap_or_else(|error| Receipt {
            version: 1,
            exit_code: 125,
            signal: None,
            cause: ExecutionCause::ControlError,
            descendants_quiesced: false,
            detail: Some(format!("{error:#}")),
        });
        // A complete success receipt plus a successful supervisor exit are both
        // required. The supervisor is still trusted to hold no live workload.
        let exit_deadline = Instant::now() + TERMINATION_GRACE;
        let supervisor_ok = loop {
            match supervisor
                .child
                .as_mut()
                .context("missing supervisor child")?
                .try_wait()
            {
                Ok(Some(status)) => {
                    supervisor.child.take();
                    break status.success();
                }
                Ok(None) if Instant::now() < exit_deadline => {
                    tokio::time::sleep(POLL_INTERVAL).await
                }
                _ => break false,
            }
        };
        if !supervisor_ok {
            receipt.descendants_quiesced = false;
            receipt.cause = ExecutionCause::ControlError;
            receipt.exit_code = 125;
            receipt.detail =
                Some("local supervisor did not exit successfully; lease retained".into());
        }
        let mut retained = true;
        let mut retention_reason = if !receipt.descendants_quiesced {
            LeaseRetentionReason::DescendantQuiescenceUnproven
        } else if config.keep {
            LeaseRetentionReason::Requested
        } else if config.capture_logs || config.managed_rtk_tee {
            LeaseRetentionReason::CapturedLogs
        } else {
            LeaseRetentionReason::CleanupFailed
        };
        if receipt.descendants_quiesced
            && !config.keep
            && !config.capture_logs
            && !config.managed_rtk_tee
        {
            let cleanup = (|| -> Result<()> {
                let metadata = std::fs::symlink_metadata(&lease_path)?;
                if !metadata.is_dir()
                    || metadata.dev() != lease_metadata.dev()
                    || metadata.ino() != lease_metadata.ino()
                {
                    bail!("guard lease identity changed; refusing cleanup");
                }
                // std's fd-relative Linux implementation does not follow links.
                // Only this unique job directory is removed, never root/caches.
                std::fs::remove_dir_all(&lease_path).context("remove quiesced guard lease")
            })();
            match cleanup {
                Ok(()) => {
                    retained = false;
                    retention_reason = LeaseRetentionReason::Reclaimed;
                }
                Err(error) => {
                    receipt.exit_code = 125;
                    receipt.cause = ExecutionCause::ControlError;
                    receipt.detail = Some(format!("{error:#}"));
                }
            }
        }
        let mut logs = if config.capture_logs {
            vec![lease_path.join("stdout.log"), lease_path.join("stderr.log")]
        } else {
            vec![]
        };
        if config.managed_rtk_tee {
            logs.push(lease_path.join("rtk-tee"));
        }
        let result = ExecutionResult {
            exit_code: receipt.exit_code,
            signal: receipt.signal,
            cause: receipt.cause,
            local: true,
            logs,
            lease_path,
            lease_retained: retained,
            lease_retention_reason: retention_reason,
            process_control: ProcessControl::DedicatedChildSubreaper,
            descendants_quiesced: receipt.descendants_quiesced,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            detail: receipt.detail,
        };
        if let Some(path) = &invocation.json_result {
            write_json_result(path, &result)
                .context("command completed, but result publication failed; do not replay")?;
        }
        Ok(result)
    }

    // The broker becomes a session/group leader, then acquires a controlling
    // terminal. Its workload starts in that same owned process group; a command
    // may subsequently leave it. This broker does not provide subtree containment.
    // Never pty.fork(): that would put the workload in a second, unowned session.
    const PTY_BROKER: &str = r#"
import errno, fcntl, os, select, signal, subprocess, sys, termios, time
os.setsid()
master, slave = os.openpty()
fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
try:
    size = fcntl.ioctl(0, termios.TIOCGWINSZ, b'\0' * 8)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, size)
except OSError:
    pass
with open(sys.argv[1], 'xb') as ready:
    ready.write(b'ready\n')
deadline = time.monotonic() + 5
while not os.path.exists(os.path.join(os.path.dirname(sys.argv[1]), 'pty-go')):
    if time.monotonic() >= deadline:
        raise RuntimeError('PTY ownership handshake timed out')
    time.sleep(0.01)
child = subprocess.Popen(sys.argv[2:], stdin=slave, stdout=slave, stderr=slave, close_fds=True)
os.close(slave)
signal.signal(signal.SIGINT, signal.SIG_IGN)
signal.signal(signal.SIGTERM, signal.SIG_IGN)
signal.signal(signal.SIGHUP, signal.SIG_IGN)
def resize(signum, frame):
    try:
        fcntl.ioctl(master, termios.TIOCSWINSZ, fcntl.ioctl(0, termios.TIOCGWINSZ, b'\0' * 8))
    except OSError:
        pass
signal.signal(signal.SIGWINCH, resize)
def write_all(fd, data):
    while data:
        data = data[os.write(fd, data):]
# Only the privately opened master changes flags, never inherited stdio's
# shared file descriptions. Queue stdin so a full PTY input buffer cannot stop
# the broker draining output needed by a child before it reads that input.
os.set_blocking(master, False)
pending_input = bytearray()
stdin_open = True
master_open = True
while master_open:
    inputs = [master]
    if stdin_open and len(pending_input) < 65536:
        inputs.append(0)
    readable, writable, _ = select.select(inputs, [master] if pending_input else [], [], 0.1)
    for fd in readable:
        try:
            data = os.read(fd, 65536 if fd == master else 65536 - len(pending_input))
        except BlockingIOError:
            continue
        except OSError as error:
            if fd == master and error.errno == errno.EIO:
                data = b''
            else:
                raise
        if not data:
            if fd == master:
                master_open = False
                break
            stdin_open = False
            # Canonical VEOF first flushes a partial line; a second VEOF makes
            # the following read return zero. A PTY has no write-half shutdown:
            # in raw mode do not invent input bytes or close unread output.
            attrs = termios.tcgetattr(master)
            if attrs[3] & termios.ICANON:
                eof = attrs[6][termios.VEOF]
                if isinstance(eof, int):
                    eof = bytes([eof])
                pending_input.extend(eof * 2)
        elif fd == master:
            write_all(1, data)
        else:
            pending_input.extend(data)
    if master_open and master in writable:
        try:
            count = os.write(master, pending_input)
            del pending_input[:count]
        except BlockingIOError:
            pass
        except OSError as error:
            if error.errno != errno.EIO:
                raise
            master_open = False
    if child.poll() is not None and not readable:
        break
status = child.wait()
os.close(master)
if status < 0:
    signal.signal(-status, signal.SIG_DFL)
    os.kill(os.getpid(), -status)
sys.exit(status)
"#;
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use rch_common::invocation::WorkspaceMode;
    use std::collections::BTreeMap;

    #[test]
    fn subreaper_test_entry() {
        let Ok(address) = std::env::var("RCH_TEST_SUBREAPER_ADDRESS") else {
            return;
        };
        let result = linux::supervise(&address);
        if let Err(error) = &result {
            eprintln!("test supervisor: {error:#}");
        }
        std::process::exit(if result.is_ok() { 0 } else { 125 });
    }

    fn invocation(command: &[&str], cwd: &Path) -> InvocationOptions {
        InvocationOptions {
            command: CommandSpec::Argv(command.iter().map(|s| (*s).into()).collect()),
            cwd: cwd.into(),
            env: BTreeMap::new(),
            timeout_seconds: Some(5),
            interactive: false,
            workspace: WorkspaceMode::Shared,
            verification: false,
            json_result: None,
        }
    }

    fn fixture() -> tempfile::TempDir {
        let mut root = tempfile::tempdir().unwrap();
        root.disable_cleanup(true);
        root
    }

    fn config(root: &Path) -> LocalGuardConfig {
        let mut config = LocalGuardConfig::new(root.join("guard"));
        config.min_free_bytes = 1;
        config
    }

    #[tokio::test]
    async fn exact_argv_cwd_environment_and_atomic_result() {
        let root = fixture();
        let mut request = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                "import os,sys; assert sys.argv[1]=='a b; $x'; assert os.environ['TEST_VALUE']=='literal'; assert os.environ['HOME'].startswith(os.environ['RCH_LOCAL_LEASE']); assert os.getcwd()==sys.argv[2]; sys.exit(17)",
                "a b; $x",
                root.path().to_str().unwrap(),
            ],
            root.path(),
        );
        request.env.insert("TEST_VALUE".into(), "literal".into());
        request.json_result = Some(root.path().join("result.json"));
        let result = run_guarded_local(&request, &config(root.path()))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 17);
        assert_eq!(result.cause, ExecutionCause::Exited);
        assert!(!result.lease_path.exists());
        assert!(!result.lease_retained);
        assert!(result.descendants_quiesced);
        assert_eq!(
            result.lease_retention_reason,
            LeaseRetentionReason::Reclaimed
        );
        let stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(request.json_result.unwrap()).unwrap()).unwrap();
        assert_eq!(stored["exit_code"], 17);
        assert_eq!(stored["local"], true);
    }

    #[tokio::test]
    async fn setsid_close_fds_descendant_is_adopted_reaped_and_scratch_reclaimed() {
        // Retain the fixture itself for failure inspection. Only the guard's
        // explicitly owned job scratch is eligible for automatic reclamation.
        let root = tempfile::tempdir().unwrap().keep();
        let marker = root.join("detached-pid");
        let request = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                r#"
import pathlib, subprocess, sys, time
code = '''import os, pathlib, signal, sys, time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
pathlib.Path(sys.argv[1]).write_text(str(os.getpid()))
end = time.monotonic() + 12
while time.monotonic() < end:
    with open(os.environ['TMPDIR'] + '/heartbeat', 'ab') as f: f.write(b'x')
    time.sleep(.02)
'''
subprocess.Popen([sys.executable, '-I', '-c', code, sys.argv[1]], close_fds=True, start_new_session=True)
end = time.monotonic() + 3
while not pathlib.Path(sys.argv[1]).exists():
    if time.monotonic() > end: raise RuntimeError('child not ready')
    time.sleep(.01)
"#,
                marker.to_str().unwrap(),
            ],
            &root,
        );
        let result = run_guarded_local(&request, &config(&root)).await.unwrap();
        let pid = std::fs::read_to_string(marker).unwrap();
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "detached child must be reaped"
        );
        assert!(!result.lease_path.exists());
        assert!(!result.lease_retained);
        assert!(result.descendants_quiesced);
        assert_eq!(
            result.process_control,
            ProcessControl::DedicatedChildSubreaper
        );
        assert_eq!(
            result.lease_retention_reason,
            LeaseRetentionReason::Reclaimed
        );
    }

    #[tokio::test]
    async fn managed_rtk_tee_preserves_raw_bytes_in_private_owned_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = fixture();
        let request = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                "import os; open(os.environ['RTK_TEE_DIR']+'/raw.log','wb').write(b'raw\\x00\\xff'); print('filtered')",
            ],
            root.path(),
        );
        let mut guard = config(root.path());
        assert!(!guard.managed_rtk_tee);
        guard.managed_rtk_tee = true;
        let result = run_guarded_local(&request, &guard).await.unwrap();
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert!(result.descendants_quiesced);
        assert!(result.lease_retained);
        assert_eq!(
            result.lease_retention_reason,
            LeaseRetentionReason::CapturedLogs
        );
        assert_eq!(result.logs, vec![result.lease_path.join("rtk-tee")]);
        assert_eq!(
            std::fs::metadata(&result.logs[0])
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::read(result.logs[0].join("raw.log")).unwrap(),
            b"raw\x00\xff"
        );
    }

    #[tokio::test]
    async fn oversized_control_request_refuses_before_execution() {
        let root = fixture();
        let mut request = invocation(&["/bin/false"], root.path());
        request
            .env
            .insert("LARGE_CONTROL_VALUE".into(), "x".repeat(1024 * 1024));
        let error = run_guarded_local(&request, &config(root.path()))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("request exceeds control budget"));
    }

    #[tokio::test]
    async fn missing_executable_is_control_failure_and_never_cleanup_proof() {
        let root = fixture();
        let request = invocation(&["/not-an-rch-executable"], root.path());
        let result = run_guarded_local(&request, &config(root.path()))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 125);
        assert_eq!(result.cause, ExecutionCause::ControlError);
        assert!(!result.descendants_quiesced);
        assert!(result.lease_retained);
        assert!(result.lease_path.is_dir());
    }

    #[tokio::test]
    async fn optional_logs_preserve_binary_streams() {
        let root = fixture();
        let request = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                "import os; os.write(1,b'out\\x00\\xff'); os.write(2,b'err\\x00\\xfe')",
            ],
            root.path(),
        );
        let mut guard = config(root.path());
        guard.capture_logs = true;
        let result = run_guarded_local(&request, &guard).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.lease_retained);
        assert_eq!(std::fs::read(&result.logs[0]).unwrap(), b"out\x00\xff");
        assert_eq!(std::fs::read(&result.logs[1]).unwrap(), b"err\x00\xfe");
    }

    #[tokio::test]
    async fn high_water_stops_writing_and_keeps_only_owned_lease() {
        let root = fixture();
        let unrelated = root.path().join("shared-cache");
        std::fs::write(&unrelated, b"untouched").unwrap();
        let mut config = config(root.path());
        config.max_bytes = 128 * 1024;
        config.keep = true;
        let request = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                "import os,time; open(os.environ['TMPDIR']+'/large','wb').write(b'x'*262144); time.sleep(10)",
            ],
            root.path(),
        );
        let result = run_guarded_local(&request, &config).await.unwrap();
        assert_eq!(result.cause, ExecutionCause::JobHighWater);
        assert_eq!(result.exit_code, 125);
        assert!(result.lease_path.is_dir());
        assert_eq!(std::fs::read(unrelated).unwrap(), b"untouched");
    }

    #[tokio::test]
    async fn timeout_kills_term_ignoring_group() {
        let root = fixture();
        let mut request = invocation(
            &["/bin/sh", "-c", "trap '' TERM; sleep 30 & wait"],
            root.path(),
        );
        request.timeout_seconds = Some(1);
        let start = std::time::Instant::now();
        let result = run_guarded_local(&request, &config(root.path()))
            .await
            .unwrap();
        assert_eq!(result.cause, ExecutionCause::Timeout, "{result:?}");
        assert_eq!(result.exit_code, 124, "{result:?}");
        assert!(result.descendants_quiesced, "{result:?}");
        assert!(!result.lease_retained);
        assert!(!result.lease_path.exists());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn exited_leader_does_not_leave_term_ignoring_descendant() {
        let root = fixture();
        let heartbeat = root.path().join("heartbeat");
        let request = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                "import os,signal,sys,time; p=os.fork();\nif p==0:\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n while True:\n  open(sys.argv[1],'ab').write(b'x'); time.sleep(.03)\nelse:\n time.sleep(.1)",
                heartbeat.to_str().unwrap(),
            ],
            root.path(),
        );
        let result = run_guarded_local(&request, &config(root.path()))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        let size = std::fs::metadata(&heartbeat).unwrap().len();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(std::fs::metadata(&heartbeat).unwrap().len(), size);
    }

    #[tokio::test]
    async fn canceled_controller_stops_children_but_retains_unacknowledged_scratch() {
        let root = fixture();
        let marker = root.path().join("cancel-child");
        let request = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                "import os,pathlib,signal,sys,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); pathlib.Path(sys.argv[1]).write_text(str(os.getpid())+'\\n'+os.environ['RCH_LOCAL_LEASE']); time.sleep(12)",
                marker.to_str().unwrap(),
            ],
            root.path(),
        );
        let guard = config(root.path());
        let task = tokio::spawn(async move { run_guarded_local(&request, &guard).await });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let data = std::fs::read_to_string(&marker).unwrap();
        let (pid, lease) = data.split_once('\n').unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let process = PathBuf::from(format!("/proc/{pid}"));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while process.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !process.exists(),
            "controller disconnect must still reap its child"
        );
        assert!(
            Path::new(lease).is_dir(),
            "no consumed receipt means no deletion authority"
        );
    }

    #[tokio::test]
    async fn concurrent_admission_refuses_without_running_second_command() {
        let root = fixture();
        let marker = root.path().join("started");
        let mut guard = config(root.path());
        guard.max_concurrent = Some(1);
        let first = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                "import pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text('started'); time.sleep(1)",
                marker.to_str().unwrap(),
            ],
            root.path(),
        );
        let second = invocation(&["/bin/false"], root.path());
        let (first_result, second_result) =
            tokio::join!(run_guarded_local(&first, &guard), async {
                for _ in 0..100 {
                    if marker.exists() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert!(marker.exists());
                run_guarded_local(&second, &guard).await
            });
        assert_eq!(first_result.unwrap().exit_code, 0);
        assert!(
            second_result
                .unwrap_err()
                .to_string()
                .contains("concurrency limit")
        );
    }

    #[tokio::test]
    async fn controls_refuse_instead_of_unguarded_spawn() {
        let root = fixture();
        let mut request = invocation(&["/bin/false"], root.path());
        request.env.insert("TMPDIR".into(), "/tmp".into());
        assert!(
            run_guarded_local(&request, &config(root.path()))
                .await
                .is_err()
        );
        request.env.clear();
        let mut guard = config(root.path());
        guard.min_free_bytes = u64::MAX;
        assert!(run_guarded_local(&request, &guard).await.is_err());
    }

    #[tokio::test]
    async fn interactive_has_real_controlling_terminal() {
        let root = fixture();
        let mut request = invocation(
            &[
                "/usr/bin/python3",
                "-I",
                "-c",
                "import os; assert all(os.isatty(fd) for fd in (0,1,2)); f=os.open('/dev/tty',os.O_RDWR); assert os.tcgetpgrp(f)==os.getpgrp(); os.close(f)",
            ],
            root.path(),
        );
        request.interactive = true;
        let result = run_guarded_local(&request, &config(root.path()))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0, "{result:?}");
    }
}
