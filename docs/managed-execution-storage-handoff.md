# Managed execution storage: local Codex handoff

Snapshot: 2026-10-09 UTC, local development continuation. Consumer: the draft PR
reviewer and the user's next local session. The user requested continued
development and said they will deploy later. Keep PR #2 draft. Deployment and
live worker/Nexus acceptance remain with the user. Windows-specific repair and
validation are deferred following the user's request to consider skipping
Windows support; existing Windows jobs have not been disabled.

## Checkout and validation state

- Repository: <https://github.com/tsunheimat/remote_compilation_helper>
- Branch: `feat/managed-execution-storage`
- Draft PR: <https://github.com/tsunheimat/remote_compilation_helper/pull/2>
- Integrated base: `main`, `67b5d4b9b8aa179d5d9d74bc0d8d01a05e209210`
- Incoming feature head: `7f68c5dc12caffae49d0860bf3c19e72e7c58a65`
- Incoming code commit: `1536190314d72b55aef3b1cd80245b3c93673bb1`
- Incoming code tree: `50665b2330f0b8c11ba2056cc0a2661f7012e69b`
- Published development checkpoint: `f207c5bff554361111c56ddc51e7cc260302cb8a`
- Published main integration: `a67ece95362fe06da6b3cbc8aacf2547a147330c`
- Repair baseline: `4e3d3e3a4f61b2d045019a586cc0c5a918678491`
- Published runtime repair batch: `45bfad2716b73dde524dc02bc376731a0f1fbf1a`
- Published CI repair baseline: `e061220552d3d5465107b6f22e1e07f61469cf30`
- Published profiling/toolchain repairs: `1c56ce1d9c5565acc295d622fa1e1d0933e10594`
- Integration tree before the fixture repairs below: `5e6708c98abcf5c1c6fe3ca3a375a72d64b31035`
- Integration worktree: `/mnt/vibe-coding-share/develop/remote_compilation_helper-storage-integration`
- Active development worktree: `/mnt/vibe-coding-share/develop/remote_compilation_helper-managed-storage`

The primary `main` checkout and the published checkpoint worktree were preserved.
The integration brings in 122 main-branch commits, including native source and
execution recovery changes. Their process identity, source ownership, pooled
targets, and completion-receipt paths remain in place. The old full-workspace
build was stopped with exit 143 during compilation, before any tests ran, so
remaining validation could target the integrated source. It is not a full-suite
pass. The incoming CI failures and checkpoint results below remain historical.

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

## Current repair batch after `1c56ce1d`

All six workflows at `1c56ce1d` are terminal. The current batch addresses the
remaining wrapper-fixture environment leak, development hashing cost, and
portable hook checks. The PR remains draft; full current CI, coverage, native
macOS confirmation, and operator deployment acceptance remain open.

- C009's recorded and live flags matched in Linux CI, but the live environment
  contained the workflow's `CARGO_HTTP_TIMEOUT` and `CARGO_NET_RETRY`. The fixture
  now removes those two caller inputs before invoking stock Cargo. Raw capture
  still observes both names if Cargo emits them; two explicit negatives cover
  that distinction. No golden or parser filter changed. Before the repair,
  clean launches passed all three channels, while the actual CI settings failed.
  After the repair, clean, CI-network, and CI-plus-coverage launches each pass
  both tests across stable, Beta, and nightly; the planted unknown Cargo key
  still fails every channel.
- Profiling a real full-toolchain worker case attributed 72.35% of sampled CPU
  events to SHA-256 code and its instantiated intrinsics. An ordinary-debug
  baseline/candidate/candidate/baseline comparison retained the complete 1.39 GB
  installed August toolchain and the original 60-second control. Baselines
  timed out at 61.77 and 62.23 seconds; candidates completed the unchanged real
  compiler/output assertions at 25.36 and 22.23 seconds. This is maintenance
  evidence, not an external-incumbent performance win. The subsequent five-target
  diagnostic cohort passed all 16 cases, without skips or filtered tests.
- The adopted Cargo development profile optimizes only `sha2:0.11.0`. The
  diagnostic unit graphs differed only in that dependency's optimization level
  among 306 units; debug information, assertions, overflow checks, application
  profiles, and all source/toolchain hash barriers remained unchanged. Release
  profiles are unchanged. Ordinary Cargo without a CLI profile override produces
  that same unit graph, and all 16 cases pass again with no skips or filters.
- The macOS shell suite recorded a hook non-interference failure before its
  later cancellation. Its inner log was outside the uploaded artifact, so the
  exact native diagnostic is unavailable. Compatibility controls reproduce two
  concrete defects: BSD-style date output breaks arithmetic, and unsupported
  `grep -P` silently admits ANSI output. The repair uses fixed-string matching
  that also rejects grep errors, and one monotonic timer around each complete
  hook process. The 50 samples and 10 ms mean limit remain enforced; individual
  samples are retained and printed. Native macOS acceptance still needs CI.
- The outer E2E runner places inner suite logs in its artifact directory as they
  are written. A controlled failure retained its nonempty inner log before the
  outer runner exited and produced a failed status. Cancellation still leaves
  unfinished cases missing; it never manufactures passes or skips.

The hook-gate admission comparison is explicit: a valid hook with unsupported
date nanoseconds changes from failure to pass; ANSI output under a grep without
`-P`, and a grep I/O error, change from false passes to failures. A 20 ms injected
delay fails both timing implementations, and the real Linux hook passes both.
The new clock measures process launch, input delivery, and completion without
including separate clock-process launches. These are fixture controls and a
Linux native check, not a macOS result or a claimed product speedup.

The required locked workspace/all-target check, Clippy with warnings denied,
and formatting all passed. The only source change during those checks was
making the shell timing sample file unique per invocation; the final shell
source passed the portability/negative controls and Bash syntax checking.
No Rust source or Cargo profile changed during this validation cohort.
The final four-file UBS scan returned exit zero, zero critical findings, and no
added-line findings. Its 13 warnings and 35 informational records are retained.
Current input hashes cover 174 files; the 26 earlier reviewed critical records
remain, so this is not an all-scope scanner pass. No scanner Cargo/AST/ShellCheck
phase is claimed.

| Terminal CI at `1c56ce1d` | Observed result |
| --- | --- |
| [CI 37992072427](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37992072427) | Check, strict Clippy, docs, format, workflow/manifest/security, and benchmark jobs passed. Linux x64 failed C009 on the two network environment keys. Linux ARM and coverage each completed 200 cases in 12 harnesses before cancellation during `rabs-cas`; both macOS jobs were still compiling. GitHub annotations identify the existing 30-minute job limits. No full workspace or 65% coverage verdict was produced. |
| Core CI E2E | 164 reported native passes, with 92 archived capability-skip records. Shell execution completed three passes and one skip, then hit the 30-minute job limit during path-fixture compilation. |
| [E2E 37992072457](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37992072457) | Linux completed all 41 discovered scripts. macOS completed 39, recorded the inner hook failure above, and hit its existing 60-minute limit during the nested reliability suite. Its outer `e2e_test.sh` result and final UI script are missing. The aggregate correctly reports 80/82 completed; its green reporting job is not suite acceptance. |
| [Test Release 37992072428](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37992072428) | All five Unix build/version/package/upload jobs passed. Each ZIP digest, embedded archive checksum, exact three executables, and executable modes were verified. Windows failed the deferred non-Unix helper import; aggregate verification was skipped. |
| [RABS 37992072425](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37992072425) | Both jobs passed. Accepted-hit delivery: 34 unit and eight integration cases. Release gates: replay seven, real compiler dependencies two, worktrees one, doctor two, overhead one. Wrapper size was 443,456 bytes; enforced p95 was 5,593 microseconds under the unchanged 10 ms limit. Build observations retained at least 4,203,968 KiB available memory and reported zero OOM kills; this does not diagnose earlier exit-143 runs. |
| [Rsync 37992072434](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37992072434) | Both jobs passed, including 64 native transfer cases. All 1,390 archived Git blobs and modes match the named head, binding the CI merge source to this revision. |

Dependabot automerge was skipped. Evidence under the retained validation base:

- `rch-ci-head1c-monitor-3ezt2vx9/`: raw logs, timeout annotations, exact collections, source binding, and verified artifacts.
- `rch-contract-ci-network-pfbeh4o4/`: three-channel clean/CI/coverage/unknown-input controls.
- `rch-large-toolchain-profile-get_9rna/`: raw CPU profile, binary identities, unit graphs, retained baselines, and all 16 diagnostic cases.
- `rch-hook-portability-Ga23SHJB/`: original/final gate controls, actual Linux binary checks, raw timings, and outer-runner failure propagation.
- Active-worktree `target/managed-storage-validation-20261009/ubs-final-timing-report-approved.pefw5dm2/`: final scanner report, command, authorization, source hashes, and retained earlier critical records.

## Profiling and toolchain repairs at `1c56ce1d`

The six workflows at `e0612205` were terminal before this batch repaired the
profiling, nested-build, shell-status, and test-preparation boundaries. At that
checkpoint, fresh CI, six large-toolchain worker cases, and operator deployment
acceptance remained open. The section above supersedes that validation state.

- An instrumented compiler guard wrote its default profile into an immutable
  package directory. The native June reproduction captured the actual files
  and identified both wrapper and guard processes as writers. The guard now
  retains the wrapper's profiling destination separately from the admitted
  compiler environment. The compiler receives a profiling variable only when
  explicitly admitted. The dependency fixtures preserve the wrapper's profile
  destination outside their inputs. Input mutation checks remain enforced.
  Running the strengthened test with the retained old wrapper fails on the
  leaked file; its explicit-profile positive control still passes.
- Five test helpers now remove their parent test package's Cargo metadata
  before asking real Cargo to build `rabsd`. A five-invocation Cargo control
  established that the inherited metadata invalidated Ring's fingerprint while
  producing identical output bytes. After the repair, all five helpers reuse
  the prepared daemon without recompiling Ring, Rustls, Asupersync, or RABS.
  Source freshness checks, compiler flags, and configured storage remain intact.
- The strict stable/Beta/nightly wrapper contract excludes four known
  cargo-llvm-cov launcher variables. Raw-capture negatives still reject an
  unknown Cargo variable and preserve observed flag, environment, and framing
  drift. The signal fixture explicitly waits for its killed child, so Bash's
  exec optimization cannot replace the shell's required numeric status 137.
- Unit-only E2E filters select their actual library or binary targets. Native
  test inventories and Cargo unit graphs establish unchanged selected cases
  and dependency features. Filters with integration matches remain broad.
  `e2e_bd-1yt6.sh` honors the configured Cargo target directory instead of
  forcing another cold build. Every existing assertion and scenario remains.
- Core shell E2E and the coverage-summary pipeline propagate failures through
  `tee`. Coverage executes the full workspace once, then generates the other
  formats and checks the unchanged 65% line threshold from that execution.
  RABS release compilation uses one Cargo job after recorded severe memory
  pressure with two; the observations did not establish a kernel OOM cause.

CI, E2E, release, and test-release now explicitly use the existing repository
pin, `nightly-2026-08-31`. The June workflow pin was introduced in `69a7ac24` to
match the fleet; `282e13f5` later moved the fleet/repository to August without
updating those workflows. This is a deliberate compiler-coverage change,
announced during development. It does not establish June compatibility. Native
June coverage still fails the live multi-worktree test after the profile repair:
the classic shared dependency directory changes its candidate set during
compilation. A retained explicit-layout experiment also failed key reuse and
was reverted. Neither assertion nor dependency fence was relaxed. The same
unchanged multi-worktree assertions pass with August's default Cargo layout.
All affected platforms require fresh CI under the aligned pin.

Local validation uses the repository's August toolchain and normal debug
profiles except where instrumentation is explicitly identified. Cohorts overlap
and must not be summed into a full-workspace pass:

| Current batch validation | Observed result |
| --- | --- |
| Workspace/all-target/all-feature locked check, strict Clippy, formatting | Passed after the final Rust edits. |
| Entire wrapper package after the guard repair | 87 passed across 16 targets; no failures, ignores, or filtered cases. |
| Five changed daemon-build helper targets after metadata repair | All 16 cases passed; no dependency/daemon recompilation inside their setup. |
| Instrumented supervisor on August | All nine cases passed, with profiles outside immutable inputs and explicit admitted-profile preservation. This is not a full coverage report. |
| Instrumented June dependency and supervisor targets | Two dependency cases and nine supervisor cases passed; the separate June multi-worktree failure remains recorded. |
| Replay fixture | All 40 existing cases passed with actual Dash and actual Bash. Native macOS confirmation remains required. |
| Strict wrapper contract | Both tests passed on all three channels with known coverage markers; the planted unknown variable still fails. |
| Four complete changed E2E scripts | Path fixtures: 45 passes including nightly topology; repo-updater: 29; reliability: 676 outcomes across 25 smoke families; cancellation: seven. No live worker acceptance is implied. |
| Changed nightly Cargo selectors through the existing E2E helper | Topology 20, repo-updater 29, and process-triage nine passed. The full nightly script and its unchanged Criterion benchmark were not run. |

UBS input hashes now cover 172 distinct current files. The two latest raw scans
returned zero critical findings; the five-helper delta has no added-line
findings. The earlier 26 reviewed critical records remain in the combined
report, so this is not an all-scope scanner pass. The official scanner is
unchanged and only its own newly generated scratch cleanup was authorized.
The retained reports record that authorization, exact commands, timestamps,
source integrity, and the unrun Cargo/AST scanner phases.

Evidence under the retained validation base below:

- `rch-ci-heade061-monitor-f2WKfYaN/`: terminal CI metadata, logs, verified source and artifact bindings.
- `rch-coverage-native-reproduction-g0c1tey3/report/`: June causal reproduction, old-wrapper control, repair trials, full wrapper run, final helper checks, August instrumentation, and four complete scripts.
- `rch-nested-cargo-metadata-7s933i9s/`: five actual Cargo invocations, dirty-fingerprint reason, and identical Ring output hashes.
- `rch-shell-target-collection-nywa8rri/`: native case inventories and unchanged selected Cargo dependency graphs.
- `rch-replay-fixture-validation-gflro1vr/report/` and `rch-coverage-contract-validation-hwry8mqk/report/`: complete shell and strict-channel controls.
- `rch-workflow-pin-alignment-_md5gvuf/`: parsed workflow review; only compiler selection, four reporting run fields, and RABS build concurrency changed.

The latest UBS reports are in the active worktree's
`target/managed-storage-validation-20261009/`, under
`ubs-coverage-e2e-report-approved.1pfvqm85/` and
`ubs-cargo-helper-report-approved.dqkt243l/`.

| Terminal CI at `e0612205` | Observed result |
| --- | --- |
| [CI 37970947675](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37970947675) | Check, strict Clippy, docs, format, workflow/manifest/security and benchmark jobs passed. macOS Intel reached the real shell-status assertion failure. Linux x64 reached nested daemon compilation before its 30-minute cancellation; the ARM jobs were incomplete. Coverage stopped at the live multi-worktree assertion; no LCOV or 65% verdict was produced. |
| Core CI E2E | 164 reported harness passes, with 92 archived capability-skip records. Shell execution completed three passes and one skip, then timed out during path-fixture compilation; eight later suites did not run. |
| [E2E 37970947587](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37970947587) | Linux completed all 41 discovered suites. macOS completed five and timed out compiling the sixth. The report records 46/82 completed and 36 missing outcomes; inner capability skips remain visible. |
| [Test Release 37970947695](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37970947695) | All five Unix build/version/package/upload jobs passed; their archives and checksums were independently verified. Windows failed the deferred non-Unix helper import; aggregate verification was skipped. |
| [RABS 37970947586](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37970947586) | Accepted-hit protocol passed 34 unit and eight integration cases. Release compilation exited 143 after severe observed RAM/swap pressure; OOM counters remained zero. Later release gates did not run. |
| [Rsync 37970947605](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37970947605) | Both jobs passed, including 64 native transfer cases; source/artifact bindings were verified. |

Dependabot automerge was skipped. These are results for the preceding head,
not acceptance of the current compiler selection or repair batch.

## Historical CI follow-up after `45bfad27`

The complete set of six workflow runs at `45bfad27` is terminal. The current
follow-up addresses its dependency and filesystem assertion failures and records incomplete
validation without extending deadlines or dropping test collections.

- The dependency budget correctly rejected the TLS adapter's fourth direct
  dependency. Its documented policy permits reviewed additions. The adapter's
  budget is now four, naming Asupersync, protocol, delivery state, and Rustls.
  The review found 732 locked packages before and after the TLS change, no new
  versions or transitive packages, and the identical Rustls provider unit in
  the native failing/passing TLS runs. The pinned Asupersync API exposes no
  provider initializer. The counter and all other crate limits are unchanged:
  four is newly admitted for this adapter; five remains rejected. This is a
  reviewed dependency addition, not a claim that the old gate was defective.
- macOS ARM rejected both raw filename renames with native `EILSEQ` (OS error
  92). The two existing tests retain unconditional byte-preserving preflight
  and decoding, and positive installation/recovery on supporting filesystems.
  Only that specific macOS error, after a valid-name control on the same volume,
  admits the refusal assertions. Production must refuse without changing the
  lossy-name alias, CAS identity, or journal rows. Inode, mode, link count,
  length, and modification time are also checked. Other I/O errors still fail.
  Recovery now reopens a file-backed journal before and after both sweeps,
  declaring the same known object domain as production startup. A filesystem
  refusal does not fabricate an installed raw-path row or prove dirty recovery.
- Both E2E workflows build the three application binaries their callers use.
  Their full test/script collections, debug prerequisites, profiles, compiler
  pins, and 30/60-minute budgets remain unchanged. The Layer-0 script still
  builds its own debug `layer0_render` helper. The prior unqualified builds
  compiled all 16 workspace crates; the macOS release prerequisite alone took
  44m45s. New-head execution is required before claiming this resolves a timeout.
- Workspace test output is now retained through a status-preserving `tee` and
  offered for upload on failure or cancellation. The E2E summary compares
  outcomes with the same revision's discovered inventory and both OS labels.
  Replaying the real artifacts now reports 43/82 completed and 39 missing;
  missing results have no invented exit code or duration. The summary remains
  informational; the native run jobs still enforce test results.
- RABS release compilation emits bounded resource observations while its
  unchanged Cargo command runs. Its own monitor is reaped while preserving the
  build status. This prepares evidence for another runner shutdown; it does not
  establish an OOM cause or a release-gate pass.

The three dependency-budget tests and the workspace/all-target/all-feature
locked check, strict Clippy, and formatting pass. The expanded default-feature
CAS run completed with 309 passes, one failure, and one ignore: the new reopen
fixture initially omitted its required domain declaration. After correcting
that setup, both non-UTF8 tests passed (309 filtered) and all required checks
passed again. The Linux positive paths are exercised; the new macOS refusal
branch still needs native CI. No production Rust changed in this follow-up.

Validation binds all 1,390 tracked source hashes/modes and the unchanged host
profile. Later changes to E2E workflow prerequisites, summary Python, and this
document have separate YAML/Bash/AST and actual-artifact reporting checks.
The summary was executed against the verified Linux41/Mac2 artifacts, the
complete Linux inventory, and an empty collection: respectively 39, zero, and
82 missing outcomes. These are report checks, not fabricated native tests.

Evidence under the retained validation base below:

- `rch-dependency-budget-validation-n83awndr/report/`: dependency review and three native gate tests.
- `rch-macos-path-contract-final-to93ngnh/report/`: full default-feature CAS run, including the retained fixture failure.
- `rch-macos-path-reopen-validation-_rncazd_/report/`: corrected native tests, compiler checks, source proof, and binary hash.
- `rch-ci-resource-observer-6h62qkw0/`: actual resource sampling and native child statuses 0/7/143 preserved by cleanup.
- `rch-ci-log-capture-review-rw56bunr/`: real shell-pipeline status/output checks, not Cargo executions.
- `rch-e2e-completeness-review-_ezhk10x/`: current summary over verified native status artifacts.
- `rch-ci-head45-monitor-0kN4dR0M/`: all 28 executed-job logs, artifact/source bindings, and release prerequisite caller reviews.

| Terminal CI at `45bfad27` | Observed result |
| --- | --- |
| [CI 37961912877](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37961912877) | Check, strict Clippy, docs, format, manifest/workflow/security and benchmark jobs passed. Linux x64 stopped at the adapter dependency budget before worker/daemon suites. macOS ARM had two real `EILSEQ` assertion failures before its 30-minute cancellation. Linux ARM and macOS Intel were still compiling at 30 minutes. Coverage timed out during instrumented compilation; no tests, LCOV, or threshold verdict resulted. |
| Core CI E2E | 164 reported Cargo test passes; live-worker acceptance is not implied. The shell phase completed three passes and one explicit skip, then timed out during path-dependency fixture setup. The remaining shell collection is incomplete. |
| [E2E 37961912912](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37961912912) | Linux completed all 41 discovered suites with matching successful archived statuses. macOS completed only API envelope and API error codes, then reached 60 minutes during the third suite's debug prerequisite build. Its other 39 outcomes are missing. The old green summary's 43 passes represent Linux41 plus Mac2, not a complete two-platform run. |
| [Test Release 37961912948](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37961912948) | All five Unix build/version/package/upload jobs passed. Their archives, checksums, executable members and hashes were independently verified. Windows failed the unresolved non-Unix helper import; repair remains deferred. The aggregate package-verification job was skipped. |
| [RABS 37961913026](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37961913026) | Accepted-hit protocol passed 34 unit and eight integration cases plus lint. Release compilation received another runner shutdown and exited 143 without a Rust compiler diagnostic. Its later gates did not run; the shutdown cause remains unknown. |
| [Rsync 37961912900](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37961912900) | Both jobs passed, including 64 selected transfer tests. The source archive matched the exact PR merge and all 1,390 source blobs/modes. |

The separate wrapper-contract workflow is `disabled_fork`, not pending approval
or missing from pagination; it was not enabled or dispatched. Fresh CI belongs
to the next published head. Keep the PR draft while the remaining gates,
including the six large-toolchain worker cases below, remain unresolved.

## Published runtime repair batch (`45bfad27`)

The broader workspace run exposed production and fixture defects beyond the
earlier checkpoint. This published batch repaired these boundaries:

- RABS creates private staging owners with Unix mode `0700`, matching the
  existing ownership checks. Fixtures now establish the same valid starting
  state before testing mutation, refusal, and recovery.
- Parsed JSON request, lease, and completion identities recursively sort object
  keys before hashing. Their identity no longer depends on whether Cargo enables
  `serde_json/preserve_order`. Arrays and scalar values remain bound; exact wire
  and file-byte identities keep their existing representation. Three literal
  digest-vector tests cover the native identity owners.
- RCH update and recovery guards explicitly unlock when their logical ownership
  ends. An unrelated child between fork and exec can no longer prolong the lock
  through an inherited descriptor. Installed-version probing retries only an
  actual `ETXTBSY` spawn refusal within one original absolute deadline.
- Classification-cache tests exercise the exact TTL boundary through the real
  lookup and eviction path with a controlled clock observation. The public path
  still reads the monotonic clock under its lock and expires only after the TTL.
- The RABS TLS adapter initializes Rustls' provider before the pinned Asupersync
  mutual-TLS verifier consults it. An already installed application provider is
  retained. This fixes a native startup panic when both Rustls providers are
  enabled; certificate, peer-key, ALPN, and timeout checks remain enforced.
- Replay, daemon-wait, stream, recovery, and CAS fixtures now establish the
  current native protocol state. Shell scenarios use portable timing and
  timeout helpers, verify current installer/proxy/source-sync behavior, and
  require actual positive Cargo test counts.
- CI test and coverage jobs install the three toolchains required by the strict
  wrapper-contract matrix. Their June compiler pin and existing job budgets
  remain in force. The Beta fixture changes only the independently observed
  contract shape described below.

Local execution used August nightly, normal debug profiles, normal libtest
parallelism, and a single shared Cargo build lane. Fresh ext4 images on the
retained storage volume supplied private `/tmp` namespaces with a consistent
PID view. The ordinary test process ran as UID/GID 1000. Each phase binds 1,390
source hashes and modes before and after execution; later fixture/documentation
edits are identified separately from the runtime builds.

| Validation phase | Actual result and limits |
| --- | --- |
| Full workspace before the final residual repairs | 225 top-level test targets: 12,091 reported passes, 16 failures, 31 ignores, zero filtered tests. Eight named capability-dependent early returns are recorded separately. The intentional failing crash-child subprocess is not counted as another top-level target. |
| Workspace/all-target/all-feature check, Clippy with `-D warnings`, and formatting | Passed after the TLS runtime repair with the locked dependency graph. Before publishing `45bfad27`, only the Beta JSON fixture and documentation changed afterward. |
| Entire RCH component after its residual repairs | 3,640 reported passes across 17 target summaries, zero failures/ignores/filtered tests. Two signing cases explicitly returned early for unavailable opt-in/cosign prerequisites. |
| Native RABS replay, wait, worker TLS, interruption, and resume targets | 44 passed, zero failures/ignores/filtered tests. The passing and failing daemon builds link the identical Rustls unit with both providers enabled. |
| Feature profile and runtime policy | 4 + 6 passed. Retained unchanged test binaries read the current manifests/source; the policy scanned 352 Rust files. |
| Controlled native lock and executable-busy schedules | All seven schedules passed, totaling 18 selected test executions. Actual inherited descriptors, unlock/reacquire overlap, and two real `ETXTBSY` returns were observed; syscall results were not injected. |
| Nine complete repaired shell scripts | All nine returned success. The three real Cargo test selections passed 30 + 42 + 1 cases; self-healing exercised 9/9 cases. Installer passed 44 assertions with one explicit macOS launchd capability skip. |
| Shell output and command audit | All 32 JSONL/NDJSON files parse: 569 records including retained duplicates. All 21 Cargo calls have complete receipts. Three optional cargo-hakari probes and one missing-lock metadata probe returned 101; none was a test invocation. |
| Strict wrapper-contract tests after the manual Beta fixture edit | Both existing tests passed unfiltered. Stable 1.98.1, Beta 1.100.0-beta.4, and nightly 1.101.0 all matched; the existing planted flag/framing/environment mutations remained enforced. The retained test harness source and binary were unchanged. |

The nine scripts are API error codes, `bd-szio`, configuration rollout,
discovery surfaces, installer, Layer-0 pack, placement controls, project sync,
and self-healing. Layer-0 ran its default three stock/three configured builds
and checked the actual compiler flags; this is not a performance-win claim.
Real Cargo legitimately rebuilt the daemon variant during the collection, so
per-script binary receipts, rather than one supposedly immutable binary, bind
those results. Existing source, host profile, and output trees were preserved;
only a previously missing empty `l0-proof` mountpoint was created and retained.

Six large-toolchain RABS worker cases were red in that full workspace run with
fully unoptimized development dependencies.
They span artifact transfer, jobserver bridging, output transfer, process
context, and worker sessions under the original capture/execution deadlines.
A separately labeled runtime check with the complete installed June toolchain
passed 13 of 16 cases but still failed three. One reached real execution and
returned timeout status 124 after producing output. These are not all pre-exec
failures, and the evidence does not establish that NFS dominates their cost.
No hashing barrier, timeout, toolchain pin, or test collection was weakened.
These failures remain the baseline for the later development-profile repair
above. Its full 16-case cohort is still not a full-workspace pass.

### Beta contract evidence and admission change

The manual Beta fixture update adds `--force-warn` and
`unused-externs-silent`, and removes `-C extra-filename` and `-L` from the
dependency-free primary/build-script profile. Its environment-name set is
unchanged. The exact two-job C009 stock/native pair executed the actual Beta
compiler for both classes, retained their normal primary-package refusals, and
produced identical artifact, dep-info, and application bytes.

A separate one-job native cache cohort exercised commit, independent
verification, three served dependency hits, and Cargo freshness. Source closure
changes, a compiler `--cfg` change, and repeated compiler errors passed their
original negative assertions. The source hashes before/after the `--cfg` change
are identical, and all three native action keys changed. Raw complete
`unused_extern` payloads, including the nonempty `unused_extern_names:["side"]`,
match the real compiler output. Observed `CLONE_THREAD` lineage binds compiler
writers; replay writers are the actual wrapper processes with matching served
receipts and no dependency compiler execution.

The original serial runner retains exit 1 because its observer rejected
whitespace in six reassembled strace write records. A separately retained,
reviewed offline parser correction accepts only that whitespace and proves the
complete byte counts, payloads, writer lineage, and receipts from unchanged
traces. Seven native behavior phases passed; the eighth observation is supported
by that separate analysis. Earlier parallel probes remain incomplete: an
observed pre-IPC fallback is consistent with the intended nonblocking breaker
lock, not proof of a timeout or a required policy repair.

The recorded admission split has 12 cases: the new exact fixture admits the
observed modern Beta shape and rejects the old Beta shape. Ten altered flag,
framing, artifact, environment, and hybrid shapes remain rejected by both.
The comparison code and stable/nightly fixtures were not relaxed or regenerated.

### Evidence locations and scanner limits

The retained validation base is
`/mnt/hdd-vibe-coding-share/remote-test-local-fallback-claude1/tmp/`:

- `rch-workspace-volume-74eX6rDB/report/`: full 225-target run and capability accounting.
- `rch-residual-validation-yv7tq8ja/report/`: entire RCH component and the initial native TLS provider failure.
- `rch-tls-provider-validation-cqiny0di/report/`: final required checks, 44 native cases, and identical Rustls dependency-unit proof.
- `rch-post-repair-native-Wwu0ZKZL/runs/`: seven native schedules and `nine.4jVfTV8K` complete script collection, including host/source preservation receipts.
- `beta-native-serial-run-NKwluaf2/`: immutable native outputs, separate corrected observation, exact fixture admission review, and isolated `--cfg` comparison.
- `rch-beta-fixture-validation-g0groc4f/report/`: strict all-channel comparison after the manual fixture update.

UBS coverage now binds 162 current Rust/Bash/Python inputs. The final recovery
and summary scan returned exit 0 with zero critical, 44 warning, and 36
informational records; its Python module reported zero findings. The expanded
filesystem tests' assertion and bounded expected-value construction warnings
were reviewed, not suppressed. Earlier raw scans remain nonzero, and 26 reviewed
critical records remain in the combined manifest. One is on the existing
installed-version `Command::new(path)` call moved into the bounded retry loop;
its trusted path derivation and literal `--version` argument are unchanged.
The other reviewed expressions are inherited. Warnings are not exhaustively
cleared, and no all-scope scanner pass is claimed. The official scanner remained
unchanged; no AST/ShellCheck/Cargo scanner phase or TOML/YAML scan is implied.
Optional Python package analyzers were disabled; no package installation ran.
Current combined hashes and bounded triage are in
`target/managed-storage-validation-20261009/ubs-reopen-summary-report-approved.2puzqs72/`.

## Published baseline CI (`4e3d3e3a`)

These terminal results predate the batch above. Fresh CI belongs to its own
published head; local selected passes do not turn these runs green.

| Workflow | Observed result |
| --- | --- |
| [CI 37926280812](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37926280812) | Check, Clippy, docs, format, workflow/manifest/security and benchmark jobs passed. Linux x64 stopped at two missing CAS domain declarations, repaired in this batch. macOS ARM reported two non-UTF8 path failures before its 30-minute limit; their exact native error remains open and direct-stderr diagnostics are added. Linux ARM, macOS Intel, coverage and the core E2E job also reached their existing limits. No LCOV report was produced. |
| [E2E 37926280750](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37926280750) | Ubuntu passed all 41 outer scripts. macOS passed 34 and failed seven; their portability/behavior repairs are in this batch. Both inner collections passed 11 cases with two explicit opt-in skips. Native macOS acceptance still requires fresh CI. |
| [Test Release 37926280696](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37926280696) | Four Unix jobs passed. macOS Intel completed packaging/upload but its final job was cancelled at one hour. All five Unix packages independently passed archive/checksum/inventory verification. Windows failed at the inherited non-Unix helper import and remains deferred; aggregate verification was skipped. |
| [RABS 37926280978](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37926280978) | Accepted-hit protocol passed 34 unit and 8 integration cases plus lint. Release compilation received a runner shutdown signal and exited 143; cause remains unknown. Size, replay, worktree, doctor and overhead gates did not run. |
| [Rsync 37926280842](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37926280842) | Both jobs passed. The source archive was independently bound to the exact PR merge and all 1,390 published files. |

The core CI `true_e2e` step separately reported 164 passes, including 92 named
early returns without live-worker prerequisites and 72 exercised cases. The
failed overall job is not a complete E2E pass. Full baseline logs, annotations,
verified artifacts, and source analysis remain in the integration worktree's
`target/managed-storage-integration-validation-20261009/ci-4e3d3e3a4f61/`.
Keep the PR draft while the remaining gates are unresolved. Deployment and live
SSH/Nexus acceptance remain with the user.

## Historical fixture checkpoint (`4e3d3e3a`)

At this earlier checkpoint, post-integration changes repaired test and logging
boundaries; executable production Rust was identical to `a67ece95`. The CAS publication fixture now
registers the known domains when reopening its store, matching coordinator boot.
Quarantine tests require the native refusal and unchanged durable snapshot when
an ordinary serving update attempts to erase quarantine. They retain the
subsequent blocked-renewal assertion against the unchanged revision.

Real-Cargo conformance now observes both supported build-directory layouts and
keys the actual environment used for relocated execution. June's original
fixture passed one case and failed two; both June and August now pass all three.
The byte-mutation, dependency-key, and unkeyed-environment refusal checks remain.
The build-script E2E fixture reads Cargo's reported `OUT_DIR`, requires exactly
one build-script event, and checks the generated file's original content.
Daemon recovery closes restart admission and checks the idle lease snapshot
before requesting shutdown; its exit, socket-removal, and restart assertions
retain their existing budgets.

Shell repairs replace seven platform-specific `grep` paths, collect and encode
four scenario logs correctly, and verify the exact absent socket in both native
status-error renderings. Stream checks still require status exit 1, successful
pipe consumers, empty stdout, and the corresponding plain stderr diagnostic.
The retry script now executes the hook rewrite, requires new durable remote
completions, and measures one baseline sync attempt and three transient-case
attempts: two failures followed by success. Its old selector ran zero tests;
the corrected selector runs four positive/negative transport cases, followed by
eight retry-configuration cases. The positive-count guard rejects the old
zero-test selector with exit 1.

The final JSONL audit also found one malformed quoted-hook event in two copies
of the same log. The shared logger now uses JSON encoding for strings and helper
data, preserves failure/skip statuses, and rejects malformed caller data.
Paired native-function probes preserve quotes, backslashes, newlines, tabs,
terminal metadata, and failure/skip reasons. Its clock uses the existing
portable helpers; the old implementation returned a nonnumeric value under a
BSD-shaped `date` probe, while the repaired helper returns numeric milliseconds.

| Earlier checkpoint local check | Result and scope |
| --- | --- |
| Workspace/all-target/all-feature check, Clippy with `-D warnings`, formatting | Passed after all five Rust fixture repairs; later changes are Bash/documentation only |
| Full CAS library, all features | 311 passed, 0 failed, 1 hardware-dependent reflink test ignored |
| Real-Cargo conformance | 3/3 on both `nightly-2026-06-06` and `nightly-2026-08-31`; Clippy and formatting also passed on both |
| Feature-gated `true_e2e` | 164 reported passes; 92 explicitly return early without live-worker setup, leaving 72 exercised cases |
| Frozen 41-script collection | All 41 outer scripts returned success after retained launcher/locale corrections; the subsequent output audit found the retry selector and shared logger defects described above |
| Aggregate after the shared logger repair | 11 child scripts passed, 0 failed, 2 explicit opt-in skips; all 33 Cargo test invocations selected positive counts |
| Repaired retry script | Passed with exact 1/3 sync-attempt counts, two new remote completion receipts, force-local admission, and 4 + 8 actual unit cases |
| Shared logger boundary probes | Passed exact data roundtrips and malformed-data rejection; failure remains exit 1 and skip remains exit 4 |
| Full workspace test execution and checkpoint CI | Pending when this checkpoint was published; later terminal results are recorded above |

The frozen collection records 173 actual Cargo calls, including 63 test
invocations: 62 selected positive counts and the old zero-count retry invocation
that the later repair replaces. The first 24 binary-only scripts have a shared
phase binary snapshot; the Cargo-backed scripts have per-case binary receipts.
Real Cargo builds legitimately changed debug binary bytes during the collection.
The CLI/daemon versions still identify 2.1.16 at `a67ece95`; each result is bound
to its source snapshot and recorded binary variant, not one supposedly unchanged
binary. The genuine release CLI is separately hashed and is not a debug alias.

Retained collection evidence is in `/var/tmp/rch-a67-full41-gmzr5k/`, especially
`full41-reconciliation.json`, `final-native-binaries.json`, and the two frozen
source manifests covering 1,390 files. Retry and logger before/after evidence is
in `/var/tmp/rch-x1ek-final-gUrUDu/report/`. Initial failures remain: the launcher
omitted the debug binary path and Git trust setting, and its stripped environment
lacked a Unicode locale. The unchanged icon binary went from 24 passed/6 failed
to 30 library passes with only `LANG=C.UTF-8` added. The full UI rerun includes
31 icon matches, 49 context matches, and 10 theme matches. Self-test reports five
Rust selections but exercises two; three real-worker selections explicitly skip.
Missing Bun, macOS launchd, real workers, and optional cargo-hakari capability
checks remain distinct from exercised checks. No operator storage was cleaned.

The final aggregate rerun is retained in
`/var/tmp/rch-aggregate-logger-final-k8hxuzac/`. It preserves all 13 child logs and
parses 49 JSONL files containing 11,593 copied records with zero malformed
records; the quoted hook message decodes to the exact native delegated command.
Its 120 actual Cargo calls include 33 successful positive-count test invocations
and 74 non-test metadata probes returning 101 on fixture lockfile/manifest cases:
73 used `--locked` and one used `--no-deps`. Those probe statuses are retained separately. Executable
and test-source hashes matched before/after; documentation is not a runtime input
to that bounded rerun.

The unchanged UBS scanner's last seven-file supplement passed with 0 critical,
8 warning, and 34 informational records; bounded added-line review found only
intentional assertions, a fixture JSON parser, and route literals misread as
division. The retry-script and shared-logger supplements then passed with zero
findings, bringing current hash-bound coverage to 100 distinct inputs. These
supplements do not erase the earlier raw exit 1 or the 20 reviewed baseline
critical expressions. Remaining warnings are not exhaustively cleared, and no
all-scope UBS pass is claimed. Scanner commands, authorization, UTC times,
unchanged scanner/input hashes, and combined coverage manifests are retained
under `target/managed-storage-validation-20261009/ubs-*-report-approved.*/`.

## Published integration CI (`a67ece95`)

All runs below are terminal. They tested the integration before the fixture
repairs above, so local repairs do not turn these failed runs into passes.

| Workflow | Observed result |
| --- | --- |
| [CI 37912525698](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37912525698) | Check, format, workflow/manifest guards, security, docs, and benchmark jobs passed. June Clippy failed on the inherited range loop; Linux x64 reported 308 CAS passes/3 failures/1 ignore. Both defects are repaired locally above. Linux ARM, both macOS jobs, and coverage reached their unchanged 30-minute limits; their required collection/gates are incomplete. Core E2E and release build were skipped. |
| [E2E 37912525685](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37912525685) | Ubuntu: 40 outer scripts passed, 1 failed at stream TEST6. The original stderr was not retained, so its exact historical prefix is unproven. macOS failed `bd-1vzb` because `/bin/grep` was absent, then hit the unchanged one-hour limit during `bd-2m7j` after a 31m34s release build. Remaining macOS scripts did not complete. |
| [Test Release 37912525784](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37912525784) | All five Unix build/version/package/upload jobs passed. Downloaded artifact ZIP digests, original tar checksums, and flat three-binary inventories were independently verified. Windows compilation failed on an inherited unresolved helper import at `rch/src/update/mod.rs:318`; Windows work is deferred. Aggregate verification was skipped. |
| [RABS 37912525782](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37912525782) | Accepted-hit protocol passed. Release compilation stopped after an explicit runner shutdown and exit 143; the shutdown cause is unknown. Size, replay, worktree, doctor, and overhead gates did not run. |
| [Rsync transport 37912525705](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/37912525705) | Both jobs passed, including real rsync transport checks. |

Full logs, GitHub job metadata, annotations, archive digests, and bounded source
triage are in `target/managed-storage-integration-validation-20261009/ci-a67ece95362f/`.
No timeout was increased and no job/assertion was disabled to admit this patch.
New source publication must receive its own CI record. Keep the PR draft.

## Integrated storage and archive repairs

Private `/tmp` now wraps only the workload. The native watchdog publishes its
process record in the worker's normal namespace, where daemon cancellation and
orphan discovery can see it. The scratch lease and cleanup remain outside the
tracked process group. Canonical source, target, and managed storage paths that
would be hidden by `/tmp` are rejected, including symlink aliases. Failed setup
also prevents the entire watchdog launch. A tmp-only private profile rejects a
native or explicit Cargo cache hidden by `/tmp` before using its contents.

A native before/after probe used the actual production shell scripts and real
mount namespaces. Before the repair, parent-side cancellation returned the
missing-record result while the private workload was running. Afterward it found
the live record, cancelled the job with status 137, and cleaned scratch. The
deadline path also retained status 137 and cleaned scratch. All four compiled
private-storage regressions then passed with mount privileges, requiring actual
mounted execution; ordinary-user runs exercise privilege refusal separately.

Main's package-output policy also selected Cargo's temporary verification copy
beside the final archive. `cargo package` now selects the final archive only;
publish dry-runs retain their existing temporary archive locations. Both use
the same conservative command parser. The real-Cargo success case retains its
single-archive and byte-identity assertions. Its failed-verification fixture now
proves the exact broken source and excluded input in a bounded final or temporary
archive, and requires exit 101 with Cargo's verification diagnostic. Both June
and August Cargo produced the temporary failed archive in local probes.

Integrated validation logs are under
`target/managed-storage-integration-validation-20261009/`. The first run's NFS
bind mount reported UID 0 for files created by UID 1000, causing Git ownership
and sticky-directory unlink failures. The same compiled tests passed unchanged
in a fresh local-disk `/tmp` namespace. No operator files were removed. Six
non-compiling source/manifest/workflow guards passed, including all six release
asset gate self-tests. June and August formatting checks passed. Broader
workspace, cross-platform, and release execution remains distinct from the
targeted results below.

| Integrated check | Result |
| --- | --- |
| Workspace/all-target/all-feature check and Clippy with warnings denied | Passed after the runtime changes; two subsequent rustdoc-only corrections do not change executable code |
| Workspace/all-feature/no-dependency rustdoc, warnings denied | Passed after correcting the test-only producer link and `/proc/<getpid>` markup |
| RCH `managed_` selection | 45 passed, including real Cargo archive success/failure checks and original watchdog status assertions |
| Privileged private-storage selection | 4 passed with actual mounting, native cancellation, deadlines, hidden-path and cache checks; overlaps the ordinary selection |
| Package-output policy / shared parser / shared rejection matrices | 3 / 1 / 2 passed |
| Native Cargo isolation | 5 passed |
| Shared execution storage | 6 passed |
| Runtime policy / native reentry refusal | 6 passed across 352 source files; 1 native-context test passed with zero I/O |
| Mock toolchain preflight | 1 test passed across 6 isolated child-process cases |
| Worker probe / reload CLI | 3 passed; 1 reload test passed across 12 real CLI/socket cases |
| Integrated debug binaries | `rch`, `rchd`, and `rch-wkr` built successfully as 2.1.16 |
| `bd-2ga8` daemon ownership and output scenarios | Native owned-child cleanup passed; all 11 scenarios passed, including 8 actual delegated remote completions and byte checks |
| Pipeline / output preservation / project sync on integrated binaries | 13 / 15 / 14 passed; source/helper and binary SHA manifests stayed unchanged |
| Full workspace tests, feature-gated `true_e2e`, complete 41-script suite, release gates | Not established by these selected checks; broader validation remains pending |

`bd-2ga8` previously lost its daemon PID through command substitution and
accepted obsolete-command usage errors as successful executions. It now retains
the actual daemon child, stops and waits for that child, preserves diagnostics,
and requires real delegated execution with a new matching daemon completion.
Native error scenarios validate typed errors and exact nonzero statuses.

The pre-merge debug binaries report `f207c5bf` in their Git stamp. Retained
source/binary SHA manifests bind these local tests to the integrated working
tree; they are not release artifacts or proof of a later published revision.

The final integrated UBS scan used the unchanged v5.4.33 scanner on 85 Rust/Bash
files, from 08:58:01.052481688 to 08:59:43.309100481 UTC on 2026-10-09. It returned
**exit 1**, with 19 critical, 10,569 warning and 5,234 informational records.
Input and scanner hashes matched before/after. All 19 critical source expressions
also exist in base `67b5d4b9`; none is on a feature-added line. Bounded review
found non-secret comparisons, intentional executable selections, test commands,
and a test receipt literal. Warnings and informational records were not
exhaustively cleared. This is not a passing UBS result. The exact command,
source hashes, triage and the user's scratch-cleanup authorization are retained
under `target/managed-storage-validation-20261009/ubs-final-integrated-report-approved.4FImGd0tNp/`.
After the two doc repairs and daemon-fixture repair, a three-file supplement
passed with exit 0 (0 critical, 281 warning, 103 informational records), replacing
the changed file's prior findings and bringing coverage to 87 distinct inputs.
The final `bd-2ga8` delegation update then passed a one-file scan with zero
findings; the other 86 input hashes still matched. Combined current criticals
remain 19, all baseline expressions. Neither supplemental pass erases the main
scan's exit 1. The final audit and combined input hashes are in
`target/managed-storage-validation-20261009/ubs-bd2ga8-final-report-approved.0WJ6wIhzB1/`.

## Published checkpoint repairs and evidence (`f207c5bf`)

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

| Check | Result at the published checkpoint, before main integration |
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
| Full workspace test collection | Interrupted during compilation with exit 143; no tests ran |
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
changed Rust/Bash files. Its checkpoint result is **exit 1**, with 6 critical, 3,817
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

## Remaining acceptance

1. Inspect the current branch and preserve existing work. Read the current PR
   checks and source-bound receipts before repeating any historical failure.
2. Complete full workspace/coverage and current-source CI. Preserve the original
   gates and report timeout, capability skip, runner shutdown, and assertion
   failure separately. The old rustdoc, checksum, runtime-policy, watchdog,
   binary-preparation, and failure-propagation defects above are already repaired.
3. Validate deployment, real SSH, and Nexus behavior in the user's environment
   when the user is ready. Privileged Linux mounting and native cancellation
   already passed locally; they do not prove acceptance on an operator's worker.
4. Leave Windows-specific repair deferred and keep PR #2 draft while required
   acceptance remains open. Existing Windows build failures remain visible.

The current CI/E2E/release configuration follows `rust-toolchain.toml` at
`nightly-2026-08-31`. Earlier June results remain historical; the alignment and
the unresolved June failures are documented above. Record the actual toolchain
for each result, and do not silently change pins just to obtain a pass.

Commands from the incoming handoff, retained as historical reproduction
examples; use the current revision and receipts when choosing a new run:

```bash
# Reproduce historical June policy/docs results, not the current CI selection.
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
