//! Exclusive lifetime ownership of a local Unix-socket pathname.
//!
//! A refused connect is necessary, but not sufficient, to retire a stale
//! socket: the endpoint lock and directory/socket generations must still
//! agree. Timeout, permissions, malformed state and replacement are not proof
//! of death. The lock inode is permanent, not PID-file state to delete.
//!
//! Networking and deadlines belong to the caller. Retain this lease until the
//! listener AND all admitted sessions/writers have ended. These advisory locks
//! fence participating daemons, not a hostile process with the same UID.

use rustix::fs::{AtFlags, Mode, OFlags, chmodat, open, openat, unlinkat};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

fn refused(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, detail)
}

fn inode(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

/// An observed socket generation; callers cannot manufacture one from a PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketStamp {
    inode: (u64, u64),
    ctime: (i64, i64),
    uid: u32,
    gid: u32,
    mode: u32,
}

impl SocketStamp {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            inode: inode(metadata),
            ctime: (metadata.ctime(), metadata.ctime_nsec()),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
        }
    }
}

/// A fork can retain a duplicate open description until exec despite CLOEXEC.
/// Explicit unlock makes release a property of this owner, not the last child.
#[derive(Debug)]
struct Locked(File);

impl Drop for Locked {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Non-clone endpoint lease, shared only by an outer lifetime owner. Dropping
/// it releases the lock but does NOT blindly remove a socket.
#[derive(Debug)]
pub struct EndpointLease {
    requested_parent: PathBuf,
    parent: PathBuf,
    directory: File,
    name: OsString,
    path: PathBuf,
    lock_name: OsString,
    uid: u32,
    bound: Option<SocketStamp>,
    // Release after the other owned descriptors and state.
    lock: Locked,
}

impl EndpointLease {
    /// Acquire without waiting, before probing, binding, chmod or retirement.
    /// New parents are private; existing parents must be same-user and not
    /// group/world writable. Parent aliases converge on one lock inode.
    ///
    /// # Errors
    /// Invalid paths, unsafe ownership/types, contention or filesystem errors.
    pub fn acquire(path: &Path) -> io::Result<Self> {
        if !path.is_absolute()
            || path.file_name().is_none()
            || path
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(refused("endpoint must be absolute without traversal"));
        }
        let requested_parent = path
            .parent()
            .ok_or_else(|| refused("endpoint parent"))?
            .to_owned();
        let name = path
            .file_name()
            .ok_or_else(|| refused("endpoint name"))?
            .to_owned();
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&requested_parent)?;
        let parent = fs::canonicalize(&requested_parent)?;
        let directory = File::from(open(
            &parent,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let uid = rustix::process::geteuid().as_raw();
        validate_directory(&directory.metadata()?, uid)?;
        // NOT cargo-rabs's startup lock: that descriptor is deliberately
        // inherited by the daemon. A new open description would contend.
        let mut lock_name = OsString::from(".rabs-");
        lock_name.push(&name);
        lock_name.push(".endpoint.lock");
        let file = File::from(openat(
            &directory,
            lock_name.as_os_str(),
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?);
        validate_lock(&file.metadata()?, uid)?;
        file.try_lock()
            .map_err(|error| io::Error::other(format!("endpoint lock unavailable: {error}")))?;
        // Install the explicit-unlock guard immediately, including on every
        // validation error below.
        let lock = Locked(file);
        let lease = Self {
            path: parent.join(&name),
            requested_parent,
            parent,
            directory,
            name,
            lock_name,
            uid,
            bound: None,
            lock,
        };
        lease.verify()?;
        Ok(lease)
    }

    /// Canonical parent spelling of the leased directory entry.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Kernel effective UID used for every ownership check.
    #[must_use]
    pub const fn owner_uid(&self) -> u32 {
        self.uid
    }

    /// Revalidate namespace and lock identity before each endpoint mutation.
    /// Directory timestamps are not identities: unrelated sibling entries can
    /// legitimately change while a daemon is alive.
    ///
    /// # Errors
    /// Replaced/retargeted parents, unsafe metadata or lost lock identity.
    pub fn verify(&self) -> io::Result<()> {
        let held = self.directory.metadata()?;
        validate_directory(&held, self.uid)?;
        for named in [
            fs::symlink_metadata(&self.parent)?,
            fs::metadata(&self.requested_parent)?,
        ] {
            validate_directory(&named, self.uid)?;
            if inode(&named) != inode(&held) {
                return Err(refused("endpoint parent was replaced or retargeted"));
            }
        }
        let held = self.lock.0.metadata()?;
        let named = fs::symlink_metadata(self.parent.join(&self.lock_name))?;
        validate_lock(&held, self.uid)?;
        validate_lock(&named, self.uid)?;
        if inode(&held) != inode(&named) {
            return Err(refused(
                "endpoint lock pathname no longer names its held inode",
            ));
        }
        Ok(())
    }

    fn socket(&self, require_private: bool) -> io::Result<Option<SocketStamp>> {
        self.verify()?;
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_socket()
            || metadata.uid() != self.uid
            || metadata.nlink() != 1
            || (require_private && metadata.mode() & 0o777 != 0o600)
        {
            return Err(refused(
                "endpoint must be a same-user, single-link private socket",
            ));
        }
        self.verify()?;
        Ok(Some(SocketStamp::of(&metadata)))
    }

    /// Observe without following a final symlink. Unsafe entries are refusals,
    /// never something startup may repair.
    ///
    /// # Errors
    /// Filesystem errors, unsafe metadata or lost endpoint ownership.
    pub fn observe(&self) -> io::Result<Option<SocketStamp>> {
        self.socket(true)
    }

    /// Retire exactly the socket that returned ConnectionRefused. Pass the
    /// actual connection error, not a timeout or failed status read.
    ///
    /// # Errors
    /// Wrong probe result, changed generation, lost ownership or unlink error.
    pub fn retire_refused(&self, observed: SocketStamp, error: &io::Error) -> io::Result<()> {
        if self.bound.is_some() || error.kind() != io::ErrorKind::ConnectionRefused {
            return Err(refused(
                "retirement requires a refused connect before binding",
            ));
        }
        if self.observe()? != Some(observed) {
            return Err(refused("endpoint changed during its liveness probe"));
        }
        // Use the held directory, never a potentially retargeted parent alias.
        // Participating creators hold this lock across their entire lifetime.
        unlinkat(&self.directory, self.name.as_os_str(), AtFlags::empty())?;
        Ok(())
    }

    /// Bind a new listener and retain its exact post-claim generation.
    /// On Linux, restrict the socket descriptor BEFORE publishing its pathname:
    /// a crash between bind and claim must not leave a broadly accessible inode
    /// that the next daemon correctly refuses to take over. This changes no
    /// process-wide umask and never repairs an existing endpoint's permissions.
    /// Other Unix platforms retain their native bind followed by claim.
    ///
    /// # Errors
    /// Lost ownership, an existing endpoint, bind/listen errors, or claim failure.
    pub fn bind_listener(&mut self) -> io::Result<std::os::unix::net::UnixListener> {
        self.verify()?;
        if self.bound.is_some() {
            return Err(refused("endpoint already bound by this lease"));
        }
        let listener = bind_new_listener(&self.path)?;
        self.claim_bound()?;
        Ok(listener)
    }

    /// Call once after successfully binding a NEW listener. Record its final
    /// post-chmod generation for identity-checked teardown.
    ///
    /// # Errors
    /// Repeated claim, disappearance, replacement, unsafe type or chmod error.
    pub fn claim_bound(&mut self) -> io::Result<()> {
        if self.bound.is_some() {
            return Err(refused("endpoint already bound by this lease"));
        }
        let before = self
            .socket(false)?
            .ok_or_else(|| refused("new endpoint disappeared"))?;
        // The held private parent, not its absolute pathname. Even if an
        // ancestor is renamed concurrently, chmod cannot follow that new
        // ancestor to an unrelated file. Same-UID hostile mutation is outside
        // this advisory lease's boundary.
        chmodat(
            &self.directory,
            self.name.as_os_str(),
            Mode::from_raw_mode(0o600),
            AtFlags::empty(),
        )?;
        let after = self
            .observe()?
            .ok_or_else(|| refused("new endpoint disappeared after chmod"))?;
        if before.inode != after.inode || before.uid != after.uid || before.gid != after.gid {
            return Err(refused(
                "endpoint changed while claiming the bound listener",
            ));
        }
        self.bound = Some(after);
        Ok(())
    }

    /// Retire our exact generation only after listener and sessions end.
    /// False means absent/replaced, not permission to repair the replacement.
    ///
    /// # Errors
    /// Unsafe replacement, lost ownership or filesystem errors; nothing else
    /// is removed on their account.
    pub fn retire_bound(&self) -> io::Result<bool> {
        let Some(bound) = self.bound else {
            return Ok(false);
        };
        if self.observe()? != Some(bound) {
            return Ok(false);
        }
        unlinkat(&self.directory, self.name.as_os_str(), AtFlags::empty())?;
        Ok(true)
    }
}

#[cfg(target_os = "linux")]
fn bind_private_socket(path: &Path) -> io::Result<rustix::fd::OwnedFd> {
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, bind, socket_with};

    let socket = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )?;
    // Linux's unix_bind_bsd derives the pathname mode from this descriptor's
    // inode AND the caller's umask. Normal masks may remove permissions but
    // can never add group/world access. claim_bound still enforces exact 0600.
    rustix::fs::fchmod(&socket, Mode::from_raw_mode(0o600))?;
    bind(&socket, &SocketAddrUnix::new(path)?)?;
    Ok(socket)
}

#[cfg(target_os = "linux")]
fn bind_new_listener(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    let socket = bind_private_socket(path)?;
    rustix::net::listen(&socket, 128)?;
    Ok(socket.into())
}

#[cfg(not(target_os = "linux"))]
fn bind_new_listener(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    std::os::unix::net::UnixListener::bind(path)
}

fn validate_directory(metadata: &fs::Metadata, uid: u32) -> io::Result<()> {
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return Err(refused(
            "endpoint parent must be same-user and not group/world writable",
        ));
    }
    Ok(())
}

fn validate_lock(metadata: &fs::Metadata, uid: u32) -> io::Result<()> {
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(refused(
            "endpoint lock must be a same-user private single-link regular file",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "socket_endpoint_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "linux"))]
mod private_bind_tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    struct OwnedChild(Child);

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }

    #[test]
    fn interrupted_bind_child() {
        let Some(path) = std::env::var_os("RABS_TEST_PRIVATE_BIND_PATH") else {
            return;
        };
        let path = PathBuf::from(path);
        let mask = u32::from_str_radix(&std::env::var("RABS_TEST_BIND_MASK").unwrap(), 8).unwrap();
        // Only this dedicated post-exec helper changes its process umask.
        rustix::fs::umask(Mode::from_raw_mode(mask));
        let lease = EndpointLease::acquire(&path).unwrap();
        let socket = bind_private_socket(lease.path()).unwrap();
        if std::env::var("RABS_TEST_BIND_LISTEN").as_deref() == Ok("1") {
            rustix::net::listen(&socket, 128).unwrap();
        }
        let ready = path.with_extension("ready");
        let pending = path.with_extension("pending");
        fs::write(&pending, b"bound before claim\n").unwrap();
        fs::rename(&pending, &ready).unwrap();
        // The parent kills and reaps this process at the real syscall boundary.
        // No destructor, chmod, endpoint claim or graceful retirement runs.
        let mut byte = [0];
        let _ = std::io::stdin().read_exact(&mut byte);
        panic!("interrupted-bind child must be killed before it can continue");
    }

    #[test]
    fn killing_before_claim_leaves_a_private_recoverable_endpoint() {
        for mask in [0o000, 0o022, 0o077] {
            for listen in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let path = root.path().join("edge.sock");
                let ready = path.with_extension("ready");
                let mut child = OwnedChild(
                    Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "socket_endpoint::private_bind_tests::interrupted_bind_child",
                            "--nocapture",
                        ])
                        .env("RABS_TEST_PRIVATE_BIND_PATH", &path)
                        .env("RABS_TEST_BIND_MASK", format!("{mask:o}"))
                        .env("RABS_TEST_BIND_LISTEN", if listen { "1" } else { "0" })
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::inherit())
                        .spawn()
                        .unwrap(),
                );
                let deadline = Instant::now() + Duration::from_secs(5);
                while !ready.exists() && Instant::now() < deadline {
                    assert!(child.0.try_wait().unwrap().is_none(), "bind helper exited");
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert_eq!(fs::read(&ready).unwrap(), b"bound before claim\n");
                let before = SocketStamp::of(&fs::symlink_metadata(&path).unwrap());
                assert_eq!(before.mode & 0o777, 0o600, "mask={mask:o}, listen={listen}");
                assert!(EndpointLease::acquire(&path).is_err());
                child.0.kill().unwrap();
                assert!(!child.0.wait().unwrap().success());
                let mut lease = EndpointLease::acquire(&path).unwrap();
                assert_eq!(lease.observe().unwrap(), Some(before));
                let error = UnixStream::connect(&path).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
                lease.retire_refused(before, &error).unwrap();
                let listener = lease.bind_listener().unwrap();
                assert!(UnixStream::connect(&path).is_ok());
                assert_eq!(lease.observe().unwrap().unwrap().mode & 0o777, 0o600);
                drop(listener);
                assert!(lease.retire_bound().unwrap());
            }
        }
    }
}
