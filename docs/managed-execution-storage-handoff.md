# Managed execution storage: local Codex handoff

Snapshot: 2026-10-09 UTC, local development continuation. Consumer: the draft PR
reviewer and the user's next local session. The user requested continued
development and said they will deploy later. Keep PR #2 draft. Deployment and
live worker/Nexus acceptance remain with the user.

## Checkout and validation state

- Repository: <https://github.com/tsunheimat/remote_compilation_helper>
- Branch: `feat/managed-execution-storage`
- Draft PR: <https://github.com/tsunheimat/remote_compilation_helper/pull/2>
- Base: `main`, `35375c21719a22c9039c816c143fc09f3aed770b`
- Incoming feature head: `7f68c5dc12caffae49d0860bf3c19e72e7c58a65`
- Incoming code commit: `1536190314d72b55aef3b1cd80245b3c93673bb1`
- Incoming code tree: `50665b2330f0b8c11ba2056cc0a2661f7012e69b`
- Local worktree: `/mnt/vibe-coding-share/develop/remote_compilation_helper-managed-storage`

The repairs described below are included with this development snapshot. The
primary `main` checkout was preserved. Full workspace testing was still running
when this snapshot was prepared; there is no new all-green CI or merge-readiness
claim. The incoming CI failures are retained below as historical evidence.

For a fresh local checkout:

```bash
git clone --branch feat/managed-execution-storage https://github.com/tsunheimat/remote_compilation_helper.git
cd remote_compilation_helper
git status --short --branch
git log -8 --oneline
```

For an existing checkout, inspect its changes before fetching/switching. Preserve
other work; do not reset, clean, overwrite, or delete it. Read `AGENTS.md` and
`/data/projects/AGENTS.md` if present locally. That file and `br` remain unavailable
in this environment. No Beads issues have been closed.

## Local repairs and evidence

The storage cleanup now holds the lease through a temporary hard link outside
the job directory. This avoids NFS retaining an open `.nfs*` entry inside a job
being removed. Cleanup still requires the exclusive lease, original inode, and
owner marker; active descendants retain their scratch. Required worker tools
and filesystem support are documented in `docs/guides/configuration.md`.

The runtime policy retains the existing owned-thread architecture, as selected
after the user delegated that decision. The daemon and its joined executor
threads can own separate runtimes. Synchronous transport entry rejects an active
Asupersync context before I/O. The static gate scans all 345 RABS source files as
Rust syntax, excludes explicit test-only bodies, and pins each reviewed entry to
its function and exact construction/entry counts. Comments and string literals
are no longer mistaken for calls. Async entries, unreviewed calls, opaque macro
entries, and stale allowance counts remain failures. This establishes the
reviewed ownership and no-reentrancy boundaries, not one runtime per process.

Additional reproduced defects were repaired:

- Effective configuration overlays now validate/normalize `remote_base`, retain
  a valid lower-priority value after a rejected override, and agree with source
  inspection. Cache schema 5 invalidates the earlier unvalidated merges.
- Explicit unknown worker probes and failed daemon reloads now return nonzero
  status with one error response. Successful empty all-worker probes and reloads
  retain their successful status. Human error diagnostics go to stderr.
- Mock toolchain preflight uses the same mock transport as health monitoring,
  executes the real preflight command, and preserves failure classifications.
- E2E checks execute the hook's actual delegated command, inspect native
  envelopes and exit codes, verify output bytes/transfer behavior, and require
  positive executed-test counts for named Cargo selections. A failed fixture
  directory change now prevents hook/exec invocation in the caller's directory.
- CI prepares the debug binaries required by `true_e2e` and propagates its
  failure through `tee`. Unix package checksums name the downloaded archive
  basename. Rustdoc links and literal markup were repaired without suppressing
  warnings or changing item visibility.

Local results use `nightly-2026-08-31` (rustc 1.100.0-nightly, `908501772`),
default debug profiles, and CLI Git dependency transport. Targeted compilation
used two jobs. Broader compilation uses one job after the shared host's `/tmp`
tmpfs filled. Isolated mount namespaces bind fresh retained NFS fixtures onto
`/tmp` and drop back to the normal user for tests; operator files are untouched.

| Check | Result at this snapshot |
| --- | --- |
| Workspace, all-target, all-feature check | Passed |
| Workspace, all-target, all-feature Clippy, `-D warnings` | Passed before subsequent doc-comment-only repairs |
| Formatting and whitespace checks | Passed |
| Workspace/all-feature/no-dependency documentation, warnings denied | Passed after all reported markup repairs |
| RCH `managed_` tests | 35 passed, including the original watchdog status assertion of 137 |
| Shared execution storage | 6 passed on NFS; the private-mount case also passed separately with privileges and required actual mounted execution |
| Unchanged full-stderr watchdog regression | Passed with its original assertion |
| Runtime policy | 6 passed; the incoming gate had one pass and two failures |
| Native transport reentry refusal | Passed inside an actual runtime context, with zero reads/writes |
| Remote-base tests | 6 passed |
| Worker probe CLI filter | 3 passed |
| Reload CLI matrix | 1 test passed across 12 real CLI/socket cases |
| Mock preflight | 1 test passed across 6 isolated child-process cases |
| Self-healing | 9 native cases passed, including cooldown and recovery ownership checks |
| Pipeline / output / project sync | 13 / 15 / 14 cases passed again after the cwd guard fix, with mock SSH and native local rsync fixtures |
| Installer | 44 assertions passed across 15 cases; one macOS launchd capability skip; native fixture installs/uninstalls ran in a private `/tmp` namespace |
| API envelopes / API error codes / error experience / saved-time | All four scripts passed; error experience exercised 90 UI unit tests, saved-time exercised 7 unit tests |
| Full workspace test collection | In progress, `cargo test --locked --workspace --no-fail-fast` |
| Feature-gated `true_e2e`, aggregate `e2e_test`, full 41-script runner, release gates | Pending broader execution |

Bun preparation passed its executable-independent cases; four cases requiring
a working Bun runtime were not run. Installer launchd behavior needs macOS.
The source-sync and pipeline fixtures do not establish live SSH or Nexus
acceptance. Local Linux results do not establish Windows/macOS results or parity
with CI's explicit June nightly.

Logs are retained under `target/managed-storage-validation-20261009/`, including
failed attempts. The host `/tmp` exhaustion prevented the first API-script
attempts from opening logs; those attempts are retained and the scripts passed
after isolated reruns. No operator storage was cleaned to obtain a pass.

### UBS result and bounded triage

The user explicitly authorized cleanup of newly generated UBS scratch. Official
UBS v5.4.33 at `89d5f354005a0d1a679b56fe31d65d44d184878a` ran unmodified on 51
changed Rust/Bash files. Its final result is **exit 1**, with 6 critical, 3,817
warning, and 1,350 informational records. This is not a passing scanner result.

Source triage found that all six critical expressions are unchanged in the
incoming HEAD: one public trust-scope enum comparison, the managed executable
selection primitive, two test-only generated GC-script executions, a test-only
shim executable, and the fixed PowerShell/pwsh selection loop. The reported
heuristics do not demonstrate vulnerabilities at these reviewed sites. Other
unchanged warnings were not exhaustively audited.

UBS did find the new delegated-helper cwd bug. Its two warnings disappeared after
the explicit directory guards were added; negative fixtures verify that neither
phase runs from a missing initial directory, and execution stops if the directory
moves after the hook. The same scanner bytes/options found no newly added records
on rerun. No rule suppression or threshold changes were used.

The retained `ubs-final-report-approved.AAeJdVPLu6/` and
`ubs-report-approved.3tT9xgpDfi/` directories contain raw reports, source/HEAD
triage, input hashes, commands, UTC times, and the cleanup authorization record.
AST/ShellCheck tools were unavailable; Cargo/audit-related UBS phases were
explicitly not evaluated, and TOML is unsupported. Compiler/lint/test evidence
above is separate from UBS coverage.

## Implemented feature and boundaries

The requested capability is opt-in placement of remote execution caches and
scratch on a selected worker volume, plus persistent remote environment defaults
for package proxies such as Nexus. Preserve native RCH workspace ownership,
source synchronization, Cargo target pools, dependency preparation, watchdogs,
and durable completion receipts. Do not replace this architecture with a generic
remote-test wrapper.

- `[execution.storage]`: root, independent cache/tmp/HOME overrides, retention,
  environment tmp mode, and Linux private-mount tmp mode.
- Unique owned scratch directories and inherited leases. Cleanup only reaps
  eligible owned, unlocked job directories; active descendants retain storage.
- `[environment.remote]`: literal defaults, allowlisted controller override,
  internally managed path precedence, validation, and redacted inspection.
  Dependency preparation receives the configured environment too.
- Placement is not a security sandbox. Environment mode cannot redirect
  applications that hardcode `/tmp`; private-mount mode requires privileges and
  refuses without them. Windows managed storage is explicitly unsupported.
- Source mirrors, source transfer staging, standalone `rch-wkr execute`, canary
  self-tests, RABS, and controller storage are outside this new setting. Cargo
  target storage retains its existing pooled-target configuration. Cache
  credentials/configuration must be provisioned on the chosen worker paths.

| File | Responsibility |
| --- | --- |
| `rch-common/src/execution_storage.rs` | Storage configuration, validation, package cache mapping, scratch wrapper/leases/cleanup, tests |
| `rch-common/src/types.rs` | Shared configuration types |
| `rch/src/config.rs` | Config loading, remote defaults, validation |
| `rch/src/commands/config.rs` | Get/set/show handling and redaction |
| `rch/src/transfer.rs` | Environment composition, dependency preparation, managed execution integration and tests |
| `rch/src/hook.rs`, `rch/src/hook/transfer_orchestration.rs` | Thread configured storage/environment through existing execution |
| `docs/guides/configuration.md` | Operator configuration, precedence, scope, requirements and limitations |

Commit history, oldest first:

| Commit | Work |
| --- | --- |
| `22dd0a17` | Core feature; 13 files, +1203/-33 |
| `3d4e890f` | Synchronize guarded bundled skill manifest hashes |
| `aa095002` | Restore safe operating guidance and Git CLI dependency transport |
| `ad761da6` | Fix all-features SSH fixture; cross-platform CI runner/toolchain transport adjustments |
| `b75c2ff1` | Windows journal path recovery and regression tests; native PowerShell exit propagation; lint/docs fixes; share initial E2E build per OS; portable scenario discovery/timing |
| `922ee67c` | Repair non-Unix hook interface drift with shared config/topology policies and explicit unsupported operations |
| `15361903` | Further rustdoc/import formatting repairs; bound CI/RABS compiler concurrency at two; RABS cache |

Windows recovery retains exact Unix path bytes, supports UTF-8 paths elsewhere,
and leaves undecodable journal paths dirty without touching a lossy alias. Do
not replace this with unsafe conversion or silently lossy filesystem access.

## Incoming GitHub CI evidence (historical code `15361903`)

These runs finished on 2026-10-08, before the local repairs above. The incoming
handoff was documentation only. Neither it nor these old results establish CI
success for the local continuation.

| Workflow | Final result and evidence |
| --- | --- |
| [Main CI 37726615188](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37726615188) | Failed. Format, workflow lint, manifest guards, all-target/all-feature check, Clippy with warnings denied, security and benchmark jobs passed. Docs, tests and coverage have failures detailed below. Main release job skipped. |
| [Test Release 37726615164](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37726615164) | All six build/version-smoke/package/upload jobs passed: Linux x86_64, Linux musl, Linux ARM64, macOS Intel, macOS ARM64, Windows x86_64. Windows provisional-recovery test step passed. Aggregate artifact verification failed on Unix checksum paths. |
| [E2E 37726615177](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37726615177) | Ubuntu: 25 pass, 16 fail, 0 skip. macOS canceled while running scenarios. Aggregate report generation passed; this is not a test pass. |
| [RABS Binaries 37726615200](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37726615200) | Runner shutdown / exit 143 during release compilation; size/doctor/overhead gates never ran. Cause remains unestablished. |

### 1. Rustdoc: remaining unescaped argv index

Job `113149055599` reports an unresolved link to `0` at
`rabsd/src/edge/shadow.rs:33` in:

```rust
/// Full argv after the wrapper (argv[0] = real tool path).
```

The expression needs inline-code markup. This repair was deliberately left for
the next session after the stop request.

### 2. Nested-runtime policy tests and coverage

`rabs-asupersync/tests/nested_runtime_prohibition.rs` runs three tests; one passed
and two failed in Linux x64 (`113149055657`), macOS ARM (`113149055616`), and
coverage (`113149055527`):

- `allowlist_entries_point_at_live_pattern_sites`, line 171: stale allowlist
  entry `rabs-wkr/src/main.rs`; it no longer contains a runtime entry.
- `no_rabs_crate_contains_nested_runtime_patterns`, line 122: detector finds
  `.block_on(` / `block_on(async` in the following files in the x64/ARM Mac logs.

```text
rabs-asupersync/src/worker_transport.rs
rabsd/src/prepared_jobs.rs
rabsd/src/worker_exec.rs
rabsd/src/coord/secure_worker_delivery.rs
rabsd/src/coord/secure_worker_delivery/interrupt.rs
rabsd/src/edge/server.rs
rabsd/src/edge/server/rustc_outputs.rs
rabsd/src/prepared_jobs/wait.rs
rabsd/src/prepared_jobs/follow.rs
rabs-wkr/src/reconnect.rs
rabs-wkr/src/input_deadline_session_tests.rs
```

Investigate ownership and whether each occurrence is production, a legitimate
runtime entry, or test content. Preserve the single-owned-runtime invariant;
do not blindly expand the allowlist, skip tests, or weaken the detector.

Coverage successfully prebuilt binaries, then failed in these same tests during
LCOV generation (exit 101). Linux ARM job `113149055633` is marked canceled by
GitHub, but its log also records these two assertion failures and exit 101.
macOS Intel job `113149055557` was canceled during compilation. Neither canceled
job is a pass. Full workspace success and feature-test coverage are not proved.

### 3. Release checksum paths

Verification job `113159811642` changes into `artifacts` and runs
`sha256sum -c *.sha256`. The five Unix checksum files refer to `dist/<archive>`,
but the downloaded archives are at `artifacts/<archive>`. They fail with missing
files, not a reported hash mismatch. The Windows ZIP checksum passes.

Inspect `.github/workflows/test-release.yml`, Unix packaging near line 138 and
verification near line 176. Generate checksums using archive basenames from
inside `dist`, or preserve the expected layout when downloading. Keep checksum
verification enforced.

### 4. E2E failures and one masked failure

Standalone Ubuntu E2E job `113146776489` failed these 16 scripts:

```text
e2e_api_envelope.sh          e2e_api_error_codes.sh
e2e_bd-155i.sh               e2e_bd-c7xr.sh
e2e_bd-zked.sh               e2e_bd-zp4j.sh
e2e_bun_prepare.sh           e2e_config_inspect.sh
e2e_error_experience.sh      e2e_force_resync.sh
e2e_install_test.sh          e2e_output_validation.sh
e2e_pipeline.sh              e2e_project_sync.sh
e2e_real_fleet_smoke.sh       e2e_test.sh
```

Their individual causes have not been triaged. Download `e2e-ubuntu-latest`
from run `37726615177`; the shared runner preserves per-script `.build.log`,
`.jsonl` and `.status` files. Artifact `e2e-summary` contains the aggregate report.
macOS job `113146776296` was canceled during `e2e_bd-1yt6.sh`; it has partial
artifacts. Do not label all these failures infrastructure failures.

Separately, main CI E2E job `113151388704` recorded **157 pass, 7 fail** in
`rch --test true_e2e`. The seven fail-open tests could not start
`target/debug/rchd` because that file was absent. The workflow first builds
release binaries but this harness requested a debug daemon. The cargo E2E
command currently ends in `|| true`, masking its nonzero status; the later
shell E2E step was canceled. Investigate binary setup and make the test result
an enforced gate rather than treating the green step as evidence of success.

### 5. RABS runner shutdown

Job `113146119382` logged "The runner has received a shutdown signal" at
2026-10-08T04:23:50Z, then exit 143 during compilation. Two compiler jobs did
not resolve it. An earlier retry behaved similarly. There is no established
OOM diagnosis and no evidence from the unrun release gates. Preserve logs and
investigate the host/cancellation event separately from assertion failures.

## Suggested next local session

1. Read the instructions and inspect the branch. Keep all existing work.
2. Fix the small rustdoc and checksum-path defects, then address the runtime
   policy failures with source-level ownership analysis.
3. Run targeted feature and policy checks; resolve the watchdog discrepancy on
   a host with a consistent PID/proc view. Inspect E2E artifacts before reruns.
4. Repair E2E binary preparation and failure propagation, then reproduce failing
   scenarios individually. Broaden validation once concrete failures are fixed.
5. Establish privileged Linux mount success, live SSH, and Nexus behavior in the
   user's environment before claiming those capabilities verified. Keep draft
   status until the necessary checks pass at a named revision.

Toolchain drift matters: `rust-toolchain.toml` pins `nightly-2026-08-31`, while
main CI and Test Release explicitly set `RUSTUP_TOOLCHAIN=nightly-2026-06-06`.
The standalone E2E workflow installs June nightly without that override; RABS
relies on the checkout's toolchain. Record the actual toolchain for each result.
Do not silently change pins just to obtain a pass.

Examples for the next local session; none were executed for this handoff:

```bash
# Reproduce the main CI policy and docs failures using its explicit toolchain.
CARGO_NET_GIT_FETCH_WITH_CLI=true CARGO_BUILD_JOBS=2 cargo +nightly-2026-06-06 test --locked -p rabs-asupersync --test nested_runtime_prohibition -- --nocapture
CARGO_NET_GIT_FETCH_WITH_CLI=true CARGO_BUILD_JOBS=2 RUSTDOCFLAGS='-D warnings' cargo +nightly-2026-06-06 doc --locked --no-deps --all-features --workspace

# Targeted feature checks on the repository-pinned toolchain.
CARGO_NET_GIT_FETCH_WITH_CLI=true CARGO_BUILD_JOBS=2 cargo +nightly-2026-08-31 test --locked -p rch-common execution_storage -- --nocapture
CARGO_NET_GIT_FETCH_WITH_CLI=true CARGO_BUILD_JOBS=2 cargo +nightly-2026-08-31 test --locked -p rch managed_ -- --nocapture

# Read recorded evidence before starting another full E2E suite.
gh run view 37726615188 --repo tsunheimat/remote_compilation_helper --job 113149055657 --log
gh run download 37726615177 --repo tsunheimat/remote_compilation_helper --name e2e-ubuntu-latest --dir target/handoff-e2e-ubuntu
gh run download 37726615177 --repo tsunheimat/remote_compilation_helper --name e2e-summary --dir target/handoff-e2e-summary
```

`scripts/run_all_e2e.sh --filter=e2e_config_inspect.sh
--out=target/e2e-local` can select a single scenario after its prerequisites are
prepared. The CI setup builds the release workspace and places `target/release`
on `PATH`. The separate `true_e2e` harness also needs its expected debug daemon;
inspect the harness before deciding the build/profile fix. Respect local RCH
offloading instructions and the user's chosen validation location.

## Historical validation only

Before the user switched hosted validation to GitHub CI, code commit
`22dd0a176730d761378f557f807a4e1e332a5fc8` (tree
`c57626c95b36d6738e3db82f2cb97de2e3e06376`) was checked locally using August
nightly, one Cargo job, `RUSTFLAGS="-Z threads=1"`, disabled incremental
compilation, and disabled debug info:

| Check | Historical result |
| --- | --- |
| Format; workspace/all-target check and Clippy, default features | Passed |
| RCH `managed_` filter | 34 passed, 1 failed |
| `rch-common` `execution_storage` filter | 6 passed |
| `cargo_home_boundary`, `add_cargo_isolation`, `build_remote_command` filters | 4/4, 5/5, 10/10; overlap other filtered checks |

`managed_storage_watchdog_timeout_releases_tmp_without_changing_status` expected
137 but got 125 with `IDENTITY_UNAVAILABLE`; the unchanged full-stderr watchdog
test failed the same way. Keep the assertions and diagnose process identity
instead of accepting 125 as success. The private-mount test exercised refusal
because `unshare` was not permitted; successful privileged mounting remains
unverified. No live SSH/Nexus success was established. These historical results
do not replace full validation of the latest code revision.
