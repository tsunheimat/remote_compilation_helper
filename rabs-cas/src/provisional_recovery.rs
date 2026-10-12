//! Ownership-safe recovery of provisional installs after lineage
//! failure (bead M019; risk R86; plan §88).
//!
//! When provisional lineage fails — producer generation failed,
//! superseded with divergent objects, authority lost (M007), or a
//! different winner committed divergent bytes (M017) — every output that
//! was already INSTALLED to a real path by a consuming operation is a
//! liability: Cargo may have fingerprinted against it, and a blind
//! delete could destroy user state RABS does not own.
//!
//! The journal ([`crate::metadata_store::RabsMetadataStore::
//! insert_provisional_install`]) records every such install. Recovery is
//! ownership-safe by construction:
//!
//! - ONLY paths recorded verbatim in the journal are ever touched;
//! - a recorded path is REMOVED only when its current bytes still hash
//!   to the exact identity recorded at install time (`removed`);
//! - a path whose bytes changed, or that cannot be verified, is marked
//!   `dirty` and LEFT IN PLACE for Cargo revalidation or an explicit
//!   private target reset;
//! - a path already gone is bookkept as `removed` without error.
//!
//! ACCEPTANCE anchor (with M016): dirty-target scenarios show no skipped
//! invalid dependents on the next build.
//!
//! # Dependency rules
//!
//! Same as the crate: `rabs-protocol` types only; no async runtime.
//! Filesystem effects live HERE (this crate owns durable storage);
//! metadata effects flow through [`RabsMetadataStore`].

use std::path::PathBuf;

use crate::blob_store::recompute_file_digest;
use crate::metadata_store::{ProvisionalInstallInsert, RabsMetadataStore, StoreError};
use rabs_protocol::generation::AttemptId;

/// Outcome of one recovery sweep over failed lineage's installed
/// outputs. Every journal row ends in a terminal state (`removed` or
/// `dirty`) or was already terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecoverySummary {
    /// Paths deleted because current bytes still matched the recorded
    /// identity exactly, plus paths that were already gone.
    pub removed: usize,
    /// Paths PRESERVED but marked `dirty`: bytes diverged from what this
    /// operation installed, so ownership cannot be proven. Cargo must
    /// revalidate, or the target requires an explicit private reset.
    pub marked_dirty: usize,
}

impl RecoverySummary {
    /// Whether any path was left dirty (needs revalidation).
    #[must_use]
    pub fn has_dirty(&self) -> bool {
        self.marked_dirty > 0
    }
}

/// Record one provisional output installed to `path` (M019). The digest
/// is RECOMPUTED FROM DISK at record time — the journal captures what IS
/// there, not what the caller believes — so later ownership checks are
/// honest even if the installer's view drifted.
///
/// Idempotent per (pin, attempt, path): re-recording keeps the FIRST
/// identity (INSERT OR IGNORE semantics).
///
/// # Errors
/// Store failures; unreadable/unhashable path.
pub fn record_installed_output(
    store: &mut dyn RabsMetadataStore,
    pin_key: &str,
    consumer_worker: &str,
    consumer_attempt: AttemptId,
    path: &std::path::Path,
    installed_seq: u64,
) -> Result<(), ProvisionalInstallError> {
    let object =
        recompute_file_digest(path).map_err(|_| ProvisionalInstallError::UnreadablePath {
            path: path.as_os_str().as_encoded_bytes().to_vec(),
        })?;
    store.insert_provisional_install(&ProvisionalInstallInsert {
        pin_key: pin_key.to_owned(),
        consumer_worker: consumer_worker.to_owned(),
        consumer_attempt: consumer_attempt.0,
        installed_path: path.as_os_str().as_encoded_bytes().to_vec(),
        object,
        installed_seq,
    })?;
    Ok(())
}

/// Sweep the journal for `root_pin_keys` PLUS their full transitive
/// descendant pins (the same set M017's cascade invalidates) and recover
/// ownership-safely. Metadata-side invalidation is the caller's job
/// (M007/M017 triggers); this closes the filesystem side coherently.
///
/// Never deletes user state: removal requires byte-exact identity match.
///
/// # Errors
/// Store failures.
pub fn recover_after_lineage_failure(
    store: &mut dyn RabsMetadataStore,
    root_pin_keys: &[String],
) -> Result<RecoverySummary, ProvisionalInstallError> {
    // Expand roots to the transitive descendant closure — the same
    // reachability the invalidation cascade used — so no descendant's
    // installs are skipped (M016's "no skipped invalid dependents").
    let mut visited = std::collections::BTreeSet::new();
    let mut frontier: Vec<String> = root_pin_keys.to_vec();
    while let Some(key) = frontier.pop() {
        if visited.insert(key.clone()) {
            frontier.extend(store.list_provisional_pin_descendants(&key)?);
        }
    }
    let pin_keys: Vec<String> = visited.into_iter().collect();
    let rows = store.list_provisional_installs_for_pins(&pin_keys)?;

    let mut summary = RecoverySummary::default();
    for row in rows {
        if row.state != "installed" {
            continue; // already recovered in an earlier sweep
        }
        let outcome = decode_installed_path(&row.installed_path).and_then(|path| {
            match std::fs::metadata(&path) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    // Already gone: bookkeep, never error.
                    Ok(true)
                }
                Err(_) => Err(()),
                Ok(_) => match recompute_file_digest(&path) {
                    Ok(current) if current == row.object => fs_remove(&path),
                    _ => Err(()),
                },
            }
        });
        match outcome {
            Ok(removed_now) => {
                store.set_provisional_install_state(
                    &row.pin_key,
                    &row.consumer_attempt_hex,
                    &row.installed_path,
                    "removed",
                )?;
                summary.removed += 1;
                let _ = removed_now;
            }
            Err(()) => {
                store.set_provisional_install_state(
                    &row.pin_key,
                    &row.consumer_attempt_hex,
                    &row.installed_path,
                    "dirty",
                )?;
                summary.marked_dirty += 1;
            }
        }
    }
    Ok(summary)
}

// Preserve Unix path bytes exactly. On other platforms, only UTF-8 journal
// paths can be decoded safely with the standard library. An undecodable path
// becomes dirty without touching the filesystem; lossy decoding could delete
// a different file. This also preserves Windows paths with unpaired surrogates
// for explicit revalidation rather than guessing their identity.
fn decode_installed_path(bytes: &[u8]) -> Result<PathBuf, ()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes)
            .map(PathBuf::from)
            .map_err(|_| ())
    }
}

fn fs_remove(path: &std::path::Path) -> Result<bool, ()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(()),
    }
}

/// Everything the recovery layer can fail with. Store failures carry
/// verbatim; filesystem verification failures become `dirty` marks, not
/// errors — a sweep never aborts half-recovered over one hostile path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionalInstallError {
    /// Underlying store failure.
    Store(StoreError),
    /// The path could not be read/hashed at record time.
    UnreadablePath {
        /// Path bytes attempted.
        path: Vec<u8>,
    },
}

impl From<StoreError> for ProvisionalInstallError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

impl std::fmt::Display for ProvisionalInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(e) => write!(f, "store error: {e:?}"),
            Self::UnreadablePath { path } => {
                write!(
                    f,
                    "cannot hash installed path {:?}",
                    String::from_utf8_lossy(path)
                )
            }
        }
    }
}
impl std::error::Error for ProvisionalInstallError {}

// ---------------------------------------------------------------------
// Tests — the M019 acceptance suite: journal + ownership-safe recovery.
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata_store::{RusqliteEngine, SqlMetadataStore};
    use crate::provisional_pins::{
        ProducerContracts, ProvisionalIdentity, ProvisionalReader, TerminalGate, authorize_reader,
        descendant_terminal_gate, invalidate_lineage, open_provisional_pin, resolve_for_reader,
    };
    use rabs_protocol::authority::{ClusterId, CoordinatorAuthority};
    use rabs_protocol::generation::{ActionGenerationId, ExecutionLeaseId};
    use rabs_protocol::raw_bytes::RawBytes;
    use rabs_protocol::result_identity::{DigestAlgorithm, ObjectId, OutputRole, TypedDigest};

    fn unique_tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "m019-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fixture() -> SqlMetadataStore<RusqliteEngine> {
        let engine = RusqliteEngine::open_in_memory().unwrap();
        SqlMetadataStore::open(engine).unwrap()
    }

    fn identity(attempt_tag: u128) -> ProvisionalIdentity {
        let mut bytes = [0u8; 32];
        bytes[0] = 10;
        bytes[31] = 10;
        ProvisionalIdentity {
            authority: CoordinatorAuthority {
                cluster_id: ClusterId("cluster-1".to_owned()),
                credential_generation: 1,
                term: 101,
                incarnation_id: rabs_protocol::authority::CoordinatorIncarnationId(0xAA01),
            },
            action_key: TypedDigest {
                algorithm: DigestAlgorithm::Sha256V1,
                domain: "rabs.action-key.sha256.v1",
                bytes,
            },
            generation: ActionGenerationId(0x50),
            attempt: AttemptId(attempt_tag),
            lease: ExecutionLeaseId(attempt_tag + 1),
            role: OutputRole::ProvisionalMetadata,
            virtual_path: RawBytes::new(b"target/debug/deps/libfeat.rmeta".to_vec()),
        }
    }

    fn contracts() -> ProducerContracts {
        let mut bytes = [0u8; 32];
        bytes[0] = 200;
        ProducerContracts {
            toolchain: TypedDigest {
                algorithm: DigestAlgorithm::Sha256V1,
                domain: "rabs.action-key.sha256.v1",
                bytes,
            },
            events: TypedDigest {
                algorithm: DigestAlgorithm::Sha256V1,
                domain: "rabs.action-key.sha256.v1",
                bytes: [1u8; 32],
            },
        }
    }

    /// A pin plus a dependent that consumed it AND installed its early
    /// output to a real path (the M016 nasty ordering, minimally).
    fn pin_with_installing_dependent(
        store: &mut SqlMetadataStore<RusqliteEngine>,
        dir: &std::path::Path,
        file_name: &str,
        contents: &[u8],
    ) -> (ProvisionalIdentity, String, PathBuf) {
        let producer = identity(30);
        open_provisional_pin(
            store,
            &producer,
            &ObjectId(contracts().toolchain),
            &contracts(),
        )
        .unwrap();
        authorize_reader(
            store,
            &producer,
            &ProvisionalReader::DependentAttempt {
                worker: "worker-b".to_owned(),
                attempt: AttemptId(31),
            },
        )
        .unwrap();
        resolve_for_reader(
            store,
            &producer,
            &ProvisionalReader::DependentAttempt {
                worker: "worker-b".to_owned(),
                attempt: AttemptId(31),
            },
        )
        .unwrap();

        let path = dir.join(file_name);
        std::fs::write(&path, contents).unwrap();
        record_installed_output(
            store,
            &producer.pin_key(),
            "worker-b",
            AttemptId(31),
            &path,
            7,
        )
        .unwrap();
        (producer.clone(), producer.pin_key(), path)
    }

    #[test]
    fn m019_exact_match_is_removed_and_journal_closed() {
        let mut store = fixture();
        let dir = unique_tmp("exact");
        let (producer, pin_key, path) =
            pin_with_installing_dependent(&mut store, &dir, "out.rmeta", b"installed-bytes");

        assert_eq!(
            recover_after_lineage_failure(&mut store, std::slice::from_ref(&pin_key)).unwrap(),
            RecoverySummary {
                removed: 1,
                marked_dirty: 0
            }
        );
        assert!(!path.exists());
        // Journal reached a terminal state; second sweep is a no-op.
        let rows = store
            .list_provisional_installs_for_pins(&[pin_key])
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, "removed");
        assert_eq!(
            recover_after_lineage_failure(&mut store, &[producer.pin_key()]).unwrap(),
            RecoverySummary::default()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn m019_user_overwrite_is_marked_dirty_and_preserved() {
        let mut store = fixture();
        let dir = unique_tmp("dirty");
        let (_producer, pin_key, path) =
            pin_with_installing_dependent(&mut store, &dir, "out.rmeta", b"installed-bytes");

        // USER state: someone rewrote the file after RABS installed it.
        std::fs::write(&path, b"user-edited-content").unwrap();

        let summary =
            recover_after_lineage_failure(&mut store, std::slice::from_ref(&pin_key)).unwrap();
        assert_eq!(
            summary,
            RecoverySummary {
                removed: 0,
                marked_dirty: 1
            }
        );
        assert!(summary.has_dirty());
        // NEVER guess-delete user state.
        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"user-edited-content");
        // The dirty audit lists it for Cargo revalidation.
        let dirty = store.list_provisional_installs_by_state("dirty").unwrap();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].installed_path, path.as_os_str().as_encoded_bytes());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn m019_unicode_path_is_recovered_verbatim() {
        let mut store = fixture();
        let dir = unique_tmp("unicode");
        let (_, pin_key, path) =
            pin_with_installing_dependent(&mut store, &dir, "output-\u{03bb}.rmeta", b"installed");

        assert_eq!(
            decode_installed_path(path.as_os_str().as_encoded_bytes()).unwrap(),
            path
        );
        assert_eq!(
            recover_after_lineage_failure(&mut store, &[pin_key]).unwrap(),
            RecoverySummary {
                removed: 1,
                marked_dirty: 0
            }
        );
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn m019_non_utf8_unix_path_is_recovered_without_lossy_aliasing() {
        use crate::digest_set::ATP_OBJECT_CONTENT_DOMAIN;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::MetadataExt;

        let dir = unique_tmp("raw-path");
        let database = dir.join("journal.sqlite");
        let mut store = SqlMetadataStore::open(RusqliteEngine::open(&database).unwrap()).unwrap();
        let (_, pin_key, original) =
            pin_with_installing_dependent(&mut store, &dir, "original.rmeta", b"installed");
        let path = dir.join(std::ffi::OsStr::from_bytes(b"output-\xff.rmeta"));
        let lossy_alias = dir.join("output-\u{fffd}.rmeta");
        std::fs::write(&lossy_alias, b"installed").unwrap();
        let alias_identity = || {
            let metadata = std::fs::metadata(&lossy_alias).unwrap();
            (
                metadata.dev(),
                metadata.ino(),
                metadata.nlink(),
                metadata.mode(),
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
            )
        };
        let alias_before = alias_identity();
        assert_eq!(
            decode_installed_path(path.as_os_str().as_bytes()).unwrap(),
            path
        );
        assert_ne!(path, lossy_alias);
        // A valid-name rename on the same volume must work before a native
        // filename refusal can explain the invalid-name result.
        let control = dir.join("rename-control.rmeta");
        std::fs::rename(&original, &control).unwrap();
        std::fs::rename(&control, &original).unwrap();
        let before = store
            .list_provisional_installs_for_pins(std::slice::from_ref(&pin_key))
            .unwrap();
        let expected_removed = match std::fs::rename(&original, &path) {
            Ok(()) => {
                record_installed_output(&mut store, &pin_key, "worker-b", AttemptId(31), &path, 8)
                    .unwrap();
                2
            }
            Err(error) => {
                #[cfg(target_os = "macos")]
                let expected_refusal = Some(rustix::io::Errno::ILSEQ.raw_os_error());
                #[cfg(not(target_os = "macos"))]
                let expected_refusal: Option<i32> = None;
                assert!(
                    expected_refusal.is_some() && error.raw_os_error() == expected_refusal,
                    "unexpected native filename refusal: {error:?}"
                );
                assert_eq!(std::fs::read(&original).unwrap(), b"installed");
                assert_eq!(
                    record_installed_output(
                        &mut store,
                        &pin_key,
                        "worker-b",
                        AttemptId(31),
                        &path,
                        8
                    ),
                    Err(ProvisionalInstallError::UnreadablePath {
                        path: path.as_os_str().as_bytes().to_vec()
                    })
                );
                assert_eq!(
                    store
                        .list_provisional_installs_for_pins(std::slice::from_ref(&pin_key))
                        .unwrap(),
                    before,
                    "a refused path must not create or change a journal identity"
                );
                1
            }
        };
        let installed = store
            .list_provisional_installs_for_pins(std::slice::from_ref(&pin_key))
            .unwrap();
        assert_eq!(installed.len(), expected_removed);
        assert_eq!(std::fs::read(&lossy_alias).unwrap(), b"installed");
        drop(store);
        let mut store = SqlMetadataStore::open(RusqliteEngine::open(&database).unwrap()).unwrap();
        // Fresh readers declare the same static object domain as production
        // coordinator startup; unknown stored domains still fail closed.
        store.intern_domain(ATP_OBJECT_CONTENT_DOMAIN);
        assert_eq!(
            store
                .list_provisional_installs_for_pins(std::slice::from_ref(&pin_key))
                .unwrap(),
            installed
        );

        assert_eq!(
            recover_after_lineage_failure(&mut store, std::slice::from_ref(&pin_key)).unwrap(),
            RecoverySummary {
                removed: expected_removed,
                marked_dirty: 0
            }
        );
        assert!(!original.exists());
        assert!(!path.exists());
        assert_eq!(std::fs::read(&lossy_alias).unwrap(), b"installed");
        let mut expected_terminal = installed;
        for row in &mut expected_terminal {
            row.state = "removed".to_owned();
        }
        drop(store);
        let mut store = SqlMetadataStore::open(RusqliteEngine::open(&database).unwrap()).unwrap();
        store.intern_domain(ATP_OBJECT_CONTENT_DOMAIN);
        assert_eq!(
            store
                .list_provisional_installs_for_pins(std::slice::from_ref(&pin_key))
                .unwrap(),
            expected_terminal,
            "terminal state is durable without changing raw paths or object identities"
        );
        assert_eq!(
            recover_after_lineage_failure(&mut store, std::slice::from_ref(&pin_key)).unwrap(),
            RecoverySummary::default()
        );
        assert_eq!(std::fs::read(&lossy_alias).unwrap(), b"installed");
        assert_eq!(alias_identity(), alias_before);
        drop(store);
        let mut store = SqlMetadataStore::open(RusqliteEngine::open(&database).unwrap()).unwrap();
        store.intern_domain(ATP_OBJECT_CONTENT_DOMAIN);
        assert_eq!(
            store
                .list_provisional_installs_for_pins(&[pin_key])
                .unwrap(),
            expected_terminal,
            "a repeated sweep must preserve the durable terminal audit rows"
        );
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(not(unix))]
    fn m019_undecodable_path_is_marked_dirty_without_touching_lossy_alias() {
        let mut store = fixture();
        let dir = unique_tmp("undecodable-path");
        let (_, pin_key, path) =
            pin_with_installing_dependent(&mut store, &dir, "out.rmeta", b"installed");
        let mut encoded = dir.as_os_str().as_encoded_bytes().to_vec();
        encoded.extend_from_slice(b"/output-\xff.rmeta");
        store
            .insert_provisional_install(&ProvisionalInstallInsert {
                pin_key: pin_key.clone(),
                consumer_worker: "worker-b".to_owned(),
                consumer_attempt: 31,
                installed_path: encoded.clone(),
                object: recompute_file_digest(&path).unwrap(),
                installed_seq: 8,
            })
            .unwrap();
        let lossy_alias = dir.join("output-\u{fffd}.rmeta");
        std::fs::write(&lossy_alias, b"installed").unwrap();

        assert_eq!(
            recover_after_lineage_failure(&mut store, &[pin_key]).unwrap(),
            RecoverySummary {
                removed: 1,
                marked_dirty: 1
            }
        );
        assert_eq!(std::fs::read(&lossy_alias).unwrap(), b"installed");
        let dirty = store.list_provisional_installs_by_state("dirty").unwrap();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].installed_path, encoded);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn m019_already_gone_paths_bookkeep_without_error() {
        let mut store = fixture();
        let dir = unique_tmp("gone");
        let (_producer, pin_key, path) =
            pin_with_installing_dependent(&mut store, &dir, "out.rmeta", b"installed-bytes");

        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            recover_after_lineage_failure(&mut store, &[pin_key]).unwrap(),
            RecoverySummary {
                removed: 1,
                marked_dirty: 0
            }
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn m019_cascade_reaches_descendant_pins_installs() {
        let engine = RusqliteEngine::open_in_memory().unwrap();
        let mut store = SqlMetadataStore::open(engine).unwrap();
        let dir = unique_tmp("cascade");

        // A -> B chain: B consumed A and installed B's OWN early output.
        let a = identity(30);
        let b = identity(31);
        open_provisional_pin(
            &mut store,
            &a,
            &ObjectId(contracts().toolchain),
            &contracts(),
        )
        .unwrap();
        authorize_reader(
            &mut store,
            &a,
            &ProvisionalReader::DependentAttempt {
                worker: "worker-b".to_owned(),
                attempt: AttemptId(31),
            },
        )
        .unwrap();
        resolve_for_reader(
            &mut store,
            &a,
            &ProvisionalReader::DependentAttempt {
                worker: "worker-b".to_owned(),
                attempt: AttemptId(31),
            },
        )
        .unwrap();
        open_provisional_pin(&mut store, &b, &ObjectId(contracts().events), &contracts()).unwrap();

        let b_path = dir.join("b-output.rmeta");
        std::fs::write(&b_path, b"b-early-bytes").unwrap();
        record_installed_output(
            &mut store,
            &b.pin_key(), // journaled under DESCENDANT pin B
            "worker-b",
            AttemptId(31),
            &b_path,
            9,
        )
        .unwrap();

        // Lineage fails AT THE ROOT A: metadata invalidation FIRST (the
        // M007/M017 cascade cancels B's obligation), then the recovery
        // sweep must reach B's install through the same materialized
        // descendant closure — no skipped invalid dependents.
        invalidate_lineage(&mut store, &a, "producer generation failed").unwrap();
        let summary = recover_after_lineage_failure(&mut store, &[a.pin_key()]).unwrap();
        assert_eq!(
            summary,
            RecoverySummary {
                removed: 1,
                marked_dirty: 0
            }
        );
        assert!(!b_path.exists());

        // And the terminal gate refuses B permanently (metadata side).
        let gate = descendant_terminal_gate(&mut store, "worker-b", AttemptId(31)).unwrap();
        assert!(matches!(gate, TerminalGate::Refused { .. }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn m019_recording_requires_readable_path() {
        let mut store = fixture();
        let missing = unique_tmp("missing").join("does-not-exist.rmeta");
        let err = record_installed_output(
            &mut store,
            &identity(30).pin_key(),
            "worker-x",
            AttemptId(40),
            &missing,
            1,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ProvisionalInstallError::UnreadablePath {
                path: missing.as_os_str().as_encoded_bytes().to_vec()
            }
        );
        std::fs::remove_dir_all(missing.parent().unwrap()).unwrap();
    }
}
