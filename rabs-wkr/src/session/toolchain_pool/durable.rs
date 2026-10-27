//! Optional restart-persistent compiler inputs. This is not an action cache.
//!
//! A private, exclusively locked directory contains atomically published trees.
//! An exclusive directory reservation is charged before capture; an exclusive
//! empty completion marker publishes only the fully verified, synced tree. This
//! works on NFS without relying on unsupported RENAME_NOREPLACE. Names and markers
//! are lookup hints only: every open rehashes the complete selected dataset and
//! rejects writable files and hardlink aliases. Incomplete captures (including
//! legacy unmarked entries) remain charged but are never lookup candidates.
//! Nothing is automatically removed, repaired, or adopted after interruption.
//!
//! Set RABS_WORKER_TOOLCHAIN_CACHE_DIR to a dedicated absolute private directory
//! and RABS_WORKER_TOOLCHAIN_CACHE_BYTES to a nonzero retention budget. Without
//! CACHE_DIR the existing process-local policy is unchanged. The durable store
//! has at most eight entries and the configured logical-byte budget, including
//! incomplete reservations. A full store uses private execution copies; it never
//! evicts a published tree. Inspect pending entries before operator-directed
//! cleanup with the worker stopped. Filesystem I/O is checkpointed but kernel
//! calls are not claimed interruptible. No state or directories are deleted here.

use super::{MAX_POOL_BYTES, MAX_POOL_ENTRIES, WAIT_SLICE, checkpoint, invalid};
use rabs_sandbox::toolchain_dataset::{
    PreparedToolchain, ToolchainIdentity, ToolchainLimits, capture_toolchain, open_toolchain,
};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, TryLockError};

pub(super) const CONFIG_DIRECTORY: &str = "RABS_WORKER_TOOLCHAIN_CACHE_DIR";
const LOCK: &str = ".toolchain-cache-v1.lock";
const OBJECTS: &str = "toolchains-v1";
const PENDING: &str = ".pending-";
const COMPLETE: &str = ".complete";

struct CacheLock(File);

impl Drop for CacheLock {
    fn drop(&mut self) {
        // All execution leases retain the DurableCache through Arc. Once that
        // owner ends, a descriptor inherited by an unrelated fork must not
        // extend exclusivity until the child eventually execs or exits.
        let _ = self.0.unlock();
    }
}

pub(super) struct DurableCache {
    root: PathBuf,
    root_handle: File,
    objects_handle: File,
    // Remains locked while ANY execution lease owns this cache, not merely
    // while its supervisor is present in the process-global weak registry.
    _lock: CacheLock,
    max_bytes: u64,
    max_entries: usize,
    writer: Mutex<()>,
    // A visible marker cannot grant a lease until its publication sync succeeds.
    // An uncertain publication refuses subsequent loads until the store reopens.
    publication_confirmed: Mutex<bool>,
}

struct Catalogue {
    entries: usize,
    bytes: u64,
}

fn require(condition: bool, detail: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(detail))
    }
}

fn name(identity: &ToolchainIdentity) -> String {
    let hash: String = identity
        .sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("v1-{hash}-{}-{}", identity.files, identity.bytes)
}

fn identity_from_name(value: &str) -> io::Result<ToolchainIdentity> {
    let mut fields = value
        .strip_prefix("v1-")
        .ok_or_else(|| invalid("unknown persistent toolchain namespace"))?
        .split('-');
    let hash = fields.next().unwrap_or_default();
    require(
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid persistent toolchain digest",
    )?;
    let mut sha256 = [0; 32];
    for (index, byte) in sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hash[index * 2..index * 2 + 2], 16)
            .map_err(|_| invalid("invalid persistent toolchain digest"))?;
    }
    let number = |text: Option<&str>| -> io::Result<u64> {
        let text = text.ok_or_else(|| invalid("missing persistent toolchain size"))?;
        require(
            !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()),
            "invalid persistent toolchain size",
        )?;
        text.parse()
            .map_err(|_| invalid("persistent toolchain size overflow"))
    };
    let identity = ToolchainIdentity {
        sha256,
        files: number(fields.next())?,
        bytes: number(fields.next())?,
    };
    require(
        fields.next().is_none() && name(&identity) == value,
        "noncanonical persistent toolchain name",
    )?;
    let limits = ToolchainLimits::default();
    require(
        identity.bytes <= limits.max_bytes && identity.files <= limits.max_entries as u64,
        "persistent toolchain identity exceeds dataset bounds",
    )?;
    Ok(identity)
}

fn catalogue_identity(value: &str) -> io::Result<ToolchainIdentity> {
    if let Some(pending) = value.strip_prefix(PENDING) {
        let (identity, suffix) = pending
            .rsplit_once('-')
            .ok_or_else(|| invalid("invalid pending toolchain reservation"))?;
        require(
            !suffix.is_empty()
                && suffix.len() <= 32
                && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "invalid pending toolchain reservation suffix",
        )?;
        identity_from_name(identity)
    } else {
        identity_from_name(value)
    }
}

fn ordinary_path(path: &Path) -> io::Result<()> {
    require(
        path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
        "persistent toolchain cache must be a named absolute path without traversal",
    )?;
    let mut prefix = PathBuf::new();
    for part in path.components() {
        prefix.push(part.as_os_str());
        require(
            fs::symlink_metadata(&prefix)?.is_dir(),
            "persistent toolchain cache path contains a symlink or non-directory",
        )?;
    }
    Ok(())
}

#[cfg(unix)]
fn private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    require(
        meta.is_dir() && meta.mode() & 0o7777 == 0o700,
        "persistent toolchain cache directories must be private (0700)",
    )
}

#[cfg(not(unix))]
fn private_directory(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "persistent toolchain cache requires Unix permissions",
    ))
}

fn make_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    File::open(
        path.parent()
            .ok_or_else(|| invalid("cache directory has no parent"))?,
    )?
    .sync_all()
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.file_type() == right.file_type()
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        false
    }
}

impl DurableCache {
    pub(super) fn open(root: &Path, max_bytes: u64, max_entries: usize) -> io::Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "persistent toolchain datasets require Linux anchored verification",
            ));
        }
        require(
            max_bytes != 0
                && max_bytes <= MAX_POOL_BYTES
                && max_entries != 0
                && max_entries <= MAX_POOL_ENTRIES,
            "persistent toolchain cache requires a nonzero bounded retention budget",
        )?;
        require(
            root.is_absolute()
                && root.file_name().is_some()
                && root
                    .components()
                    .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
            "persistent toolchain cache must be a named absolute path without traversal",
        )?;
        match fs::symlink_metadata(root) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                ordinary_path(
                    root.parent()
                        .ok_or_else(|| invalid("cache parent missing"))?,
                )?;
                make_directory(root)?;
            }
            Err(error) => return Err(error),
        }
        ordinary_path(root)?;
        private_directory(root)?;
        // Refuse an unrelated existing directory before adding even our lock.
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            require(
                entry.file_name() == LOCK || entry.file_name() == OBJECTS,
                "persistent toolchain cache directory contains unrelated data",
            )?;
        }
        let root_handle = File::open(root)?;
        let lock_path = root.join(LOCK);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = match options.open(&lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let named = fs::symlink_metadata(&lock_path)?;
                require(
                    named.is_file() && named.len() == 0,
                    "invalid persistent cache lock",
                )?;
                OpenOptions::new().read(true).write(true).open(&lock_path)?
            }
            Err(error) => return Err(error),
        };
        let named = fs::symlink_metadata(&lock_path)?;
        let opened = lock.metadata()?;
        require(
            same_file(&named, &opened) && opened.is_file() && opened.len() == 0,
            "persistent cache lock changed while opening",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            require(
                opened.nlink() == 1
                    && opened.mode() & 0o7777 == 0o600
                    && opened.uid() == root_handle.metadata()?.uid(),
                "persistent cache lock must be private, owned and unaliased",
            )?;
        }
        lock.try_lock().map_err(|error| {
            io::Error::other(format!("persistent toolchain cache already owned: {error}"))
        })?;
        let lock = CacheLock(lock);
        require(
            same_file(&opened, &fs::symlink_metadata(&lock_path)?),
            "persistent cache lock replaced during acquisition",
        )?;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            require(
                entry.file_name() == LOCK || entry.file_name() == OBJECTS,
                "persistent toolchain cache directory contains unrelated data",
            )?;
        }
        let objects = root.join(OBJECTS);
        match fs::symlink_metadata(&objects) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => make_directory(&objects)?,
            Err(error) => return Err(error),
        }
        private_directory(&objects)?;
        let objects_handle = File::open(&objects)?;
        lock.0.sync_all()?;
        objects_handle.sync_all()?;
        root_handle.sync_all()?;
        let cache = Self {
            root: root.to_path_buf(),
            root_handle,
            objects_handle,
            _lock: lock,
            max_bytes,
            max_entries,
            writer: Mutex::new(()),
            publication_confirmed: Mutex::new(true),
        };
        cache.catalogue(&|| false)?;
        Ok(cache)
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    fn check_root(&self) -> io::Result<()> {
        ordinary_path(&self.root)?;
        private_directory(&self.root)?;
        let objects = self.root.join(OBJECTS);
        private_directory(&objects)?;
        require(
            same_file(
                &self.root_handle.metadata()?,
                &fs::symlink_metadata(&self.root)?,
            ) && same_file(
                &self.objects_handle.metadata()?,
                &fs::symlink_metadata(&objects)?,
            ),
            "persistent toolchain cache root was replaced",
        )
    }

    fn catalogue(&self, stopped: &impl Fn() -> bool) -> io::Result<Catalogue> {
        self.check_root()?;
        let mut result = Catalogue {
            entries: 0,
            bytes: 0,
        };
        for entry in fs::read_dir(self.root.join(OBJECTS))? {
            checkpoint(stopped)?;
            let entry = entry?;
            require(
                result.entries < self.max_entries,
                "persistent toolchain entry budget exceeded",
            )?;
            let name = entry.file_name();
            let identity = catalogue_identity(
                name.to_str()
                    .ok_or_else(|| invalid("non-UTF-8 persistent toolchain reservation"))?,
            )?;
            private_directory(&entry.path())?;
            result.entries += 1;
            result.bytes = result
                .bytes
                .checked_add(identity.bytes)
                .filter(|bytes| *bytes <= self.max_bytes)
                .ok_or_else(|| invalid("persistent toolchain byte budget exceeded"))?;
        }
        Ok(result)
    }

    pub(super) fn load(
        &self,
        expected: &ToolchainIdentity,
        stopped: &impl Fn() -> bool,
    ) -> io::Result<Option<PreparedToolchain>> {
        checkpoint(stopped)?;
        self.check_root()?;
        if expected.bytes > self.max_bytes {
            return Ok(None);
        }
        let path = self.root.join(OBJECTS).join(name(expected));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        private_directory(&path)?;
        let mut confirmed = self.publication(stopped)?;
        let marker_path = path.join(COMPLETE);
        let named = match fs::symlink_metadata(&marker_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        require(
            named.is_file() && named.len() == 0,
            "invalid persistent toolchain completion marker",
        )?;
        let marker = File::open(&marker_path)?;
        let opened = marker.metadata()?;
        require(
            same_file(&named, &opened) && opened.is_file() && opened.len() == 0,
            "persistent toolchain completion marker changed while opening",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            require(
                opened.nlink() == 1
                    && opened.mode() & 0o7777 == 0o600
                    && opened.uid() == self.root_handle.metadata()?.uid(),
                "persistent toolchain completion marker must be private, owned and unaliased",
            )?;
        }
        let mut children = 0;
        for child in fs::read_dir(&path)? {
            let child = child?;
            require(
                child.file_name() == "tree" || child.file_name() == COMPLETE,
                "persistent toolchain entry has unexpected files",
            )?;
            children += 1;
        }
        require(children == 2, "persistent toolchain tree missing")?;
        private_directory(&path.join("tree"))?;
        // A previous process may have died between marker creation and fsync.
        // Reconfirm its durability before granting any restart lease. The tree
        // was synced before the marker could exist, and is still reverified below.
        *confirmed = false;
        marker.sync_all()?;
        File::open(&path)?.sync_all()?;
        *confirmed = true;
        drop(confirmed);
        let limits = ToolchainLimits {
            max_bytes: expected.bytes.min(ToolchainLimits::default().max_bytes),
            ..ToolchainLimits::default()
        };
        let prepared = open_toolchain(&path.join("tree"), expected, &limits, stopped)?;
        // Source inventories may contain hardlinks and writable installation
        // files. A persisted captured tree may not: no writable alias is granted
        // merely because the content identity matches again after a restart.
        #[cfg(unix)]
        {
            use rabs_sandbox::toolchain_transfer::ToolchainEntryKind;
            use std::os::unix::fs::MetadataExt;
            let owner = self.root_handle.metadata()?.uid();
            for entry in prepared.entries()? {
                checkpoint(stopped)?;
                let path = prepared.root().join(&entry.path);
                let metadata = fs::symlink_metadata(&path)?;
                require(
                    metadata.uid() == owner,
                    "persistent toolchain entry has another owner",
                )?;
                match entry.kind {
                    ToolchainEntryKind::File { executable, .. } => require(
                        metadata.is_file()
                            && metadata.nlink() == 1
                            && metadata.mode() & 0o7777 == if executable { 0o555 } else { 0o444 },
                        "persistent toolchain file is writable, aliased or changed type",
                    )?,
                    ToolchainEntryKind::Directory => private_directory(&path)?,
                    ToolchainEntryKind::Symlink { .. } => require(
                        metadata.file_type().is_symlink(),
                        "persistent toolchain symlink changed type",
                    )?,
                }
            }
        }
        prepared.verify(stopped)?;
        checkpoint(stopped)?;
        Ok(Some(prepared))
    }

    /// Capture directly into the durable store, without a second toolchain copy.
    /// Capacity or writer contention uses the existing private-capture lane.
    /// Corruption/I/O/cancellation is an error, not permission to trust a path.
    pub(super) fn capture(
        &self,
        source: &Path,
        expected: &ToolchainIdentity,
        stopped: &impl Fn() -> bool,
    ) -> io::Result<Option<PreparedToolchain>> {
        checkpoint(stopped)?;
        if expected.bytes > self.max_bytes {
            return Ok(None);
        }
        let _writer = match self.writer.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => {
                return Err(invalid("persistent toolchain writer poisoned"));
            }
        };
        if let Some(prepared) = self.load(expected, stopped)? {
            return Ok(Some(prepared));
        }
        let used = self.catalogue(stopped)?;
        if used.entries >= self.max_entries || expected.bytes > self.max_bytes - used.bytes {
            return Ok(None);
        }
        let limits = ToolchainLimits {
            max_bytes: expected.bytes.min(ToolchainLimits::default().max_bytes),
            ..ToolchainLimits::default()
        };
        require(
            expected.files <= limits.max_entries as u64 && expected.bytes <= limits.max_bytes,
            "persistent toolchain capture exceeds dataset bounds",
        )?;
        let entry = self.root.join(OBJECTS).join(name(expected));
        // mkdir is an exclusive reservation, including on NFS. Never adopt or
        // replace an existing incomplete entry; its full identity stays charged.
        // Sync the parent before copying so the reservation survives interruption.
        match make_directory(&entry) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                private_directory(&entry)?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        let result = (|| -> io::Result<Option<PreparedToolchain>> {
            let prepared = capture_toolchain(
                source,
                &entry.join("tree"),
                Some(expected),
                &limits,
                stopped,
            )?;
            prepared.sync(stopped)?;
            File::open(&entry)?.sync_all()?;
            checkpoint(stopped)?;
            self.publish(&entry, stopped)?;
            checkpoint(stopped)?;
            let prepared = self
                .load(expected, stopped)?
                .ok_or_else(|| invalid("published persistent toolchain disappeared"))?;
            Ok(Some(prepared))
        })();
        result.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "persistent toolchain capture failed; inspect retained entry {}: {error}",
                    entry.display(),
                ),
            )
        })
    }

    fn publication(&self, stopped: &impl Fn() -> bool) -> io::Result<MutexGuard<'_, bool>> {
        loop {
            // Check immediately before every acquisition attempt, including the
            // one that succeeds after waiting. Never invoke caller callbacks
            // while holding the publication guard (they may inspect the cache).
            checkpoint(stopped)?;
            match self.publication_confirmed.try_lock() {
                Ok(confirmed) => {
                    require(
                        *confirmed,
                        "persistent toolchain publication durability unconfirmed",
                    )?;
                    return Ok(confirmed);
                }
                Err(TryLockError::WouldBlock) => std::thread::sleep(WAIT_SLICE),
                Err(TryLockError::Poisoned(_)) => {
                    return Err(invalid("persistent toolchain publication poisoned"));
                }
            }
        }
    }

    fn publish(&self, entry: &Path, stopped: &impl Fn() -> bool) -> io::Result<()> {
        self.publish_with_sync(entry, stopped, |marker, directory| {
            marker.sync_all()?;
            directory.sync_all()
        })
    }

    // The injected operation exercises the real publication/read exclusion and
    // uncertain-I/O path in tests; production always syncs both real descriptors.
    fn publish_with_sync(
        &self,
        entry: &Path,
        stopped: &impl Fn() -> bool,
        sync: impl FnOnce(&File, &File) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut confirmed = self.publication(stopped)?;
        let directory = File::open(entry)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        *confirmed = false;
        let marker = match options.open(entry.join(COMPLETE)) {
            Ok(marker) => marker,
            Err(error) => {
                // A collision did not publish anything. Other NFS I/O errors
                // may have created the marker despite an uncertain response.
                if error.kind() == io::ErrorKind::AlreadyExists {
                    *confirmed = true;
                }
                return Err(error);
            }
        };
        sync(&marker, &directory)?;
        *confirmed = true;
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
