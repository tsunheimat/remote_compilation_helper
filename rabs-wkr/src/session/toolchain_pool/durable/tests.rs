//! Real filesystem, dataset verifier and pool tests. Reopening models restart;
//! these fixtures are not power-loss, TLS, compiler-execution or speedup proof.
use super::super::ToolchainPool;
use super::*;
use rabs_sandbox::canonical_namespace::{Bind, CanonicalNamespaceSpec};
use rabs_sandbox::toolchain_dataset::fingerprint_toolchain;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

const BYTES: &[u8] = b"compiler\0\xff";

struct Fixture {
    _owner: PathBuf,
    source: PathBuf,
    cache: PathBuf,
    identity: ToolchainIdentity,
}

impl Fixture {
    fn new() -> Self {
        // Retain the real filesystem evidence on success and failure; persistent
        // cache tests must not erase interrupted captures during unwinding.
        let owner = tempfile::Builder::new()
            .prefix("rabs-durable-toolchain-test-")
            .tempdir()
            .unwrap()
            .keep();
        let root = owner.canonicalize().unwrap();
        eprintln!("retained durable toolchain fixture: {}", root.display());
        let source = root.join("installation");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::create_dir_all(source.join("lib/empty")).unwrap();
        fs::write(source.join("bin/compiler"), BYTES).unwrap();
        fs::set_permissions(
            source.join("bin/compiler"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("compiler", source.join("bin/rustc")).unwrap();
        let identity =
            fingerprint_toolchain(&source, &ToolchainLimits::default(), || false).unwrap();
        Self {
            _owner: owner,
            source,
            cache: root.join("cache"),
            identity,
        }
    }

    fn pool(&self) -> ToolchainPool {
        ToolchainPool::with_directory(1024, 8, Some(&self.cache)).unwrap()
    }

    fn entry(&self) -> PathBuf {
        self.cache.join(OBJECTS).join(name(&self.identity))
    }

    fn objects(&self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = fs::read_dir(self.cache.join(OBJECTS))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        paths
    }
}

#[test]
fn durable_publication_does_not_require_noreplace_directory_renames() {
    let f = Fixture::new();
    // Measure the old production primitive on the SAME filesystem. Its failure
    // is not waived in production and its success is not labeled an NFS failure:
    // this records the host capability while exercising the new real cache path.
    let legacy_source = f._owner.join("legacy-directory-source");
    let legacy_target = f._owner.join("legacy-directory-target");
    fs::create_dir(&legacy_source).unwrap();
    fs::write(legacy_source.join("sentinel"), BYTES).unwrap();
    let legacy = rabs_cas::materialization::publish_new_directory(&legacy_source, &legacy_target);
    eprintln!("same-filesystem legacy directory publication: {legacy:?}");

    let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
    let prepared = cache
        .capture(&f.source, &f.identity, &|| false)
        .unwrap()
        .unwrap();
    prepared.verify(|| false).unwrap();
    assert_eq!(
        fs::read(prepared.root().join("bin/compiler")).unwrap(),
        BYTES
    );
    let marker = fs::symlink_metadata(f.entry().join(COMPLETE)).unwrap();
    assert!(marker.is_file());
    assert_eq!(marker.len(), 0);
    assert_eq!(marker.mode() & 0o7777, 0o600);
    drop(prepared);
    drop(cache);
    let reopened = DurableCache::open(&f.cache, 1024, 8).unwrap();
    reopened
        .load(&f.identity, &|| false)
        .unwrap()
        .unwrap()
        .verify(|| false)
        .unwrap();
}

#[test]
fn durable_pool_reopens_exact_inodes_without_the_original_installation() {
    let f = Fixture::new();
    let pool = f.pool();
    let first = pool
        .acquire(&f.source, Some(&f.identity), || false)
        .unwrap();
    let root = first.root().to_path_buf();
    assert_eq!(root, f.entry().join("tree"));
    let inode = fs::metadata(root.join("bin/compiler")).unwrap().ino();
    assert_ne!(
        inode,
        fs::metadata(f.source.join("bin/compiler")).unwrap().ino()
    );
    assert_eq!(first.entry_count().unwrap(), 6);
    drop(first);
    drop(pool);
    fs::rename(
        &f.source,
        f.source.with_file_name("unavailable-installation"),
    )
    .unwrap();
    let pool = f.pool();
    assert!(
        pool.lock().unwrap().entries.is_empty(),
        "startup must not fabricate verified in-memory hits"
    );
    let lease = pool.lookup(&f.identity, || false).unwrap().unwrap();
    assert_eq!(lease.root(), root);
    assert_eq!(lease.disposition(), "reused");
    assert_eq!(
        fs::metadata(root.join("bin/compiler")).unwrap().ino(),
        inode
    );
    assert_eq!(fs::read(root.join("bin/compiler")).unwrap(), BYTES);
    assert_eq!(
        fs::read_link(root.join("bin/rustc")).unwrap(),
        Path::new("compiler")
    );
    assert!(root.join("lib/empty").is_dir());
    assert_eq!(f.objects(), vec![f.entry()]);
    assert_eq!(fs::read_dir(pool.directory.0.path()).unwrap().count(), 0);
    // Actual execution acquisition also finds the retained tree without having
    // to trust or reopen the vanished installation path supplied by a caller.
    let execution = pool
        .acquire(&f.source, Some(&f.identity), || false)
        .unwrap();
    assert_eq!(execution.root(), root);
    assert_eq!(
        fs::metadata(execution.root().join("bin/compiler"))
            .unwrap()
            .ino(),
        inode
    );
    execution.verify(|| false).unwrap();
}

#[test]
fn the_last_execution_lease_keeps_the_durable_store_exclusively_owned() {
    let f = Fixture::new();
    let pool = f.pool();
    // Keep the descriptor lifetime of an unrelated fork separate from the
    // cache's real execution leases. The last lease must remain the owner.
    let inherited = pool
        .directory
        .1
        .as_ref()
        .unwrap()
        ._lock
        .0
        .try_clone()
        .unwrap();
    let lease = pool
        .acquire(&f.source, Some(&f.identity), || false)
        .unwrap();
    drop(pool);
    assert!(ToolchainPool::with_directory(1024, 8, Some(&f.cache)).is_err());
    lease.verify(|| false).unwrap();
    let root = lease.root().to_path_buf();
    drop(lease);
    assert!(
        root.exists(),
        "a persistent tree is not TempDir-owned execution scratch"
    );
    let reopened = f.pool();
    assert!(reopened.lookup(&f.identity, || false).unwrap().is_some());
    drop(inherited);
    assert!(ToolchainPool::with_directory(1024, 8, Some(&f.cache)).is_err());
}

#[test]
fn mismatching_identity_hints_never_select_or_rename_a_retained_dataset() {
    let f = Fixture::new();
    let pool = f.pool();
    drop(
        pool.acquire(&f.source, Some(&f.identity), || false)
            .unwrap(),
    );
    drop(pool);
    let pool = f.pool();
    let before = f.objects();
    for identity in [
        ToolchainIdentity {
            sha256: [0; 32],
            ..f.identity
        },
        ToolchainIdentity {
            files: f.identity.files + 1,
            ..f.identity
        },
        ToolchainIdentity {
            bytes: f.identity.bytes + 1,
            ..f.identity
        },
    ] {
        assert!(pool.lookup(&identity, || false).unwrap().is_none());
    }
    assert_eq!(f.objects(), before);
    assert!(pool.lock().unwrap().entries.is_empty());
    drop(pool);
    let foreign = ToolchainIdentity {
        sha256: [0x33; 32],
        ..f.identity
    };
    let renamed = f.cache.join(OBJECTS).join(name(&foreign));
    fs::rename(f.entry(), &renamed).unwrap();
    let pool = f.pool();
    assert!(
        pool.lookup(&foreign, || false).is_err(),
        "a valid-looking filename is not content evidence"
    );
    assert_eq!(fs::read(renamed.join("tree/bin/compiler")).unwrap(), BYTES);
}

#[test]
fn reopen_rejects_corruption_writable_files_aliases_and_extra_namespace() {
    for fault in 0..4 {
        let f = Fixture::new();
        let pool = f.pool();
        drop(
            pool.acquire(&f.source, Some(&f.identity), || false)
                .unwrap(),
        );
        drop(pool);
        let file = f.entry().join("tree/bin/compiler");
        match fault {
            0 => {
                fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
                fs::write(&file, vec![b'x'; BYTES.len()]).unwrap();
                fs::set_permissions(&file, fs::Permissions::from_mode(0o555)).unwrap();
            }
            1 => fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap(),
            2 => fs::hard_link(&file, f.cache.parent().unwrap().join("writable-alias")).unwrap(),
            _ => fs::write(f.entry().join("unexpected"), b"not a dataset member").unwrap(),
        }
        let before = fs::read(&file).unwrap();
        let pool = f.pool();
        assert!(pool.lookup(&f.identity, || false).is_err(), "fault {fault}");
        assert!(
            pool.acquire(&f.source, Some(&f.identity), || false)
                .is_err(),
            "configured persistence must not replace corrupt evidence with the original source"
        );
        assert_eq!(fs::read(&file).unwrap(), before);
        assert_eq!(f.objects(), vec![f.entry()]);
    }
}

#[test]
fn disk_capacity_keeps_old_datasets_and_uses_verified_private_overflow() {
    for by_bytes in [false, true] {
        let f = Fixture::new();
        let budget = if by_bytes { f.identity.bytes } else { 1024 };
        let entries = if by_bytes { 8 } else { 1 };
        let pool = ToolchainPool::with_directory(budget, entries, Some(&f.cache)).unwrap();
        let first = pool
            .acquire(&f.source, Some(&f.identity), || false)
            .unwrap();
        let root = first.root().to_path_buf();
        drop(first);
        fs::write(f.source.join("bin/compiler"), b"other!").unwrap();
        let second_id =
            fingerprint_toolchain(&f.source, &ToolchainLimits::default(), || false).unwrap();
        let second = pool.acquire(&f.source, Some(&second_id), || false).unwrap();
        assert!(!second.root().starts_with(&f.cache));
        assert_eq!(
            fs::read(second.root().join("bin/compiler")).unwrap(),
            b"other!"
        );
        assert!(
            root.exists(),
            "in-memory LRU eviction must never delete durable input"
        );
        assert_eq!(f.objects(), vec![f.entry()]);
        second.verify(|| false).unwrap();
        drop(second);
        drop(pool);
        let pool = ToolchainPool::with_directory(budget, entries, Some(&f.cache)).unwrap();
        assert!(pool.lookup(&f.identity, || false).unwrap().is_some());
        assert!(pool.lookup(&second_id, || false).unwrap().is_none());
    }
}

#[test]
fn interrupted_reservations_remain_charged_and_are_never_adopted_on_restart() {
    let f = Fixture::new();
    let cache = DurableCache::open(&f.cache, f.identity.bytes, 1).unwrap();
    let error = cache
        .capture(&f.source, &f.identity, &|| !f.objects().is_empty())
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(f.entry().is_dir());
    assert!(!f.entry().join(COMPLETE).exists());
    let retained = f.objects();
    assert_eq!(retained.len(), 1);
    assert_eq!(cache.catalogue(&|| false).unwrap().bytes, f.identity.bytes);
    assert_eq!(cache.catalogue(&|| false).unwrap().entries, 1);
    drop(cache);
    let cache = DurableCache::open(&f.cache, f.identity.bytes, 1).unwrap();
    assert!(cache.load(&f.identity, &|| false).unwrap().is_none());
    assert!(
        cache
            .capture(&f.source, &f.identity, &|| false)
            .unwrap()
            .is_none()
    );
    assert_eq!(f.objects(), retained);
    drop(cache);
    let pool = ToolchainPool::with_directory(f.identity.bytes, 1, Some(&f.cache)).unwrap();
    let lease = pool
        .acquire(&f.source, Some(&f.identity), || false)
        .unwrap();
    assert!(!lease.root().starts_with(&f.cache));
    lease.verify(|| false).unwrap();
    assert_eq!(
        f.objects(),
        retained,
        "private overflow cannot erase or reset a pending reservation"
    );
}

#[test]
fn a_complete_unpublished_tree_is_not_a_hit_but_post_publish_cancellation_is_recoverable() {
    let f = Fixture::new();
    let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
    let error = cache
        .capture(&f.source, &f.identity, &|| {
            f.entry().join(COMPLETE).exists()
        })
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(f.entry().exists());
    let verified = cache.load(&f.identity, &|| false).unwrap().unwrap();
    verified.verify(|| false).unwrap();
    drop(verified);
    drop(cache);
    let pending = f
        .cache
        .join(OBJECTS)
        .join(format!("{PENDING}{}-Crash7", name(&f.identity)));
    fs::rename(f.entry(), &pending).unwrap();
    let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
    assert!(cache.load(&f.identity, &|| false).unwrap().is_none());
    assert_eq!(cache.catalogue(&|| false).unwrap().entries, 1);
    let recaptured = cache
        .capture(&f.source, &f.identity, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(recaptured.root(), f.entry().join("tree"));
    assert!(pending.join("tree/bin/compiler").exists());
    assert_eq!(cache.catalogue(&|| false).unwrap().entries, 2);
}

#[test]
fn complete_unmarked_entries_are_charged_but_never_adopted_or_overwritten() {
    let f = Fixture::new();
    let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
    make_directory(&f.entry()).unwrap();
    let prepared = capture_toolchain(
        &f.source,
        &f.entry().join("tree"),
        Some(&f.identity),
        &ToolchainLimits::default(),
        || false,
    )
    .unwrap();
    prepared.sync(|| false).unwrap();
    File::open(f.entry()).unwrap().sync_all().unwrap();
    let inode = fs::metadata(f.entry().join("tree/bin/compiler"))
        .unwrap()
        .ino();
    drop(prepared);
    assert!(cache.load(&f.identity, &|| false).unwrap().is_none());
    assert!(
        cache
            .capture(&f.source, &f.identity, &|| false)
            .unwrap()
            .is_none()
    );
    drop(cache);
    let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
    assert_eq!(cache.catalogue(&|| false).unwrap().entries, 1);
    assert_eq!(cache.catalogue(&|| false).unwrap().bytes, f.identity.bytes);
    assert!(cache.load(&f.identity, &|| false).unwrap().is_none());
    assert!(
        cache
            .capture(&f.source, &f.identity, &|| false)
            .unwrap()
            .is_none()
    );
    assert!(!f.entry().join(COMPLETE).exists());
    assert_eq!(f.objects(), vec![f.entry()]);
    assert_eq!(
        fs::metadata(f.entry().join("tree/bin/compiler"))
            .unwrap()
            .ino(),
        inode
    );
    assert_eq!(
        fs::read(f.entry().join("tree/bin/compiler")).unwrap(),
        BYTES
    );
}

#[test]
fn completion_marker_collisions_never_replace_or_poison_published_entries() {
    let f = Fixture::new();
    let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
    let lease = cache
        .capture(&f.source, &f.identity, &|| false)
        .unwrap()
        .unwrap();
    let marker_inode = fs::metadata(f.entry().join(COMPLETE)).unwrap().ino();
    let tree_inode = fs::metadata(lease.root().join("bin/compiler"))
        .unwrap()
        .ino();
    assert_eq!(
        cache.publish(&f.entry(), &|| false).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let next = cache.load(&f.identity, &|| false).unwrap().unwrap();
    assert_eq!(
        fs::metadata(next.root().join("bin/compiler"))
            .unwrap()
            .ino(),
        tree_inode
    );
    assert_eq!(
        fs::metadata(f.entry().join(COMPLETE)).unwrap().ino(),
        marker_inode
    );
    assert_eq!(f.objects(), vec![f.entry()]);
}

#[test]
fn completion_marker_corruption_links_and_permissions_fail_closed() {
    for fault in 0..5 {
        let f = Fixture::new();
        let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
        drop(
            cache
                .capture(&f.source, &f.identity, &|| false)
                .unwrap()
                .unwrap(),
        );
        drop(cache);
        let marker = f.entry().join(COMPLETE);
        let retained = f.cache.parent().unwrap().join("retained-marker");
        match fault {
            0 => fs::write(&marker, b"not complete").unwrap(),
            1 => fs::set_permissions(&marker, fs::Permissions::from_mode(0o644)).unwrap(),
            2 => fs::hard_link(&marker, &retained).unwrap(),
            3 => {
                fs::rename(&marker, &retained).unwrap();
                symlink(&retained, &marker).unwrap();
            }
            _ => {
                fs::rename(&marker, &retained).unwrap();
                fs::create_dir(&marker).unwrap();
            }
        }
        let inode = fs::symlink_metadata(&marker).unwrap().ino();
        let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
        assert!(cache.load(&f.identity, &|| false).is_err(), "fault {fault}");
        assert!(
            cache.capture(&f.source, &f.identity, &|| false).is_err(),
            "fault {fault}"
        );
        assert_eq!(fs::symlink_metadata(&marker).unwrap().ino(), inode);
        assert_eq!(
            fs::read(f.entry().join("tree/bin/compiler")).unwrap(),
            BYTES
        );
    }
}

#[test]
fn visible_marker_cannot_grant_a_lease_until_publication_sync_is_confirmed() {
    use std::sync::mpsc;
    use std::time::Duration;

    for fail_sync in [false, true] {
        let f = Fixture::new();
        let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
        make_directory(&f.entry()).unwrap();
        let prepared = capture_toolchain(
            &f.source,
            &f.entry().join("tree"),
            Some(&f.identity),
            &ToolchainLimits::default(),
            || false,
        )
        .unwrap();
        prepared.sync(|| false).unwrap();
        File::open(f.entry()).unwrap().sync_all().unwrap();
        let inode = fs::metadata(prepared.root().join("bin/compiler"))
            .unwrap()
            .ino();
        drop(prepared);
        std::thread::scope(|scope| {
            let (created_tx, created_rx) = mpsc::sync_channel(1);
            let (release_tx, release_rx) = mpsc::sync_channel(1);
            let cache_ref = &cache;
            let fixture = &f;
            let publisher = scope.spawn(move || {
                cache_ref.publish_with_sync(&fixture.entry(), &|| false, |marker, directory| {
                    created_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    marker.sync_all()?;
                    if fail_sync {
                        // A deterministic I/O fault after marker visibility, not
                        // a claim to emulate a filesystem/server power loss.
                        return Err(io::Error::other("injected directory sync failure"));
                    }
                    directory.sync_all()
                })
            });
            created_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(f.entry().join(COMPLETE).is_file());
            assert!(matches!(
                cache.publication_confirmed.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            let (started_tx, started_rx) = mpsc::sync_channel(1);
            let (loaded_tx, loaded_rx) = mpsc::sync_channel(1);
            let reader = scope.spawn(move || {
                started_tx.send(()).unwrap();
                loaded_tx
                    .send(cache_ref.load(&fixture.identity, &|| false))
                    .unwrap();
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(
                loaded_rx.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            // A different lookup must still observe cancellation while the
            // real publication path holds the gate around its sync operation.
            let (waiting_tx, waiting_rx) = mpsc::sync_channel(1);
            let (cancelled_tx, cancelled_rx) = mpsc::sync_channel(1);
            let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancel_ref = std::sync::Arc::clone(&cancelled);
            let waiter = scope.spawn(move || {
                let checkpoints = std::sync::atomic::AtomicUsize::new(0);
                let result = cache_ref.load(&fixture.identity, &|| {
                    use std::sync::atomic::Ordering;
                    // Initial load check is zero; check two is a retry after
                    // the first failed publication-lock acquisition.
                    if checkpoints.fetch_add(1, Ordering::SeqCst) == 2 {
                        waiting_tx.send(()).unwrap();
                    }
                    cancel_ref.load(Ordering::SeqCst)
                });
                cancelled_tx.send(result).unwrap();
            });
            waiting_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            let cancellation = cancelled_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(
                cancellation.err().unwrap().kind(),
                io::ErrorKind::Interrupted
            );
            waiter.join().unwrap();
            assert!(matches!(
                cache.publication_confirmed.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            release_tx.send(()).unwrap();
            let published = publisher.join().unwrap();
            let loaded = loaded_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            reader.join().unwrap();
            if fail_sync {
                assert!(
                    published
                        .unwrap_err()
                        .to_string()
                        .contains("injected directory sync failure")
                );
                assert!(
                    loaded
                        .err()
                        .unwrap()
                        .to_string()
                        .contains("durability unconfirmed")
                );
                assert!(cache.capture(&f.source, &f.identity, &|| false).is_err());
            } else {
                published.unwrap();
                loaded.unwrap().unwrap().verify(|| false).unwrap();
            }
        });
        drop(cache);
        // Reopening really syncs the marker and parent before reusing the exact
        // original verified inode after an uncertain prior publication.
        let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
        let lease = cache.load(&f.identity, &|| false).unwrap().unwrap();
        assert_eq!(
            fs::metadata(lease.root().join("bin/compiler"))
                .unwrap()
                .ino(),
            inode
        );
        lease.verify(|| false).unwrap();
    }
}

#[test]
fn optional_writer_contention_never_blocks_verified_lookup_or_creates_a_reservation() {
    let f = Fixture::new();
    let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
    drop(
        cache
            .capture(&f.source, &f.identity, &|| false)
            .unwrap()
            .unwrap(),
    );
    let before = f.objects();
    let writer = cache.writer.lock().unwrap();
    assert!(
        cache
            .capture(Path::new("/absent"), &f.identity, &|| false)
            .unwrap()
            .is_none()
    );
    assert!(cache.load(&f.identity, &|| false).unwrap().is_some());
    assert_eq!(f.objects(), before);
    drop(writer);
}

#[test]
fn persistent_storage_is_protected_even_for_an_unpinned_private_execution() {
    let f = Fixture::new();
    let pool = f.pool();
    let pinned = pool
        .acquire(&f.source, Some(&f.identity), || false)
        .unwrap();
    let private = pool.acquire(&f.source, None, || false).unwrap();
    let writable = f.cache.parent().unwrap().join("writable");
    fs::create_dir(&writable).unwrap();
    let alias = writable.join("alias");
    symlink(&f.cache, &alias).unwrap();
    let objects = f.cache.join(OBJECTS);
    for lease in [&pinned, &private] {
        let mut valid = CanonicalNamespaceSpec::new();
        valid
            .ro_binds
            .push(Bind::new(lease.root(), rabs_sandbox::layout::TOOLCHAIN));
        valid
            .rw_binds
            .push(Bind::new(&writable, rabs_sandbox::layout::WORKSPACE));
        lease.validate_namespace(&valid).unwrap();
        for path in [
            f.cache.as_path(),
            objects.as_path(),
            alias.as_path(),
            f.cache.parent().unwrap(),
        ] {
            let mut bad = valid.clone();
            bad.rw_binds[0].backing = path.to_path_buf();
            let before = bad.clone();
            assert!(lease.validate_namespace(&bad).is_err());
            assert_eq!(
                bad, before,
                "validation cannot partially rewrite the sandbox"
            );
        }
    }
}

#[test]
fn configuration_lock_and_directory_identity_fail_closed_without_erasing_files() {
    let f = Fixture::new();
    assert!(ToolchainPool::with_directory(0, 8, Some(&f.cache)).is_err());
    assert!(!f.cache.exists());
    assert!(ToolchainPool::with_directory(1024, 8, Some(Path::new("relative"))).is_err());
    let cache = DurableCache::open(&f.cache, 1024, 8).unwrap();
    assert!(DurableCache::open(&f.cache, 1024, 8).is_err());
    fs::rename(f.cache.join(OBJECTS), f.cache.join("old-objects")).unwrap();
    make_directory(&f.cache.join(OBJECTS)).unwrap();
    assert!(
        cache.load(&f.identity, &|| false).is_err(),
        "an open cache cannot follow a replaced directory"
    );
    assert!(f.cache.join("old-objects").exists());
    drop(cache);
    assert!(
        DurableCache::open(&f.cache, 1024, 8).is_err(),
        "unrelated retained data is not silently removed"
    );
}

#[test]
fn cache_paths_links_modes_and_quota_downgrades_are_never_silently_fixed() {
    let f = Fixture::new();
    let pool = f.pool();
    drop(
        pool.acquire(&f.source, Some(&f.identity), || false)
            .unwrap(),
    );
    drop(pool);
    let alias = f.cache.with_file_name("cache-alias");
    symlink(&f.cache, &alias).unwrap();
    assert!(DurableCache::open(&alias, 1024, 8).is_err());
    assert!(DurableCache::open(&f.cache, f.identity.bytes - 1, 8).is_err());
    fs::set_permissions(&f.cache, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(DurableCache::open(&f.cache, 1024, 8).is_err());
    assert_eq!(fs::metadata(&f.cache).unwrap().mode() & 0o7777, 0o755);
    fs::set_permissions(&f.cache, fs::Permissions::from_mode(0o700)).unwrap();
    fs::hard_link(f.cache.join(LOCK), f.cache.with_file_name("lock-alias")).unwrap();
    assert!(DurableCache::open(&f.cache, 1024, 8).is_err());
    assert_eq!(fs::metadata(f.cache.join(LOCK)).unwrap().nlink(), 2);
    assert_eq!(
        fs::read(f.entry().join("tree/bin/compiler")).unwrap(),
        BYTES
    );
}

#[test]
fn persistent_reservation_names_use_complete_canonical_bounded_identity() {
    let id = ToolchainIdentity {
        sha256: [0xab; 32],
        files: 1,
        bytes: 11,
    };
    let good = name(&id);
    assert_eq!(identity_from_name(&good).unwrap(), id);
    assert_eq!(
        catalogue_identity(&format!("{PENDING}{good}-Ab1234")).unwrap(),
        id
    );
    for bad in [
        String::new(),
        good.replace("v1-", "v2-"),
        good.to_uppercase(),
        format!("v1-{}-01-11", "ab".repeat(32)),
        format!("v1-{}-1-18446744073709551616", "ab".repeat(32)),
        format!("v1-{}-100001-11", "ab".repeat(32)),
        format!("v1-{}-1-8589934593", "ab".repeat(32)),
        format!("{good}-extra"),
        format!("{PENDING}{good}-"),
        format!("{PENDING}{good}-../../outside"),
    ] {
        assert!(catalogue_identity(&bad).is_err(), "{bad}");
    }
}
