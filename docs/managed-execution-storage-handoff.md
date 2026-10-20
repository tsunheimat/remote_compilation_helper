# Managed execution storage: local Codex handoff

## October 20 continuation: executable and endpoint fixture custody

The published baseline is `69b95b9d418d035ed8affa58b4d83163aa175b25`.
[CI 39376800751](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/39376800751)
passed formatting/full fatal-warning Clippy, security/integrity, both macOS
workspace rows, Linux ARM64, both complete E2E jobs and their aggregate.
Linux x64 had exactly one failing test: the library toolchain-membership fixture
could not launch its `-vV` probe (`Text file busy`, OS error 26). Shipping remained
correctly gated. [RABS 39376800789](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/39376800789)
then completed every gate successfully, including M4 and doctor/overhead.
These are baseline results, not results for the following test-only patch.

The integration test's executable writer was already isolated, but the equivalent
library helper still opened executable files in the multithreaded test process.
A concurrent fork can inherit that writable descriptor until exec, even when the
originating descriptor is CLOEXEC. The library helper now opens its writer only
in a post-exec child, then reaps that child before any LiveFacts probe. Its second
script-replacement caller uses the same helper. Script bytes, permissions,
production probing and test parallelism are unchanged. The production prefix of
`live_facts.rs` is byte-identical to the baseline.

The new Linux regression deliberately retains an actual executable writer in an
owned process. The real probe must refuse it with ETXTBSY without running the
script. After the exact writer is released and reaped, the same unchanged bytes
must probe successfully, retain current facts, and emit exactly the two expected
identity-probe records. No production retry, timeout, guard or assertion is relaxed.

A subsequent repeated run exposed the same custody mistake in the legacy-socket
fixture: dropping its multithreaded-parent listener did not establish that an
unrelated fork had released every inherited listening descriptor. That fixture
now creates the listener after exec in a dedicated child, validates its readiness
against the exact child PID, retains the live-refusal/unchanged-inode assertions,
and releases and reaps that child before demanding ConnectionRefused and safe
stale takeover. The fixture's control message does not depend on pipe EOF. Its
owned-child guard also reaps on assertion failure. Production endpoint code and
its one-second bind checks are unchanged; the modified file is `cfg(test)` only.

On the configured `u25-agent-test` runner with pinned `nightly-2026-08-31`, the
final two-file patch passed formatting, `cargo check --locked --workspace
--all-targets`, and full-workspace/all-target/all-feature Clippy with fatal
warnings. It then passed **eight complete `rabsd --lib` runs: 769 passed, zero
failed, ignored or filtered in each run**, with normal parallel test execution.
Logs and exact validated file hashes are retained in the owned ignored
`target/fixture-custody-j6Di5E5X/` tree. Earlier runs, including the discovered
socket failure, are retained under `target/remote-pinned-validation/lib-probe-writer-er4jaauj/`.
The initial cold compilation batch reached its execution limit during Clippy;
the independent final batch completed successfully without changing any gate.
The runner's normalization of Cargo caches/builds to `/srv/remote-test` roots
was honored throughout. No compilation or testing ran on the source host.

UBS v5.5.3 static scans of the two changed Rust files and their unchanged parent
both exit 1. The ten
unchanged critical labels mistake filesystem `FileSig` values (device, inode,
size and timestamps) for secret signatures. Newly reported findings are confined
to test assertions/filesystem I/O and slash-containing shell strings; there are
no new critical or production findings. Raw reports and comparison are retained
under `target/ubs-fixture-custody-4jxha633/`. The scan is not described as clean;
no suppressions or skipped categories were added. Compiler-backed checks are
separate from this explicitly static scan.

The 12-workflow graph is unchanged. Keep PR #3 draft until the final published
revision passes all native rows, both E2E suites, RABS gates, six shipping targets
and `ci-success`. Main and deployed workers are unchanged.

## Upstream v2.1.34 integration — 2026-10-16

This section supersedes the checkout/status statements below; older evidence
remains historical. PR #2 was merged into `main` on October 12.

Integration branch: `feat/upstream-v2.1.34-integration`. Its starting fork commit
is `21c680abc36a07d1edbaf7c9e4c8f4e794165d9b`; its pinned upstream parent is
`ff152a046086f26845cbd4e9b0541a7b782fc64f` (annotated release `v2.1.34`).
The tag contributes 591 upstream-only commits. The moving upstream `main` was
not used. Development is isolated in a CodePier worktree; unpublished edits in
the original checkout, including new command-routing work, are not included.

The integration retains configurable package-cache/scratch/HOME placement,
private-workload `/tmp`, and persistent remote environment defaults. Source
claims, Cargo target pools, process ownership and durable completion remain
with their native RCH owners. Conflict reconciliation specifically combines:

- Managed configuration keys with upstream local-slot and Layer-0 controls.
- Upstream worker dependency preparation inside the managed environment,
  timeout and process group, without the old duplicate bootstrap path.
- Descriptor-relative recovery ownership with explicit lock release on drop.
- RABS capture/publication capacity reservations with per-action diagnostics.
- Upstream cold-build and package-verification tests instead of obsolete
  fixture expectations, while preserving private-path and cleanup observations.
- Both new RABS CI gates and the fork's existing resource diagnostics.

The dependency-preparation regression now invokes the production preparation
shell with a worker double. It checks exact argv/runtime, one preparation per
invocation, cache/proxy/scratch inheritance, dependency executable PATH, clean
workload stdout, refusal before workload start, and scratch removal on both
success and refusal. This is not a real package-install or fleet acceptance claim.

Validation at the initial integration checkpoint:

| Check | Observed result |
| --- | --- |
| Git conflict index and conflict-marker scan | Clear |
| `git diff --check` | Passed |
| Remote `cargo fmt --all -- --check` after formatting | Passed on the runner's installed formatter |
| Remote `cargo check --workspace --all-targets --locked` | Blocked before compilation: runner selects stable Rust 1.93 and rejects `-Z threads=4`; no rustup is installed |
| Pinned-nightly compiler, Clippy and Rust regressions | Pending GitHub CI on the integration revision |
| Real worker SSH, private-mount success, Nexus and fleet acceptance | Not performed |

All commands that execute tests or compilation are sent to remote/CI tooling.
Do not change the repository's `nightly-2026-08-31` pin to accommodate the
remote runner. The formatter result does not establish pinned-toolchain parity.
Keep the integration PR draft until its exact-revision checks and required
worker acceptance are complete. No main-branch update or deployment is part
of this checkpoint.

### CI follow-up after workflow consolidation

Observed on `1b9a330fe0b4123bc16134edbc88ddbaee54c268`:
[CI 38889175124](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38889175124)
passed security, bundle/workflow integrity and pinned formatting. Compiler-backed
Clippy stopped at an oversized staging error and fixed-size journal chunking;
the aggregate correctly failed and downstream native/E2E/package jobs did not run.
[RABS 38889174619](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38889174619)
built the release spine and passed the worker-cache, prepared-build, stream-drain,
output/session and wrapper-size gates. Its sample-gate suite reported 19 passes
and 8 failures, so later gates in that step were not executed.

The follow-up fixes the observed causes rather than relaxing CI:

- Keep the cold corruption attribution in a boxed payload, preserving the
  original typed failure and exact replica quarantine settlement. Remove the
  existing large-error suppression and add a Result-footprint regression.
- Use fixed-size array chunks for the digest journal while retaining its torn
  final-record handling and all existing byte-integrity checks.
- Persist SPRT checkpoints as append-only, increasing receipt revisions instead
  of overwriting the immutable `(kind, subject, seq=0)` record. Read the latest
  revision through the existing primary-key index; refuse malformed latest state
  and exhausted revision counters. Exact retries do not append, and consumed
  attempt markers survive conservative checkpoint rollback and database reopen.
- Initialize private-class and corruption cases independently rather than
  replacing immutable evidence. Exercise both terminal serving quarantines in
  fresh stores on both metadata backends and assert ordinary reset is rejected.
- Add regressions for successive checkpoint history, immutable conflicting
  writes, no self-generated journal churn, sequence exhaustion and invalid state.

On follow-up `97fdb6b29aaa79804f2b301375c8ebc110eabf49`,
[CI 38909444638](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38909444638)
passed the previously failing production checks and reached two additional
upstream test-only lints: a reference-count `>= readers + 1` comparison and an
unnecessary cloned one-element expectation. Their repairs preserve the same
reader admission and digest equality assertions. The main Clippy invocation now
uses `--keep-going` to collect independent target diagnostics in one run, while
retaining `--locked`, all targets/features and fatal warnings. A failed dependency
still blocks its consumers; this is not error suppression or a new CI job.

On `654c9ee8c38308c517bc6305cad455039afb1a33`,
[CI 38910363670](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38910363670)
collected the independent target graph and reported three further test-only
findings: an unused private-directory helper, a non-octal zero permission literal
and a runtime assertion on a constant retry bound. The snapshot fixture now uses
the existing private-directory helper; unreadable directories retain exactly mode
`0o000`; the retry bound is enforced at compile time. No test or assertion is
removed. This run passed pinned formatting and bundle/workflow integrity. The
corresponding RABS run passed both native installer jobs and release spine build;
its later Rust gates were still running when these findings were recorded.

The real coordinator/edge SPRT suite also gets a regression for a newer reset or
malformed checkpoint alongside an older 99-install qualification. It must refuse
replay without touching the destination or overwriting the older receipt, and a
conservative refusal must survive coordinator/database reopening. These are real
CAS/filesystem fixture operations, not a fleet or compiler performance claim.

The generic receipt immutability and quarantine-repair rules are unchanged. No
feature flag, assertion threshold or ignored-test list is relaxed.
The follow-up passed remote formatting and diff checks; its new Rust regressions
must still be evaluated by exact-revision pinned-toolchain CI. No successful
real-worker/Nexus/private-mount acceptance is implied by these fixture results.

### Replay and managed-shell continuation after `2f812efa`

The completed runs for `2f812efa63d979b11ebd3980004affc87ac96f60` are no longer
pending: CI `38911081979` passed pinned formatting, full-workspace/all-target/
all-feature Clippy and security/integrity checks. Native Linux/macOS tests and
shell E2E failed; shipping validation was consequently skipped. RABS
`38911081769` passed both installer jobs, release spine compilation and the
worker-cache/preparation/output gates, then failed in replay verification.

The next patch addresses observed runtime defects rather than changing replay
budgets or accepting failed executions:

- Preview now consults the same durable replay allowance as installation. An
  exhausted fixed budget must request fresh verification, not advertise a hit
  followed by `serve-failed`. Repeated preview is tested against actual CAS
  publication and installation without consuming receipts or creating outputs.
- Private staging uses an existing destination ancestor on the same filesystem;
  it does not create the subscriber's missing parent directories. Only final
  authorized installation creates those parents. Quarantine, abandonment and
  concurrent budget exhaustion must leave an absent subscriber tree absent.
  Exact-inode staging cleanup and partial-rename receipts remain intact.
- The live installer rechecks every destination after the staging lock handoff.
  A newly introduced symlink in a later output's parent must refuse the complete
  plan before replacing its earlier metadata output or touching unrelated files.
- Class-safety fixtures create bound generations before requesting live leases.
  The corruption test follows the existing whole-bundle staging contract: both
  fresh and populated subscriber trees remain untouched and the corrupt source
  is quarantined. A separate actual rename-collision test retains prefix coverage.
- Remote environment assignments now apply to a shell running the entire
  authored workload, before its variable expansion or pipeline parsing. This
  repairs the managed proxy/cache precedence failure. Tests retain inline
  overrides, pipeline inheritance, literal shell metacharacters, NUL output,
  stderr and nonzero status; no environment values are reparsed as source.

New coordinator and output-install tests are included in the existing RABS job,
not a new workflow. No warning, quarantine, lease, retry or replay limit is relaxed.
The final remote formatting attempt was blocked by SSH public-key authentication;
its fallback found no real Cargo executable and performed no build or test on
this source host. Diff checks passed; pinned formatting and compiler-backed
regressions must be assessed on the newly pushed revision by GitHub CI.

Unrelated failures remain open, including runtime-entry policy coverage,
platform-specific path/socket/process fixtures, Cargo package selection fixtures,
conservative cleanup observability, the obsolete automerge workflow contract
and native shell E2E. This patch is not a full-suite, fleet or deployment claim.
Keep PR #3 draft and do not update main or deploy workers.

### ZIP audit follow-up after `08875a2c`

At `08875a2c932661d77a45996298c544e26567c71f`, pinned formatting, workflow
integrity and both native RABS installer jobs passed. The audit database updated
on October 17 then reported three vulnerabilities in `zip 8.6.0`:
[RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367.html),
[RUSTSEC-2026-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368.html) and
[RUSTSEC-2026-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369.html).
The older October 16 audit pass does not qualify the current revision.

Upgrade the workspace and lock entry to patched `zip 9.0.0`. The registry's
checksum was independently matched against the downloaded crate bytes; the
selected no-default/deflate-flate2 dependency requirements are unchanged. No
unrelated dependency was upgraded or advisory ignored. Final Cargo `--locked`
and audit validation still belong to exact-revision CI, not this metadata check.
The updater explicitly rejects symlinks before directory-name classification.
Native extraction regressions preserve external bytes and permissions for both
relative and absolute link targets, including directory-shaped link names, and
prove archived SUID/SGID/sticky bits are present in the input but absent from
extracted ordinary files/directories. Existing safe regular ZIP extraction stays
covered. The unmaintained unsafe-libyaml and yanked yoke-derive warnings are
separate unresolved dependency-maintenance findings, not hidden exclusions.

### Permit-boundary and CI-policy follow-up after `44be5087`

Completed evidence for `44be5087bf98d15ff6791fbc3b0e8b1f9b0d8ad8`:
[CI 39150730505](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/39150730505)
passed the refreshed ZIP security audit and workflow integrity; two ZIP-test line
wraps stopped pinned formatting before Clippy. Those exact wraps are corrected.
[RABS 39150730517](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/39150730517)
passed both installer jobs, release compilation, 29 sampling tests, 14 staged
materialization tests, 28 coordinator tests, 14 complete-output tests, both class
safety tests and all five SPRT-budget tests. The replay target had six passes and
one failure; later release gates were not executed after that failing step.

The remaining replay test incorrectly treated a destination-arbiter refusal as
an admitted installation. Arbitration now precedes staging and permit spending:
that refusal leaves zero receipts, and the subsequent 24-way contention must
produce exactly 16 installed outputs, eight refusals and 16 contiguous receipts.
The test now checks these receipt identities as well as bytes and absent paths.
It does not change the production replay limit.

A separate deterministic regression preserves the opposite boundary: after real
private staging, a late filesystem obstruction reaches the production terminal
receipt/delivery function. The failure must retain its spent permit and leave
the obstruction untouched; only the remaining 15 installations are allowed,
and an exhausted retry cannot append another receipt. The production code is
factored without changing the ordering of authority checks, spending or delivery;
there is no test-only failure callback or mocked materializer success/error.

The stale auto-merge YAML test is replaced by `ci_workflow_policy.rs`: validation
contents stay read-only, privileged triggers/permissions and dangling job needs
are rejected, E2E/shipping retain one reusable PR owner, and dependency proposals
remain enabled without auto-merging. It includes 93 subprocess cases against the
actual aggregate Python program, not a copied result evaluator. Those Python
cases passed remotely; Rust YAML parsing and the new native regressions still
require exact-revision compiler/test results. No workflow is added or test gate
relaxed. Remote access is restored to configured `agnet-test` (hostname
`u25-agent-test`); formatter execution passed there with a source-host refusal
guard. This is not a pinned-toolchain or full-suite pass. Main and workers are
unchanged; PR #3 remains draft.

### Guarded-runtime and archive-contract reconciliation

At `d40a133b3839fdcd33346608bce460816a51561f`, CI `39198658477` passed pinned
formatting, security and workflow integrity. Clippy exposed ZIP 9's fallible
`ZipFile::name()` signature. Extraction now maps a decoding failure to a typed
installation refusal before any path is selected; it does not use lossy names.
A regression creates and successfully extracts a valid ZIP, changes only the
local/central filename bytes and UTF-8 flags, then verifies malformed extraction
cannot create a replacement-character output. The unchanged valid extraction
remains byte-identical. No dependency rollback or advisory exemption is used.

The older native failure log also exposed static review drift after runtime
entry was moved behind guarded adapters. The AST scanner now treats calls to
`block_on_top_level` and `block_on_worker_runtime` as runtime entry, including
opaque macros; original reviewed caller counts remain unchanged. Only the two
actual adapter bodies are newly reviewed entry owners, each borrowing an owned
runtime and rejecting an active Cx before calling block_on. New detector fixtures
reject both adapters from async/nested/macro/unreviewed contexts. Three split-out
fixture modules explicitly declare `#![cfg(test)]`, matching their already-gated
parent module declarations; all src files continue to be scanned independently.

Cargo archive expectations now distinguish the existing contracts rather than
widening production delivery: `cargo package` returns only final root archives;
`cargo publish --dry-run` may return tmp-registry/tmp-crate archives too. Both
managed/unmanaged cases assert exact selected and excluded path sets. The real
Cargo package/rsync fixture still verifies extracted-source provenance and final
archive byte equality, and now asserts scratch archives are not returned.

Remote formatting and diff checks passed. These changes do not establish native
runtime/packaging/ZIP-test success: those require the next pinned CI revision.
The d40 RABS run `39198658478` passed installers, release build and early worker
checks and was still executing replay verification at this checkpoint. No new
workflow, ignored test, warning exemption, runtime permission or artifact type
is introduced. Main and deployed workers remain unchanged.

### Native cancellation and stale-endpoint fixtures

Revision `5426abf4a07bafa773c01aff42281d2a7fdd38bd` passed pinned formatting,
full-workspace/all-target/all-feature Clippy with fatal warnings, the security
audit and workflow integrity in CI `39199931539`. Both native RABS installer
jobs and release spine compilation passed in `39199931558`. Native workspace
and E2E jobs started; their success is not yet established.

Two remaining older native failures are fixture/protocol mismatches. The paused
supervision daemon used to panic when a correctly cancelled wrapper closed its
socket before the delayed decision reply. It now accepts only BrokenPipe or
ConnectionReset, and only after the test explicitly confirms killing and reaping
the original caller. Ordinary replies, timeouts and unrelated errors remain
fatal. The test exercises both the original cancellation/reply race and an
already-exited wrapper before reply; neither may start a compiler, create the
forbidden output or report successful completion. Production supervision and
its cancellation deadlines are unchanged.

The stale-socket takeover fixture previously left a raw listener's umask-derived
permissions, although the real endpoint owner leaves mode 0600 and refuses to
retire a non-private socket. The positive fixture now reproduces that real mode.
A new real-daemon negative test supplies a connect-refused but mode-0660 socket
and requires a named privacy refusal without changing its inode, device, owner,
link count, permissions or ctime. A refused connect alone never grants cleanup
authority. No production endpoint policy or timeout is relaxed.

Remote formatting and diff validation passed; these two native regressions must
still pass in the exact final CI revision. The changes are in the existing
workspace suite, with no added workflow. Broader Cargo reuse, cleanup-activity
observability and shell E2E findings remain open until reproduced and resolved.
Do not merge main or deploy workers on the basis of intermediate Clippy passes.

### Completed native results and resumed integration after `1f374f02`

CI `39200654564` passed pinned formatting, fatal-warning full-workspace Clippy,
security and workflow integrity, but native suites and shell E2E failed. RABS
`39200654559` passed the replay/endpoint/maintenance/capture/input and real Cargo
layout gates; its large-input worktree case failed on missing readiness telemetry.
The final doctor/overhead gate did not run. These are failures, not acceptance.

The resumed batch reviews and retains the unpublished worktree repairs alongside
new native regression fixes. Both concurrent input warmers are now represented
by their actually observed `BothPending` state and `also_pending` diagnostic;
no additional polling delay is introduced. The cold Cargo test requires every
pending observation before execution and retains its full real-artifact ladder.
An unexplained debug-only toolchain refusal now retains complete daemon decisions
instead of only a tally; it remains open until its cause or final result is known.

Worker-owned `RCH_CH_BASE` cannot be overwritten through raw allowlists. ZIP names
flagged UTF-8 are validated as raw UTF-8 before decoding, while hostile permission
fixtures inspect raw archive attributes rather than ZIP 9's sanitized accessor.
macOS fixtures use physical temporary roots, bounded socket paths, actual flock
and timeout tools, and exact native EILSEQ refusal with lossy-neighbor protection.
The lifecycle socket fixture explicitly selects blocking reads on accepted streams
and adds a fragmented-request regression. Production endpoint authority is unchanged.

Positive cleanup fixtures re-execute the entire original test in an owned PID/proc
namespace so real CWD holders remain visible. Unknown host activity still refuses
cleanup. Linux thread-census and Darwin process-census fixtures use their actual
argument/column contracts; missing threads still retain records. Scoped sudo-unshare
requires an explicit ephemeral-CI opt-in, not a workstation or host policy change.
The Cargo placement gate now selects its actual module and rejects zero listed tests.
All 12 workflow owners and existing required checks are retained.

Remote `u25-agent-test` validation: four extracted-production activity cases passed
(idle, live CWD, recent write, unavailable process), 46 actual source-registry tests
passed, and all 93 cases against the actual CI aggregate passed. Stable remote
formatting and shell/diff checks passed. These are not pinned Rust/native-macOS or
whole-worker acceptance; exact published-revision CI must validate the new batch.
Original-checkout edits, main and deployed workers remain untouched. Keep PR draft.

### Pinned native validation and FIFO portability after `89cfa4ed`

At `89cfa4edd57261057c38583a82f82c38f223afa6`, CI `39210558778` passed
pinned formatting, fatal-warning full-workspace Clippy, security and integrity.
The Linux x64 native workspace job completed successfully. The complete Linux
E2E job also passed: Cargo true-E2E, explicit mock SSH and all 41 native shell
scenarios. The retained `e2e-ubuntu-latest` artifact (12168890907) contains 41
passing status records, including the formerly failing cleanup scenario.

Both macOS native jobs stopped during compilation because rustix does not expose
`mkfifoat` for that target. The excluded-tree regression now creates the same real
kernel FIFO through native `mkfifo -m 600`, then checks its type and exact mode.
Selected FIFOs still refuse capture, and excluded FIFOs still prove that no walk
occurs. No platform test is disabled and no production snapshot policy changes.

Pinned Rust is now installed in an owned directory under the remote validation
`target/` tree, not over the worker's system compiler and not through shell
profile changes. Installation used rustup 1.29.1 with its official checksum and
`--no-modify-path`. Tests report the repository-pinned nightly-2026-08-31 compiler
`90850177249efe0321573c569aec5d12b257f8d6`, and run only on `u25-agent-test`.
The temporary root of a remote job is private; setup and execution use one owned
persistent validation tree. No compiler/test falls back to the source machine.

Against the 89cfa4ed tree plus the FIFO change, pinned remote execution passed:

| Actual test selection | Passed / failed |
| --- | --- |
| Excluded-directory real-FIFO regression | 1 / 0 |
| `rchd` cleanup module, including owned PID/proc namespaces | 55 / 0 |
| `daemon_stop_safety`, including fragmented native socket requests | 8 / 0 |
| Extraction-related tests, including ZIP/tar safety and filename guards | 33 / 0 |
| Managed storage/environment/scratch regressions | 11 / 0 |
| Cargo-home boundary tests | 4 / 0 |
| Compound environment and authored inline override | 1 / 0 each |
| Complete real Cargo worktree target, including large inputs and tracked OUT_DIR reads | 10 / 0 |

All selections were nonempty and returned exit 0 without ignored tests. Pinned
formatting passed. Evidence and source hashes remain in the ignored remote
validation tree, under `evidence-owned-tools-8mm93nf3` and
`runtime-evidence-7osr51sz`. These are targeted native-Linux results, not proof of
macOS, six-platform shipping or a full final-revision CI pass. The broader RABS
release job had reached the real layout tests at this checkpoint. Final revision
CI must still qualify the FIFO portability correction and all remaining gates.
Main, original-checkout edits and deployed workers remain unchanged; PR is draft.

### Native shell loading, UUID authority and heartbeat observation after `63425131`

Baseline `634251316f056f347ac9b206ba02be27b45df6c9` completed the entire RABS
release workflow `39216702473`, including actual Cargo reuse and final overhead
checks. CI `39216702477` passed pinned formatting, fatal-warning Clippy, security,
integrity and the complete Linux E2E job. The four native workspace rows and
macOS shell E2E still failed, so packaging was skipped and the PR was not ready.

The next batch fixes an actual Bash 3.2 loading defect in both shared control
scripts: unbalanced case patterns inside command substitutions break when the
literal protocol is evaluated, although `sh -n` on the outer heredoc passes.
Balanced POSIX case patterns preserve the existing stat commands, error status,
receipt checks and permissions. The existing syntax regression now actually
loads the compiled protocol directly and in a cleanup subshell, retaining opaque
caller arguments. Cleanup now reports a failed protocol bootstrap as an error
instead of swallowing stderr and presenting zero removals as a successful scan.
Its new negative case leaves the original aged process record untouched.

The macOS source-sweep fixture incorrectly assumed missing Linux procfs meant
ownership was unavailable, despite the existing native uuidgen fallback. It now
exercises failed, malformed and valid native UUID generation with exact directory,
claim and byte checks. Production also checks the generator's exit status before
normalization: a failed uuidgen that emits a plausible UUID must never acquire
authority just because the final tr process succeeds. A separate Unix regression
exercises that exact failure and success boundary without deleting its sentinel.

The preflight heartbeat test accounts for its independently scheduled executor:
Drop cannot retract a complete immediate heartbeat already in the socket. The
explicit preflight flush is still required before rejection; at most two startup
messages may arrive across two subsequent production intervals. A leaked periodic
sender exceeds that bound. The positive control compares two post-transition
periodic snapshots and still forbids invented progress. Production heartbeat
periods and cancellation behavior are unchanged.

Verification on `u25-agent-test`, pinned nightly-2026-08-31:

- `cargo fmt --all -- --check` and full-workspace/all-target/all-feature Clippy
  with `--locked --keep-going -- -D warnings` passed.
- The new UUID-status test, three protocol/quoting tests, preflight-heartbeat
  regression and all 14 cleanup-command tests passed. Evidence is retained under
  `target/remote-pinned-validation/{uuid-fallback-current,parser-regression-current,
  heartbeat-current,cleanup-commands-current,current-full-clippy}.log`.
- In one before/after invocation, 12 parser comparisons covered dash, modern
  Bash and a private Linux build of GNU Bash 3.2, with direct and subshell loading.
  Both old-Bash baseline cases failed with exit 125; the patched cases and both
  baseline/current modern-shell cases passed with unchanged arguments. This is
  actual shell-parser reproduction, not native macOS execution or fleet proof.
  Evidence: `target/remote-pinned-validation/bash32-parser-comparison.log`.
- The unchanged capture suite passed once and in eight fixed CPU-constrained,
  traced repetitions of all eight tests. That non-reproduction does not resolve
  the earlier CI exit-1-versus-122 failure. No assertion or deadline was relaxed.

The ARM cold-Cargo refusal remains unexplained; the fixture now preserves the
actual daemon refusal detail when it fails instead of destroying the log after
printing only a tally. The macOS full-hook 10 ms performance gate also remains
unchanged and unresolved. The final native CI revision must qualify these changes;
no former run is relabeled as a new-revision pass. All 12 workflow owners remain,
and main, original-checkout work and deployed workers are untouched.

### Direct process formation and complete SSH previews after `798cb378`

Published baseline `798cb3781294f99dad0b65ae63c86c4021f127fb` completed
CI `39235058108`: pinned fatal-warning Clippy, formatting, security, integrity,
Linux x64 workspace tests, and both Linux and macOS E2E jobs passed. The macOS
E2E result includes the unchanged full-hook timing gate. Both macOS native rows
failed the UUID sweep fixture, ARM macOS additionally failed the cold-wait
rollover fixture, and Linux ARM failed real vendored Cargo reuse. Shipping was
therefore correctly skipped. RABS `39235058119` also failed its real-Cargo M4
phase, despite passing the preceding release and lifecycle gates.

The preserved Linux ARM and release daemon logs identify the same refusal:
`process_group(0) not honored: live leader ... not found in any /proc pgrp`.
Formation used a best-effort host scan that excludes zombie leaders and then
assumed two nonterminal waitpid observations made a missing scan entry proof of
failed setpgid. The new proof reads the directly owned, unreaped child's bounded
stat record and checks PID, parent and process group, including terminal states.
It preserves the initial full membership observation separately. Formation is
not quiescence, unknown live identity still refuses, and the existing armed
unadmitted-child guard still kills/reaps on failure or unwind. Regression coverage
includes a real unreaped zombie, a real live child in the wrong group, malformed
identities and the existing descendant, unwind and bounded-output cases. A native
pthread fixture deterministically reproduces the exact old refusal predicate:
the main thread is Z, another thread remains alive, the process-only scan is
empty and two waitpid polls are nonterminal. The corrected production formation
check accepts the same owned process; the test then kills and reaps it. The
fixture also has its own natural alarm bound. All 18 process-group tests pass.

The actual RCH -> daemon -> SSH qualification exposed a separate data-loss defect:
`read_line(String)` silently ended the SSH preview reader on invalid UTF-8 and
lost the later output. Both lanes now read bounded byte fragments before text
rendering, retain valid split UTF-8 and delimiters, and report real I/O errors.
A JoinSet owns the readers through cancellation and checks their terminal results;
cutting off a post-completion output tail is unconfirmed transport, not success.
The text-preview contract remains unchanged: workload text goes to stderr and
stdout remains the protocol surface. Exact binary data is returned as artifacts,
not misrepresented as a lossless Unicode preview. New tests cover both lanes,
NUL/invalid bytes, unterminated tails, bounded long fragments, read failures and
closed consumers.

The UUID fixture now requires exit 1 and no successful metrics for failed or
malformed UUID admission. A successful native sweep removes only the two stale
targets and checks exact retired-claim contents, including the fresh job whose
ordinary TTL activity check runs after ownership acquisition. This test now runs
on every Unix host. Production admission/retirement policy is unchanged.

The cold-wait rollover failure remains unproven locally: 35 singleflight tests
pass, including exactly 60 preparation replies before reconnect. Its test now
retains phase/reply counts, elapsed time and typed I/O refusal diagnostics, without
changing timeouts, retries, protocol authority or release-wrapper logging.
Capture's stalled-stream fixture likewise retains the non-stalled diagnostic
instead of discarding it; its exit-122 and unread-consumer assertions are intact.

Additional maintenance keeps the async CLI frame outside the pass-through entry
and separates the SSH fixture's local inetd audit log from actual remote stderr.
The same-invocation Linux development comparison was 5.55 ms versus 5.02 ms over
50 samples per binary, with all 14 real-process behavior cases unchanged. This is
not release/fleet performance evidence. All 54 real SSH/rsync policy cases and
nine imported Rust receipt-policy tests passed; spoofed server notices on remote
stderr still refuse, as do changed host keys and failed authentication.

Pinned `nightly-2026-08-31` validation on `u25-agent-test` passed all four new SSH
reader tests, all 18 process-group tests, all 35 singleflight tests, the complete
10-test real-Cargo worktree target and full workspace/all-target/all-feature
Clippy with `--locked --keep-going -- -D warnings`. Logs are under
`target/remote-pinned-validation/merge-ready-*.log`. The aggregate driver initially
failed only because its second SSH job correctly rejected missing native worker
topology configuration; the real capability probe has independent
`RCH_WKR_CANONICAL_ROOT`/`RCH_WKR_ALIAS_ROOT` settings, not the client's TOML.
The fixture now configures these owned paths and verifies the real worker's
healthy topology before starting its daemon; it does not disable preflight.

The corrected actual SSH run `rt-20261019-131320-x7y42kzg` passed failure (23) and
success (0) jobs, complete text previews, exact NUL/non-UTF-8 binary artifacts,
opaque argv, managed persistent cache/environment, distinct source inodes and
mount namespaces, and scratch removal. The daemon terminated cleanly. Evidence
and binary/source identities are in the owned
`managed-ssh-gcttlho6/verified-preview-b0tkg869` directory. No persistent worker
service, source-host compiler, global SSH configuration or deployed binary was
changed; this is isolated runner qualification, not fleet rollout.

UBS was unavailable on both hosts. A private source checkout pinned to
`3729ccf088dc4fbdcf45d9193dfb8a939a08ea06` runs static analysis on changed files;
its Cargo phases are explicitly separate from the pinned checks above. Its five
critical reports were present at the published baseline: three fixed test shell
protocols, the intentionally caller-configured process executor API, and a native
fixture resolving only literal gfind/find tool names. Each boundary is reviewed
and documented with a single-rule source annotation, not a global rule disable.
The default static scanner verdict is now exit 0 with zero unsuppressed critical
findings; other findings remain visible. Required compiler warnings, audit rules,
assertions, deadlines, platform selections and all 12 workflow owners are intact.

These working-tree results do not qualify a later commit automatically. Keep PR
#3 draft until its exact published revision passes native CI, both E2E jobs,
RABS release gates and all six shipping platforms. Main and the original
checkout's unrelated edits remain untouched.

### Diagnostic rendering and publication-timer qualification after `63f591cb`

The full remote RCH unit run exposed a reproducible presentation-test defect:
the default runner socket path wraps across two graphical diagnostic lines.
The underlying error still contains the complete path. The regression now
checks the exact unwrapped diagnostic for both the default and an explicitly
long quoted path, then checks the rendered code, cause and all recovery commands.
Production formatting and socket selection are unchanged.

A controlled default-concurrency run (96 visible/affined CPUs) also isolated a
publication-test false attribution: its timer fired after 26.818 ms, but the
one-second timeout later cancelled durable publication (1.103 s overall). The
revised test polls both actual publishers against an independently held output
lock and retains the SAME one-second responsiveness limit and two-second owner
watchdog. It rejects any publisher completing before release, verifies untouched
outputs at the timer, then finishes both original publication futures and checks
all exact bytes, complete journals and removed stages. Slow subsequent fsyncs
are not misreported as a blocked lock-wait reactor. The production publisher,
lock budget and cancellation custody are unchanged.

Negative control: temporarily restoring a blocking `File::lock` before the real
lock loop made this revised test fail with `runtime timer stalled: 2.043 s`.
The mutation was removed, and the restored file's SHA-256 exactly matched the
previously passing source. No intentionally broken implementation is committed.
The local-build-alarm ordering test only gains explicit state-error diagnostics;
its deadline, failure assertions and scan/transition policy remain unchanged.

Pinned remote validation after restoration passed formatting, workspace/all-target
`cargo check --locked`, complete workspace/all-target/all-feature Clippy with
`--keep-going -- -D warnings`, and all 3,736 RCH unit tests (zero failed, one
pre-existing ignored). The full unit target also passed before the mutation;
all 29 diagnostic tests and 65 SSH/source-ownership tests passed independently.
No CI parallelism, workflow, test-selection or production setting was altered.
Evidence is under `target/remote-pinned-validation/`:
`resume-phase-observation-tu3133xe`, `resume-causal-publication-cw6sJu61`,
`publication-blocking-negative-k6gma2w2.log`, and
`resume-final-validation-7z68ie4n`.

The auxiliary UBS scan returned exit 1 on eight critical-labelled findings in
unchanged code; the baseline scan of the same three files returned those exact
eight findings too (`resume-static-baseline-5voyf58x`). Context review identifies
seven comparisons of public exit codes, counts or fixture state strings, not
secret authentication values, and one execution of a fixed, freshly compiled
Go fixture beneath its owned temporary directory. No critical finding was
introduced, no comparison was obscured, and no scanner rule was suppressed.
This is a recorded false-positive review, NOT a clean scanner exit claim.

The running hosted native/E2E/release validation remains tied to its published
revision; these local results cannot qualify a later commit by themselves.
Keep the integration draft until the final published revision's required checks
complete. Main, the original checkout and deployed workers remain unchanged.

### Worker jobserver EOF and retained release-gate evidence

Published `63f591cbf55a3351b478a1097150e38766f7f41f` completed all 41 macOS
shell E2E scenarios, as well as Linux E2E. The Linux x64 workspace's remaining
failure was `rabs-wkr --lib`'s `bridge_kernel_credit_plus_implicit_slot_equals_the_grant`:
its final read observed WouldBlock, not EOF, while unrelated parallel tests
could temporarily retain the writer between fork and exec. The kernel-credit
fixture now runs its exact assertions in a child that creates all its FIFOs
after exec. It also deliberately retains a cloned writer, proves unlinking does
NOT authorize EOF, then requires EOF only after the final writer closes. Token
counts, the implicit slot, path cleanup, and all production bridge code remain
unchanged. A child completion marker prevents a zero-test process from passing.

An earlier saved validation invoked this library-only test through the binary
and selected zero tests; that output is not qualification. The corrected pinned
remote invocation explicitly selected `--lib` and `--exact`: 1 passed. The full
worker library then passed all 158 tests with no failures, ignored or filtered
cases, followed by full-workspace/all-target/all-feature Clippy with fatal
warnings and formatting. Evidence: `worker-bridge-final-db7bJDD9` under
`target/remote-pinned-validation/`.

RABS's hosted release M4 gate is still a separate open failure; a previous local
development-profile pass is not release qualification. The existing release
workflow now retains M4, doctor and overhead logs as an artifact using pipefail,
without changing their commands, thresholds, selection, or failure handling.
There is no new workflow. Keep draft until the final published revision completes
its native, E2E, RABS release and six-platform shipping gates.

### Final native failure isolation after `ca74bf55` — October 20

The completed `ca74bf55258c83214700eddb4e0cacf51aa877b9` hosted runs narrowed the
remaining blockers. RABS `39283814245` passed every job and gate, including real
Cargo worktree reuse, doctor and enforcing overhead. CI `39283814010` passed
Clippy/format/integrity, both ARM64 workspace rows, both complete E2E jobs and
aggregation. Shipping correctly remained skipped because the two x64 workspace
rows each failed one test. The security job found zero vulnerabilities but its
GitHub check-update API returned 404; a targeted rerun passed without changing
any dependency, advisory rule or audit configuration.

Linux x64 failed `live_output_capture` while probing a newly replaced fixture
compiler: `Text file busy (os error 26)`. Executable writers now open only after
exec in a dedicated child, and that child is reaped before LiveFacts probes it.
This prevents an unrelated parallel fixture's fork from retaining the parent's
CLOEXEC writer. The equal-byte replacement still verifies identical content and
a different inode; stale observation rejection and all publication assertions
remain. No production retry or toolchain-admission policy changed.

macOS x64 failed the actual singleflight wrapper after a cache hit was accepted.
Its assertion did not retain stderr, so the particular hosted error could not
be reconstructed. The adapter nevertheless had a concrete Darwin close race:
Apple's [sosetoptlock implementation](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/uipc_socket.c#L4749-L4754)
rejects timeout updates with EINVAL after both halves close, even with a final
reply still queued. The Apple-only branch now performs a single nonblocking
read after that precise error and restores blocking mode. An empty live socket
returns the original error, never fabricated EOF or an unbounded read. Absolute
deadlines, ownership checks, strict framing and post-acceptance fail-closed
semantics are unchanged. Native peer-close tests and a forced timeout-error
adapter test cover these boundaries; the integration assertion now retains its
complete process result. Native macOS execution is still required to qualify
this change and determine whether the hosted symptom is fully resolved.

Pinned remote job `rt-20261020-054300-abxaw80i` on `u25-agent-test` passed 92 wrapper
unit cases plus 8 capture, 8 live-protocol and 7 singleflight integration cases.
All 12 live-output-capture cases passed once, then eight more fixed repetitions
with normal parallelism. Full-workspace/all-target check, all-feature fatal-warning
Clippy and formatting also passed. Evidence is retained under the existing
ignored `target/remote-pinned-validation/peer-close-fixture-1QOze2eK/` directory.
These Linux results are not macOS or final shipping qualification. The requested
precommit UBS invocation was unavailable on the authorized runner (exit 127);
no UBS pass is claimed. No workflow, production timeout, warning policy or test
exclusion changed. Keep the PR draft until the newly published revision passes
its full required graph.

## Historical October 12 handoff

Snapshot: 2026-10-12 UTC, local development continuation. Consumer: the draft PR
reviewer and the user's next local session. The user requested continued
development and said they will deploy later. Keep PR #2 draft. Deployment and
live worker/Nexus acceptance remain with the user. Windows-specific repair and
validation are deferred following the user's request to consider skipping
Windows support; existing Windows jobs have not been disabled.

The newest CI evidence is first. Earlier sections preserve historical outcomes;
the [remaining acceptance](#remaining-acceptance) section gives the current
operator sequence.

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
- Published toolchain/hook repair baseline: `968f1b16788cdb2a6355e7f58223605e46fc480c`
- Published logging/cache/fixture repairs: `4e65b0a60ad5d97545dae20ff2fb5241b17faf18`
- CI baseline code tree: `4a878e86833d3040474e7d18776f25b5b9e301ad`
- Published canonical-root fixture repair: `4034fe2bcfb3a0ac77ee6f090addb3ab75932152`
- Published filename fixture and CI diagnostics: `425dee6fec669955ff81316e1d4f56a485b248d7`
- Published Linux compiler bound and fixture priority: `5352da591f6202a7d8727d9d94e3623eb4e9c939`
- Published Darwin fixture repairs and approved CI ceilings: `352f025f10252d1bdc09191a6b099cc2a05760fb`
- Published CI prerequisites and status evidence: `c5618ef688a1691f4b0b305a5249a56bdca6cde9`
- Published prepared-wait and coverage-report repairs: `8edaa0f0315697720fcb4453bcaed1293aec9f59`
- Published daemon response and native fixture repairs: `18e2235faf2df206939641c3256b4e6689ac0e09`
- Published live-dependency fixture and timing diagnostics: `a836301151a10f6c4e5efca380fbfa97a8b46779`
- Published hook resource diagnostics: `26cda270149e57aefee1fdf245b6b4190b730114`
- Published isolated warmup and native target contracts: `2960380a9d4821c0180b0c6917e53e1977cd8523`
- Published buffered edge and workspace fixture repairs: `f9c7ba62c763ee82dfde8be2b1d6c3d473a9b403`
- Published ownership, native fixture and coverage preparation repairs: `4ea74ded45adf1cefcb6ae0d240a0e4698a00852`
- Published process-observation fixtures and native CI preparation: `1cbcf0e4720c357e36bec4e6b7955d37af7ce1ae`
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

## Completed CI at `1cbcf0e4`

The latest source includes the process-observation fixtures, core E2E support
preparation and native Intel release runner described below. The source archive
from PR merge revision `3cbb8c01f9ac178732c6758794284899c698fd2a` independently
matches all 1,391 tracked blobs and executable modes at this feature head.

Every workflow and job in this cycle is terminal. All four workspace platforms
completed successfully, and the Intel Mac release retry passed. The ARM Mac
release retry still exceeded 30 minutes. Main CI and its aggregate check failed;
Windows release compilation also remains deferred. Neither canceled ARM build
is a complete release pass.

| Workflow or platform | Result |
| --- | --- |
| [Main CI](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38212980893), Linux x64 | Attempt 1 completed all 225 workspace harnesses: 12,126 passed, zero failed, 31 ignored and zero filtered. Carried forward into attempt 2. |
| Main CI, macOS ARM64 | Attempt 1 completed all 216 workspace harnesses: 10,028 passed, zero failed, 23 ignored and zero filtered. The inherited Mac workspace selection excludes `rchd`. Carried forward into attempt 2. |
| Main CI, Intel macOS | Attempt 2 completed all 216 workspace harnesses: 10,028 passed, zero failed, 23 ignored and zero filtered. Its separate filesystem and socket selections also passed. |
| Main CI, Linux ARM64 | Attempt 2 completed all 225 workspace harnesses: 12,126 passed, zero failed, 31 ignored and zero filtered. Its separate filesystem and socket selections also passed. |
| Main CI, coverage | Test execution, LCOV, HTML, text reporting and the unchanged 65% gate passed on attempt 1. The verified HTML reports 87.07% line coverage: 322,720 / 370,628 lines. Carried forward into attempt 2. |
| Main CI, core E2E | Attempt 1 passed all 164 Cargo cases, with zero failures, ignores or filters. All 13 shell cases completed: 11 passed, zero failed, and two opt-in cases skipped. The full job took 29m21s within its unchanged 30-minute limit. Carried forward into attempt 2. |
| Main CI, release builds | Linux x64, ARM64 and musl completed successfully in attempt 2, and Intel Mac passed in attempt 3. ARM Mac's attempt 4 retry exceeded 30 minutes despite restoring its cache, with no compiler error, completed release build or artifact. Both first Mac builds also exceeded 30 minutes. Windows reproduced nine Unix API errors in `rabs-asupersync` and remains deferred. |
| [Standalone E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38212980845) | Ubuntu and macOS each passed all 41 scripts. Each inner suite recorded 11 passes, zero failures and two explicit configuration-dependent skips. All four cleanup-observation cases passed natively on both platforms. |
| [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38212980884) | Protocol and release jobs passed: 36 + 8 protocol cases and 13 release cases, none failed, ignored or filtered. The wrapper was 444,000 bytes. The unchanged 60-sample enforcing gate recorded 3.445 ms p95 below 10 ms. This is not a paired performance comparison. |
| [Rsync](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38212980872) | 64 passed, zero failed or ignored, 3,309 filtered, using rsync 3.2.7. Compiler, format and lint checks passed. |
| [Test Release](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38212981009) | All five Unix build/smoke/package jobs passed. Every current archive independently passed its published artifact digest, archive checksum, exact executable-member list and executable-mode checks. Windows reproduced the three deferred RCH import/type errors; aggregate package verification skipped. |

Format, workflow and manifest guards, compiler checks, strict Clippy, docs,
security and benchmark gates passed on attempt 1. The separate nine-case
filesystem and 13-case socket selections passed on all four workspace platforms
in that attempt. These repeated selections are separate from the complete
workspace counts. GitHub carries successful jobs into the retry with new job
identifiers; they were not executed twice.

The first Linux ARM attempt spent 3m03s and 49m29s in its initial focused and
workspace preparation. Intel Mac spent 7m12s and 43m31s in those phases. Their
subsequent workspace invocations reused the prepared artifacts. Neither log
recorded an assertion failure, but neither run completed: Linux ARM's 148
finished harnesses recorded 2,988 passes and 11 ignores; Intel Mac's 152 recorded
6,121 passes and three ignores. Unfinished harnesses are excluded from those
counts. Both jobs have explicit 60-minute cancellation annotations, and both
compiler caches uploaded completely after cancellation.

One targeted retry of these canceled jobs and their dependents started at the
same head. Both workspace suites completed successfully after restoring the
saved compiler caches. Tests, concurrency and all deadlines remain unchanged.
This completes their validation; it is not a code fix for initial compilation
duration or a performance improvement claim. The original partial results
remain preserved. No additional job-budget proposal was applied.

The main release jobs first ran in attempt 2. Both Mac logs report no restored
cache and explicit 30-minute cancellation annotations. Their cache upload steps
then succeeded completely: 1,461,100,619 bytes for ARM and 1,486,869,692 bytes for
Intel. One targeted retry per Mac job used these saved caches at the same source
and unchanged release limit. GitHub rejected simultaneous individual-job retries
because the workflow was already running, so Intel ran first and ARM followed.
Windows and the successful checks were not rerun. These retries do not fix cold
compilation duration or erase either incomplete first build. Attempt 2's overall
workflow and aggregate check failed; all six release logs and timeout annotations
were preserved before the retry began.

The Intel retry restored the exact cache key and 1,486,869,692-byte archive from
its canceled attempt. It completed the unchanged whole-workspace release
compilation in 20m45s and uploaded its artifacts successfully. Attempt 3's
aggregate remained failed because ARM was still canceled and Windows failed.
Those results were saved before starting the ARM retry.

ARM's retry restored the exact cache key and 1,461,100,619-byte archive, but
again received the explicit 30-minute cancellation annotation. Its log has no
completed release-profile marker or compiler error, and artifact upload skipped.
No further retry was started. Attempt 4's final metadata, raw log and annotation
are preserved in `attempt4-terminal-ci-outcomes.json` and
`job-114749328664.log`; the log SHA-256 is
`be73159d26f684f517f50d0f4b9e1646f789b3e47c834c68734b835da0225f5b`.
The already-passing ARM Test Release package uses the distribution profiles and
does not replace this incomplete whole-workspace `release` build.

The complete standalone Mac run passed the unchanged hook gate with a 9.708 ms
mean across all 50 samples. The preceding revision's 13.102 ms failure remains
recorded below; the cause is unestablished and no timing fix is claimed. The
core shell suite explicitly skipped installation prompting and opt-in true E2E.
Its existing advisory performance helper is not an enforcing gate. These mock
SSH checks do not establish deployment, real-worker or Nexus acceptance.

The verified coverage HTML also reports 98.46% for
`rch-common/src/execution_storage.rs` (255 / 259 lines). No exact coverage-runtime
test count is claimed because the complete job log was not retrieved. This
hosted result is separate from the earlier failed local coverage attempts.

Raw logs, verified archives, source/package proofs, timeout annotations,
first-attempt outcomes and retry metadata are retained in
`/mnt/hdd-vibe-coding-share/remote-test-local-fallback-claude1/tmp/rch-ci-head1cbcf0-monitor-wql4kji8/`.
Windows jobs remain enabled, with Windows repair deferred. PR #2 remains draft.

## Approved job budgets after `1cbcf0e4`

On 2026-10-12 the user explicitly approved "Approve 90-minute workspace tests"
and "Approve core E2E 60 minutes" for the two prepared one-line proposals.
The workflow raises the four workspace-test jobs from 60 to 90 minutes
and core E2E from 30 to 60 minutes. Coverage remains at 60 minutes and the main
release matrix at 30 minutes.

A parsed comparison with `1cbcf0e4` proves those are the only workflow changes.
All commands, package and test selections, compiler settings, assertions and
performance/coverage thresholds are identical. Every Bash step in the two
affected jobs passes syntax checking. The recorded workspace and core E2E
results above completed under the original, smaller limits; the approved limits
were not part of those runs. This is scheduling headroom, not a performance
improvement claim or a reclassification of the earlier cancellations.

The exact before/after YAML and comparison are retained as
`ci-approved-job-budgets.before.yml`, `ci-approved-job-budgets.after.yml` and
`ci-approved-job-budgets-validation.json` in the current CI evidence directory.

The required UBS invocation for the two changed files returned exit 3,
`no-supported-languages`: this scanner has no YAML or Markdown module, so it
examined zero files. That is unavailable scanner coverage, not a passing scan;
no override or suppression was added. Its unchanged pinned implementation,
exact command and result are retained in
`target/managed-storage-validation-20261009/ubs-handoff-budget-report-approved.q4yiyhkb/`.
The workflow comparison and Bash syntax checks above provide direct validation
of the budget edits. Rust source is unchanged from the tested `1cbcf0e4` revision.

This publication changes only the two approved timeout values and this handoff,
and uses `[skip ci]`. No new CI run or binary validation is claimed for that
configuration/documentation checkpoint. The completed and failed results above
remain bound to `1cbcf0e4`; its missing ARM release result is still missing.

A separate one-line proposal raises the main release matrix from 30 to 60
minutes. It passed `git apply --check` and is retained as
`main-release-job-budget-proposal.patch` in the current CI evidence directory.
Approval was requested separately because the earlier two approvals do not
cover that job. It remains unapplied. No coverage-budget increase was applied.

## Completed CI at `4ea74ded`

Every workflow in this cycle is terminal. The four-platform workspace matrix
and coverage passed. The source archive from PR merge revision
`26e456dd3a9f11aef3aa2500571020112760258e` independently matches all 1,391 tracked
blobs and executable modes at the published feature head.

| Workflow or platform | Result |
| --- | --- |
| [Main CI](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38205488944), Linux x64 and ARM64 | Each completed all 225 workspace harnesses: 12,123 passed, zero failed, 31 ignored and zero filtered. |
| Main CI, macOS ARM64 and Intel | Each completed all 216 workspace harnesses: 10,028 passed, zero failed, 23 ignored and zero filtered. The repaired concurrent-reader, large ownership-closure and completion-marker cases passed on both native platforms. The inherited Mac workspace selection excludes `rchd`. |
| Main CI, coverage | Test execution, LCOV, HTML, text reporting and the unchanged 65% gate passed. The verified HTML artifact reports 87.08% line coverage: 322,736 / 370,600 lines. Full job-log retrieval failed, so no exact coverage-runtime test count is claimed. |
| Main CI, core E2E | All 164 Cargo cases passed, with no failures, ignores or filters. Three shell cases passed and installation prompting explicitly skipped; the 30-minute job limit interrupted the path-dependency case during compilation. The remaining shell suite is incomplete. |
| Main CI, release builds | Linux x64, ARM64, musl and macOS ARM64 passed their full-workspace builds and artifact uploads. The x64 macOS build on an ARM host exceeded 30 minutes and produced no artifact. Windows failed on nine Unix API errors in `rabs-asupersync`, separate from Test Release's RCH errors. |
| [Standalone E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38205488980) | Ubuntu passed all 41 scripts, with 11 inner passes and two explicit configuration-dependent skips. macOS completed 39 passes and two failures: stale process-record retention and hook timing. Its inner suite recorded 10 passes, one failure and two skips; the hook script stopped after its timing assertion, before later cases. |
| [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38205489017) | Protocol and release jobs passed: 36 + 8 protocol cases and 13 release cases, none failed, ignored or filtered. The unchanged 60-sample enforcing gate recorded 2.610 ms p95. This is not a paired performance comparison. |
| [Rsync](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38205489034) | 64 passed, zero failed or ignored, 3,309 filtered, using rsync 3.2.7. Compiler, format and lint checks passed. |
| [Test Release](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38205489335) | All five Unix build/smoke/package jobs passed. Every downloaded archive independently passed digest, checksum, member-name and executable-mode checks. Windows reproduced the three deferred import/type errors; aggregate package verification skipped. |

The separate nine-case filesystem and 13-case socket selections passed on every
workspace platform. Format, workflow and manifest guards, compiler checks,
strict Clippy, docs, security and benchmark gates passed.
Main CI's workflow conclusion is canceled, and its aggregate check failed.
Neither timed-out job is a complete pass. Windows repair remains deferred.

The failing Mac hook measurement used all 50 samples: mean 13.102 ms against the
unchanged 10 ms limit, median 11.320 ms and mean child CPU time 11.034 ms. The
preceding revision passed on a different runner. These observations do not
identify the cause or establish a paired performance regression; the failure
remains recorded and no timing assertion was changed.

Core E2E spent about 26 minutes preparing release/debug binaries and running its
Cargo suite before beginning shell cases. The follow-up below removes one
redundant binary preparation without changing its 30-minute ceiling. The
additional proposed job-budget increases remain unapplied.

Raw logs, verified archives, source/package proofs, timeout annotations and
complete/partial outcomes are retained in
`/mnt/hdd-vibe-coding-share/remote-test-local-fallback-claude1/tmp/rch-ci-head4ea74d-monitor-ar1ynfcr/`.
The local follow-ups below are not part of this published CI revision.

## Process-observation fixtures and CI preparation after `4ea74ded`

The standalone Mac run at `4ea74ded` failed the stale process-record cleanup
fixture. The fixture required an absent group's legacy record to disappear,
but production deliberately preserves that record when process observation is
unavailable. Darwin's [process-state implementation](https://github.com/apple-oss-distributions/adv_cmds/blob/main/ps/tasks.c)
can emit `?`; the shared identity helper treats such an unknown state as an
incomplete observation. The failed runner did not retain its exact process
snapshot, so this is not a confirmed attribution of that historical failure.

A controlled run of the unchanged test binary passed with the native Linux
snapshot and reproduced the same assertion failure after adding one unknown
state row. The fixture now supplies explicit complete, unknown-state,
missing-observer and failed-command observations. It binds the observer row to
the actual sweep shell and checks the exact `ps` arguments. The production
cleanup command and shared identity helper are unchanged. Record contents,
ages, malformed-record removal, fresh-record preservation, directory checks and
the no-kill assertion remain enforced. The live-group and process-cwd cases
continue to use native `ps` on Linux.

The complete 44-case cache-cleanup module passed locally, with zero failures or
ignores and 1,676 other cases filtered. Fault injection changed only the emitted
command copy: incorrectly permitting deletion after an uncertain observation
made all three new uncertainty cases fail, and incorrectly retaining records
after complete observation made the existing absence case fail. This admits
the valid safe-retention behavior that the old fixture rejected while explicitly
rejecting unsafe deletion and unjustified retention. No gate or deadline was
relaxed. Formatting, locked workspace/all-target compiler checks and strict
Clippy passed with source hashes held fixed.

The unchanged official UBS scan returned exit zero for this Rust file, with
zero critical, 151 warning and 72 informational records. Four records intersect
added lines: the intentional test assertion is a warning, and the shebang path
and two `printf %s` literals are arithmetic false positives. The complete
inventory and review are retained; no rule or suppression changed.

Core E2E preparation now builds only the debug daemon and worker before the
unchanged Cargo integration-test command builds `target/debug/rch` with its
`true-e2e` feature. The original three package selections remain to preserve
dependency feature resolution. All release binaries, test selections, runtime
profiles, concurrency settings, the 30-minute job ceiling and assertions are
unchanged.

Cargo's preparation graph contains 397 units instead of 729. Dependency
features are unchanged; two crates used only by build tools now receive Cargo's
ordinary host debug-info setting, accounting for 38 recursive fingerprint
differences. No compiler setting was manually changed. This is removal of
redundant preparation, not a paired performance result or proof of CI duration.

Main CI's x64 macOS release build now uses `macos-15-intel`, matching the native
Intel runner already used by workspace tests and Test Release. Its prior log
identifies an ARM host cross-compiling for x64 and records the enforced
30-minute cancellation. This does not establish why compilation took too long.
The target, whole-workspace build command, release profile, compiler settings,
artifact paths and deadline are unchanged. A parsed-workflow comparison verifies
that the runner label is the only additional semantic change and that the
validated E2E definition remains identical. Native acceptance and CI duration
still require the next published revision's results.

Local validation used the actual workflow support-binary selection, followed by
the integration-test preparation and exact unchanged Cargo test command. Cargo
emitted only the two support executables in the first step, then freshly built
RCH with `true-e2e` before running all 164 cases successfully, with zero failures,
ignores or filters. The daemon and worker hashes stayed unchanged. YAML and all
core E2E Bash steps passed syntax checks. Source hashes remained fixed; CI's mock
SSH and skip-worker-check settings were retained. A subsequent source-bound
release build and the unchanged complete shell suite also passed: 11 cases
passed, none failed, and installation prompting plus opt-in true E2E skipped.
All 13 discovered cases reached their recorded outcome. Source hashes remained
fixed throughout; a private mount kept this run's target logs separate from
earlier evidence. This local run does not prove the CI job's duration or replace
native Mac acceptance at the next published revision.

Evidence is retained under
`/mnt/hdd-vibe-coding-share/remote-test-local-fallback-claude1/tmp/` in
`rch-stale-record-observation-control-6z9ulvyi/`,
`rch-stale-record-observation-trace-btqy_9zj/`,
`rch-process-observation-fixture-validation-a21cv2g1/`,
`rch-core-e2e-support-target-graphs-l8iasfhf/` and
`rch-core-e2e-support-validation-g6z2vngr/`, with the complete shell run in
`rch-core-shell-support-validation-kjhhpjpz/`. The scanner inventory and review are
in `target/managed-storage-validation-20261009/ubs-process-observation-fixture-report-approved.gsvrgotv/`.

## Completed CI at `f9c7ba62`

Every workflow in this cycle is terminal. The source archive from the PR merge
revision `ea930f7352c76aca00c06cd7ccfda4763829d9dc` independently matches all
1,391 tracked blobs and executable modes at the published feature head.

| Workflow or platform | Final result |
| --- | --- |
| [Main CI](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38195299355), Linux x64 | All 225 workspace harnesses completed: 12,123 passed, zero failed, 31 ignored, zero filtered. Separate nine-case filesystem and 13-case socket selections also passed. |
| Main CI, macOS ARM | All 216 workspace harnesses completed: 10,027 passed, one failed, 23 ignored, zero filtered. The failure was the concurrent breaker-reader fixture's accepted socket returning `EAGAIN`. Both native dependency contract cases passed, exercising the unsupported-target refusal branch. Linux-only M4 serving is not native Mac coverage. |
| Main CI, macOS Intel | Exceeded the 60-minute limit. Completed harnesses recorded 6,346 passes, three failures, three ignores and zero filters. The failures were the same socket fixture, the large ownership-closure fixture's 60-second timeout, and an empty completion marker where `finished` was expected. Remaining targets did not finish. |
| Main CI, Linux ARM | Exceeded the 60-minute limit. Its 215 completed harnesses recorded 10,322 passes, zero failures, 31 ignores and zero filters. The remaining daemon harness was interrupted during cleanup cases with real elapsed-time requirements. This is not a complete pass. |
| Main CI, coverage | Failed with runner annotations explicitly reporting `No space left on device`; the 60-minute limit was not reached. No coverage report or 65% threshold result exists. This evidence does not establish the cause of the earlier revision's exit 143. |
| Main CI, core E2E | All 164 Cargo cases passed, with no failures, ignores or filters. The job then exceeded 30 minutes during shell path-dependency fixture compilation. The shell suite did not complete. |
| [Standalone E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38195299400) | Both platforms passed all 41 scripts. Each inner suite recorded 11 passes and two explicit configuration-dependent skips. |
| [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38195299353) | Protocol and release jobs passed: 36 + 8 protocol cases and 13 release cases, none failed, ignored or filtered. The unchanged 60-sample overhead gate passed at 2.589 ms p95; this single CI measurement is not a paired performance comparison. |
| [Rsync](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38195299425) | 64 passed, zero failed or ignored, 3,309 filtered, using rsync 3.2.7. |
| [Test Release](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38195299395) | All five Unix build/smoke/package jobs passed. Downloaded archives independently passed digest, checksum, file-name and executable-mode checks. Windows retained the deferred compiler errors; aggregate package verification skipped. |

Format, workflow and manifest guards, compiler checks, strict Clippy, docs,
security and benchmark gates passed. Main CI's aggregate failed and its release
stage skipped. Neither canceled platform job is a passing suite. The proposed
additional job-budget increases remain unapplied.

Raw logs, verified ZIPs, source/package proofs, timeout and disk-full annotations,
and parsed complete/partial outcomes are retained in
`/mnt/hdd-vibe-coding-share/remote-test-local-fallback-claude1/tmp/rch-ci-headf9c7ba-monitor-gz1_qfky/`.
Local repairs for the current failures are being validated separately; they are
not part of the published revision described in this section.

## Ownership command, fixture publication and coverage preparation after `f9c7ba62`

The concurrent breaker-reader fixture now explicitly clears nonblocking mode
on its accepted socket before applying its existing five-second read timeout.
Darwin inherits that listener flag; Linux does not. A controlled Linux socket
probe reproduced the inherited-mode failure and verified the same timeout after
clearing the flag. The complete 36-case wrapper binary unit target, formatting,
workspace/all-target check and strict Clippy passed locally for this two-line
fixture change. Fresh native Mac CI is still required.

The daemon-start fixture previously created its completion marker before
writing `finished`. The Intel Mac observed that empty file. The fixture now
closes an exclusively created pending file before atomically publishing the
marker. A controlled pause immediately after opening the writer reproduced the
old empty marker and showed that the candidate remains unpublished until the
bytes are complete. Socket behavior, descriptor assertions and all existing
fixture deadlines are unchanged.

The ownership repair command now stores its quoted root list once as shell
positional arguments. Detection, repair and verification each iterate the same
arguments. The existing large-closure fixture still uses all 4,097 roots,
including its final shell-sensitive path, all five outcomes, the 20-second
operation limit and the 60-second outer limit. A timeout now reports its mode,
state and completed privileged-call records. The source-shape assertion follows
the new representation and additionally requires all three sweeps; outcome,
payload, quoting and exit-code assertions remain enforced.

A local command-format prototype ran the incumbent and candidate scripts in
the same invocation, reversing their order in the second pass. All 40 shell
executions preserved status, stdout, stderr and privileged-call sequences. The
payload fell from 2,532,434 to 844,739 bytes. GNU Bash 5.2 cases took 2.54–4.32
seconds for the incumbent and 0.69–0.86 seconds for the candidate; `dash` showed
mixed small changes around 0.03–0.09 seconds. These observations concern this
local shell fixture. They do not establish native Mac timing or fleet build
speed. The rebuilt Rust binary emitted the exact candidate bytes for the same
fixture inputs, and its embedded marker fixture matches the controlled
publication prototype exactly.

Final focused validation held all source hashes fixed. The full SSH unit
module passed 59 cases with 3,314 filtered; the full daemon-update module passed
18 with 3,355 filtered. Neither had failures or ignores. The same-input command
check repeats one of those SSH cases and is not additional unique coverage.
Workspace test preparation compiled successfully, and formatting, locked
workspace/all-target compiler checks and strict Clippy passed. Preparation alone
is not a full-workspace runtime pass.

Coverage preparation now owns one fresh instrumented target and uses a separate
registry-only cache key. It builds binaries and tests together under the test
profile, then executes the original workspace/rich-ui test selection. Recursive
Cargo unit-graph comparison matches all 1,026 coverage units, including profile
settings and dependency edges; the sole additional unit is the standalone
`rch-wkr` binary required by fixtures. The reports and 65% line threshold remain
unchanged. YAML and all coverage-step Bash syntax checks passed. The 60-minute
workspace/coverage and 30-minute core E2E ceilings remain unchanged. A separate
core E2E preparation proposal did not match its test graph and was not applied.

An intermediate local coverage preparation built successfully, then was
intentionally interrupted with exit 130 during test compilation so these final
changes could be validated together. No tests had started and no coverage
report or threshold result was produced. Its source hashes remained fixed.
The 75 build-only raw profiles were preserved by rename in a new report
directory, with names, hashes and inode checks recorded. The final local run
reuses compiler artifacts with an empty initial raw-profile set. Its disk
allocation includes retained intermediate artifacts and cannot establish the
fresh hosted runner's capacity requirement.

The final preparation command passed, and the subsequent coverage test command
reused it without recompiling: Cargo's freshness check took 7.75 seconds.
The test run then stopped with exit 101 after 86 completed harnesses: 2,098
passed, one failed, five ignored and zero filtered. The failure was
`uploaded_source_is_read_only_while_runtime_and_artifact_output_remain_writable`:
capturing the installed 1.8 GB toolchain exceeded its unchanged 30-second
request deadline on the ext4 image backed by NFS. No coverage report or 65%
threshold result was produced.

A control used the same instrumented three-case fixture and installed toolchain
on local ext4. Two cases passed; the same case reached execution but still
expired at the deadline. A subsequent comparison on warm local ext4 completed
all three cases for both ordinary and instrumented binaries, in 24.70 and
24.13 seconds respectively. Those diagnostic passes do not replace either
failed attempt or establish a full coverage pass. Inputs, deadlines, assertions
and concurrency are unchanged. The CI runner's complete test/report/threshold
result remains required; there is no established source defect from these
timing controls alone.

The unchanged official UBS scan returned exit 1: two critical, 467 warning and
448 informational records across the three changed Rust files. Both critical
records are in unchanged code: the staging filename timestamp and an
intentional execution-state shell probe in a Linux-only unit fixture. All eight
added-line records are informational: six notices for three small reads used
only after a fixture timeout, and two path-literal/division false positives.
The complete inventory and review are retained; this is not a green scan, and
no scanner rule or suppression changed.

Evidence under the native evidence root above is in
`rch-ownership-shell-diagnostic-tvppo7ek/`,
`rch-completion-marker-control-_fb3bt6v/`,
`rch-breaker-fixture-blocking-pcnl_sia/`,
`rch-coverage-preparation-graphs-lboj1y2n/`,
`rch-coverage-workflow-validation-azrunfsu/`, and the interrupted
`rch-coverage-single-tree-ke6ktpoo/`. Completed focused validation is in
`rch-ownership-marker-final-validation-mmlwj2en/`; the failed full-coverage
attempt is in `rch-coverage-final-preparation-sqmue3ma/`. Both controls and their
isolated raw profiles are in `rch-artifact-toolchain-local-disk-control-8emy9knu/`
and `rch-artifact-toolchain-profile-comparison-vibqv_um/`. The UBS report and review are in
`target/managed-storage-validation-20261009/ubs-ownership-marker-final-report-approved.uglzw61h/`.

## Buffered edge reads and concurrent breaker readers after `2960380a`

The published revision's [RABS release job](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38185933990/job/114608132473)
passed replay, both dependency cases, the corrected Cargo worktree fixture and
doctor, then failed the unchanged wrapper overhead gate: p95 10,597 microseconds
against the strict 10 ms limit. All 60 observations are retained; six were at or
above 10 ms. This result remains a failure.

A separate native Linux trace found 4,912 one-byte socket reads across three
requests. The daemon now keeps an 8 KiB buffered reader for the whole connection,
including hello, requests, completion and capture acknowledgment. Frame size,
request count and absolute deadline checks are unchanged. Writes use the same
underlying socket without discarding prefetched input. A candidate trace of
three requests recorded six 8 KiB receive requests and no one-byte receive
requests, with three matching decision receipts. Traced timings are excluded
from the performance comparison.

The first local Cargo worktree check after this change failed with two of three
verifications. A diagnostic reproduction then showed `leaf` receiving `EAGAIN`
from the breaker lock before either concurrent dependency connected to the
daemon. The wrapper previously took an exclusive lock just to read its breaker
decision. It now permits shared decision reads; updates remain exclusive. A
probe releases its shared lock, acquires exclusive ownership without waiting,
re-reads the current state, and publishes its reservation before connecting.
Contended writers and unavailable locks still fall back locally. No gate,
assertion, concurrency setting, timeout or dependency pin was relaxed.

The same-invocation release comparison kept the incumbent daemon and wrapper
live alongside both candidate combinations. Each used five warmups and 60
observations, with forward/reverse alternating order. All three produced 65
identical normalized shadow decisions and successful compiler passthroughs.

| Local release variant | p95 | Median | Daemon CPU during measured requests |
| --- | --- | --- | --- |
| Incumbent daemon and wrapper | 5.436 ms | 4.874 ms | 0.14 s |
| Buffered daemon and incumbent wrapper | 3.887 ms | 3.211 ms | 0.04 s |
| Buffered daemon and shared-reader wrapper | 3.824 ms | 3.233 ms | 0.04 s |

The combined p95 is 29.65% lower in this controlled overhead fixture. Wrapper
CPU stayed approximately 0.164 s per variant; daemon CPU uses Linux process
counters with 10 ms resolution. This is not a fleet throughput or real-project
build speed claim. The separate, unchanged release gate passed at 4.038 ms p95.
The subsequent CI result is recorded above at `f9c7ba62`.

A black-box release comparison also held a real shared file lock in a separate
process: the incumbent produced no daemon consultation, while the candidate
consulted and preserved compiler status and breaker state. Both refused to
consult under an exclusive lock or to reserve an open-breaker probe while a
reader remained. After release, the candidate probed and reset the state to
closed. These controls preserve the breaker's exclusion semantics.

Local checks, with source hashes held:

- Edge unit tests: 32 passed, zero failures/ignores, 492 filtered; real-daemon
  edge, output-capture and serving targets: 8 + 5 + 1 passed, none filtered.
  These source-identical edge results precede the wrapper lock change.
- Final wrapper unit and socket protocol targets: 36 + 8 passed, none filtered.
- Existing seeded daemon-kill, sustained-death recovery and 200-consult soak
  cases: three passed, none ignored or filtered. This native run is separate
  from the interrupted CI coverage compilation.
- Real Cargo worktrees and registry/Git dependency targets: 1 + 2 passed in
  debug, and 1 + 2 passed in release, none filtered.
- Runtime ownership policy: six passed across 352 Rust source files. Workspace
  all-target check, strict Clippy and formatting passed.

Evidence is retained under
`/mnt/hdd-vibe-coding-share/remote-test-local-fallback-claude1/tmp/` in
`rch-live-consult-profile-s0sw_ux8/`,
`rch-buffered-edge-validation-wwsi7o2i/`,
`rch-buffered-shared-breaker-validation-_l34o0pc/`, and
`rch-buffered-breaker-comparison-7mnym7jm/`. The initial failed test, diagnostic
trace, incumbent binaries, all measurements, CPU observations and matching
decision receipts are retained. Published-head CI logs, verified artifacts and
parsed outcomes are in `rch-ci-head296038-monitor-n_2bg8zx/` under the same root.
The subsequent source-identical chaos checks are in
`rch-breaker-chaos-validation-lqc1gj6v/`.

The unchanged official UBS scan returned exit 1: one critical, 196 warnings and
97 informational findings. Its critical finding calls the existing timestamp
used in a staging filename a security token; the statement is unchanged from
HEAD, and `create_new`, mode 0600 and exclusive atomic publication guard the
file. All 24 added-line findings were reviewed: 17 enforcing test assertions and
seven contextual informational notices. This is not a green UBS result. No
rule changes or suppressions were introduced. Full findings, source/scanner
hashes and review are in
`target/managed-storage-validation-20261009/ubs-buffered-breaker-report-approved.7yj75k7r/`.

At `2960380a`, both standalone E2E platforms passed all 41 scripts; their inner
suites each recorded 11 passes and two explicit skips. The 50-sample hook mean
was 1.165 ms on Linux and 8.499 ms on macOS, under the existing 10 ms gate.
Rsync passed 64 cases with zero failures/ignores and 3,309 filtered. All five
Unix release jobs passed; downloaded packages independently verified artifact
digests, archive checksums, names and executable contents. Windows still failed
the known unavailable stream-helper compilation, so aggregate package
verification was skipped. Core E2E passed 164 Cargo cases with no failures,
ignores or filtered cases, plus 11 shell cases with two explicit skips. Its
existing non-enforcing historical timing report recorded four threshold
exceedances, 20 comparisons within threshold and nine new entries; the green
E2E job is not a pass for that advisory report. Main workspace tests and coverage
have separate verdicts: coverage ended after a runner shutdown and exit 143
during the chaos fixture's nested daemon build. Its 95 completed harnesses
recorded 2,163 passes and seven ignores without a reported assertion failure;
the interrupted target and remaining targets did not complete. No coverage
report or threshold result was produced, and the shutdown cause is
unestablished. Linux x64 completed all 225 workspace harnesses: 12,118 passed,
zero failed, 31 ignored and zero filtered. The separate nine filesystem and 13
socket selections passed and repeat workspace coverage. Linux ARM and both Macs
exceeded the unchanged 60-minute ceiling, confirmed by GitHub annotations. Each
completed 95 harnesses before interruption in `chaos_faultinjection`: Linux ARM
recorded 2,163 passes and seven ignores; each Mac recorded 2,013 passes and two
ignores, with no reported failures or filters. All three stopped during the
fixture's nested daemon compilation. Neither Mac reached the new dependency
target-contract cases, so their native execution remains unverified. Main CI
and its aggregate check failed; its release stage skipped. Every workflow at
this revision is terminal.

## Daemon preparation in workspace fixtures after `2960380a`

The timed-out jobs had already compiled the workspace test graph, then the
first wrapper fixture invoked a package-only production daemon build. Cargo's
unit graphs show different dependency features, including Asupersync, so this
invocation compiled a second dependency graph. This is a fixture preparation
problem; the timeouts are not passing test results.

The five wrapper fixtures now share one preparation helper. Workspace tests
pass their existing package/feature selection through
`RABS_TEST_WORKSPACE_ARGS`; the helper asks Cargo to prepare that same test
graph with `--no-run`. Coverage passes its existing `--workspace --features
rich-ui` selection. The helper always invokes Cargo and requires success, so an
existing daemon file alone cannot establish freshness. Scope validation refuses
excluding `rabsd`, changing the manifest, target, profile or build directory,
or executing a nested test run. Parent-package metadata is removed from the
child environment while compiler flags, instrumentation and target placement
remain. Standalone fixtures retain a production daemon build in their debug or
release profile. Release selection now applies to all five helpers; previously
only the overhead fixture passed `--release`, while the others built debug and
then selected a daemon beside the release wrapper. This closes that freshness
gap for standalone release execution.

The existing assertions, samples and deadlines are unchanged. The first local
run passed chaos, worktrees, dependencies and debug overhead, but the wrapper
contract suite failed with Linux `ETXTBSY` while directly executing its freshly
copied compiler script, before the wrapper was invoked. A diagnostic trace of
three execution cases passed but showed a fork overlapping another test's open
executable writer. A separate controlled native probe reproduced errno 26 while
a child retained a CLOEXEC writer and restored exact exit status, raw-argument
stdout and stderr after closure. The original failure's retaining PID was not
captured.

Executable writes/copies and all wrapper-fixture process launches now share a
short preparation guard, including the helper's Cargo spawn. The guard ends
after spawn, before waiting for any child; runtime tests still run concurrently.
The 14 launch-site changes preserve arguments and I/O settings, including all
seven output calls' original null-input/piped-output defaults. There are no
retries or discarded failures. No dependency version or CI ceiling changed.
This is build-setup maintenance and has no separate performance claim.
The unit graphs and closure comparison are retained in
`rch-nested-cargo-graphs-lujkabaa/` under the native evidence root above. Final
native Linux validation is retained in
`rch-workspace-daemon-preparation-final-5w5_4mne/`, with source hashes held:

- The workspace test graph compiled successfully; compilation alone is not a
  full-workspace runtime pass.
- All five complete fixture targets passed: chaos three, worktrees one,
  dependencies two, debug overhead one and wrapper contracts ten. None failed,
  was ignored or filtered. Each invoked Cargo and reused the prepared workspace
  without recompilation; preparation checks took 1.53 to 2.17 seconds.
- Standalone release worktrees, dependencies, overhead and wrapper targets
  passed 1 + 2 + 1 + 10 cases with no failures, ignores or filters. The unchanged
  enforcing overhead gate passed at 4.691 ms p95 across all 60 retained samples.
  The separate debug 5.205 ms observation is measurement-only.
- Locked workspace/all-target compiler and strict Clippy checks, plus formatting,
  passed. Earlier edge/protocol results above bind to the unchanged production
  files; repeated profiles are not additional unique coverage.

The subsequent native Mac and published-head results are recorded above. The first
failed local run is retained in
`rch-workspace-daemon-preparation-lxmuks02/`, the trace in
`rch-wrapper-script-exec-boundary-f_zpbaj4/`, and the deterministic writer
control in `rch-script-writer-control-__d1aefo/`. The first run also retains a
log-reader error: it initially counted the worktree fixture's intentional stock
compilation as a workspace rebuild. The corrected reader examines only Cargo's
preparation phase; all later fixture compilation output remains visible.

The final nine-file UBS scan returned exit 1, one critical, 360 warnings and
223 informational findings. Its critical record is the unchanged staging-name
false positive discussed above. All 31 added-line notices were reviewed: test
assertions, synchronous file operations, path-literal notices and a test-only
error assertion. No suppression or scanner-rule change was made. Source/scanner
hashes, the full result and review are retained in
`target/managed-storage-validation-20261009/ubs-daemon-preparation-final-report-approved.ojuunlkq/`.

## Fixture warmup and native target contracts after `26cda270`

The RABS release worktree test at `26cda270` exposed a fixture race. Toolchain
hashing completed during a Cargo build: `leaf` and `side` were still warming,
while `middle` executed and committed. The fixture entered its three-commit
assertion as soon as it observed any commit, so this valid partial transition
failed with one commit instead of three. The primary application's separate
`CARGO_PRIMARY_PACKAGE` refusal was expected and was not the cause.

The fixture now warms the compiler through an independent registry dependency
whose key is not one of the three measured keys. It then requires all three
real dependencies to commit in the same first build. Both subsequent verifying
builds, the three served hits with no admitted executions, comparison against
stock Cargo's bytes, the fresh rebuild and member-only edit checks remain.
The 600-second warmup bound is unchanged. A valid mid-build warmup transition
no longer contaminates the measured keys; a missing measured commit,
verification, served result, matching artifact or fresh rebuild still fails.
This corrects setup and does not claim a performance improvement.

Mac ARM reached the path-corrected fixtures and exposed the next contract:
`derive_dependency_output_declarations` explicitly supports only x64/ARM64
Linux GNU/musl targets. Its generic naming helper knows Darwin filenames, but
that is not a serving-output closure proof. Both Mac requests correctly refused
with `LIVE_DEP_OUTPUTS: UnknownTargetFamily("aarch64-apple-darwin")`; the former
`LIVE_DEP_CWD` path mismatch was absent.

The two cases now have names describing the native target contract. The actual
compiler host is checked against the four supported triples; those targets
retain their original positive serving bodies. Other Unix targets must observe
the exact unsupported-target refusal, with no admitted execution or serving,
and successfully compile locally. The fallback's stdout, diagnostics after
output-directory normalization, library bytes and dep-info are compared with
stock rustc. Other refusal reasons or unexpected decisions fail. The production
target policy and all existing deadlines remain unchanged. This verifies a
refusal contract; it does not add or prove Mac dependency serving.

Final local validation passed both complete Linux dependency cases, all 11
selected output-declaration contract cases, M4 in both debug and release, and
workspace/all-target check, strict Clippy and formatting. No runtime case was
ignored; the declaration selection filtered 324 unrelated cases. Source hashes
were held throughout. A byte comparison confirms the unchanged Linux bodies,
M4 application source and downstream assertions, and unchanged production
policy. The unsupported-target branch was compiled but has not run on an
unsupported native target yet. Evidence is under
`rch-final-target-contract-tbemelzo/`; the preceding M4-only
debug/release control and fresh release binary build are retained under
`rch-m4-isolated-warmup-sl5mc7u8/`. Repeated cases are not extra unique coverage.

The unchanged official UBS scan of both Rust files returned exit zero, zero
critical, 117 warning and 85 informational records. Its 16 added-line records
were reviewed: required assertions, an existing pattern of unique fixture-name
allocation, intentional synchronous fixture setup and a path-slash false
positive. Four unlocated notices describe scanner phases not evaluated. The
separate Cargo checks above are not attributed to UBS. Scanner/source hashes,
raw findings and review are in `ubs-final-fixture-contract-report-approved.pge6list/`.
No scanner suppression, policy change or test ignore was introduced.

All workflows at the published `26cda270` baseline are terminal:

- [Main CI](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38178630833)
  concluded cancelled; its aggregate check failed and release builds skipped.
  Both Linux architectures completed 225 workspace harnesses with 12,118
  reported passes, zero failures, 31 ignores and zero filters each. Compiler,
  strict Clippy, docs, formatting, workflow/manifest/security checks and
  benchmarks passed. Coverage completed reporting and the unchanged 65% gate
  at 86.87% lines (321,817/370,437) in the verified HTML artifact.
- Both Mac jobs reached the 60-minute limit. ARM completed 148 harnesses with
  2,775 passes, the two target-contract failures above, three ignores and zero
  filters, then stopped in the RCH binary harness. Intel completed 43 harnesses
  with 926 passes, no failures, two ignores and zero filters, then stopped in
  the protocol library harness. Neither incomplete harness nor later targets
  has a completed verdict. Both Macs passed the earlier runtime boot case and
  separate nine-case filesystem and 13-case socket selections, including the
  native peer-close assertion. Focused cases are listed separately.
- Core E2E completed within its unchanged 30-minute limit: 164 reported Cargo
  passes with 92 archived capability-skip records, then all 13 shell outcomes
  (11 passes and two opt-in skips). Install prompts and the separate true-E2E
  shell case require their explicit enabling variables. No outcome is missing.
- [Standalone E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38178630840)
  passed all 41 script statuses on both platforms, with 11 inner passes and
  two skips each. Fifty hook calls averaged 1.45431576 ms on Linux and
  9.81096002 ms on Mac against the unchanged 10 ms gate. Both resource records
  are valid; the Mac children used 0.244579 user and 0.160236 system CPU seconds
  across those calls. This pass does not explain the earlier 12.77520082 ms
  loss. Structured skip records remain qualified separately from script status.
- [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38178630841)
  passed 42 protocol and nine release cases, then failed the M4 warmup race
  described above. The 443,456-byte wrapper passed its size gate. Doctor and
  enforcing wrapper timing were not reached, so there is no p95 verdict here.
- [Rsync](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38178630853)
  passed 64 selected cases and compiler/lint checks. All 1,390 archived Git
  blobs and modes match the published head.
- [Test Release](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38178630839)
  passed all five Unix jobs, and every ZIP digest, archive checksum, executable
  inventory and mode was independently verified. Windows retained its deferred
  missing-import and two type errors; aggregate verification was skipped.

The complete or explicitly partial records, annotations and verified artifacts
are retained in `rch-ci-head26cda-monitor-541z21t1/`. Keep PR #2 draft pending
native execution of the revised fixtures and complete Mac workspace results.
The proposed workspace/core-E2E budgets remain unapplied; core E2E completed at
the existing limit in this run. The separate local full-toolchain deadline,
Windows deferral and operator deployment/live SSH/Nexus boundaries remain.

## Hook timing diagnostics after `a8363011`

The native Mac standalone E2E job at `a8363011` failed the unchanged hook
startup gate: its 50 measured invocations averaged 12.77520082 ms against the
10 ms limit. The complete sample sequence and failed result are retained in
`rch-ci-heada836-monitor-0z476whg/`. Forty other script statuses passed; the inner
suite had ten passes, one failure and two skips. The four hook cases after the
timing assertion were not reached. Linux completed all 41 script statuses,
with 11 inner passes and two skips, and averaged 1.43784872 ms for its hook
samples. Capability and pending-scenario skips remain separate from script
status; these results do not prove live-worker acceptance.

The Mac timing cause is still unknown. The RCH, shared-library and telemetry
source, dependency manifest/lockfile, toolchain, Cargo configuration, hook test
and standalone workflow are unchanged between `18e2235f` and `a8363011`.
The earlier Mac pass does not explain this failed observation.

The follow-up adds child CPU time, context-switch counts, load averages and
platform information after the original 50 measurement windows have finished.
Removing that one diagnostic block produces the byte-identical published
script. Invocation count, subprocess arguments, clock, mean calculation and
10 ms assertion remain unchanged. This is diagnostic work, with no production
performance change or improvement claim.

Local validation passed shell syntax, embedded Python compilation, formatting,
workspace/all-target check and strict Clippy, with non-document source held.
The actual measurement block also ran all 50 invocations against the verified
published Linux musl RCH executable and emitted valid resource JSON; this
checks the diagnostic on Linux, not Mac performance acceptance or the complete
shell suite. The unchanged official UBS scanner returned zero critical,
warning and informational records for the changed shell file. Evidence is in
`rch-hook-resource-validation-4ukxdf_4/`,
`rch-hook-resource-control-oxvf6tqy/` and active-worktree validation directory
`ubs-hook-resource-report-approved.tno8kbkp/`. Previous broader scanner findings
and failed CI measurements remain recorded below.

All five workflows at `a8363011` are terminal:

- [Main CI](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38173588688)
  concluded cancelled; its aggregate check failed and release builds skipped.
  Linux x64 completed 225 workspace harnesses with 12,118 reported passes,
  zero failures, 31 ignored cases and zero filtered cases. Compiler, strict
  Clippy, docs, formatting, manifest/workflow/security checks and benchmarks
  passed. Coverage completed reporting and the unchanged 65% gate at 86.87%
  lines (321,801/370,437) in the verified HTML artifact.
- Linux ARM and both Macs reached the 60-minute job limit during the chaos
  fixture's nested daemon build. Each completed 95 preceding harnesses with
  no recorded failures or filtered cases: Linux ARM had 2,163 reported passes
  and seven ignores; each Mac had 2,013 passes and two ignores. Their next
  harness and all later targets have no completed verdict. In particular,
  neither Mac reached the corrected live-dependency cases. Both Macs passed
  the earlier runtime boot case, which does not explain the preceding Intel
  timing failure. All four platforms passed the separate nine-case filesystem
  and 13-case socket selections; these are repeated coverage.
- Core E2E completed 164 reported Cargo passes with 92 archived capability-skip
  records. Its shell suite completed three passes and one skip, then reached
  the 30-minute limit during path-dependency fixture setup; nine outcomes are
  missing. GitHub's final annotations confirm all four job-limit cancellations.
- [Standalone E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38173588755)
  failed on the Mac hook timing assertion described above. Linux passed; the
  aggregate report job succeeded, which does not make the workflow a pass.
- [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38173588683)
  passed all 42 protocol and 13 release cases, with no ignores or filters.
  Wrapper size was 443,456 bytes and its enforced p95 was 6,960 microseconds
  against the unchanged strict 10 ms limit. All 60 original-order samples
  were retained and reproduce that percentile. This pass does not establish
  the cause of the earlier 10,880-microsecond loss or a performance improvement.
- [Rsync](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38173588699)
  passed 64 selected transfer cases and its compiler/lint checks. All 1,390
  Git blobs and modes in the CI source archive match `a8363011` exactly.
- [Test Release](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38173588692)
  passed all five Unix build/version/package jobs. Every downloaded ZIP digest,
  internal archive checksum, executable inventory and executable mode was
  independently verified. Windows retained the deferred import/type errors;
  aggregate verification was skipped.

Terminal metadata, annotations, raw logs, verified archives and complete or
explicitly partial outcome records are retained in
`rch-ci-heada836-monitor-0z476whg/`. Keep PR #2 draft. The Mac hook timing cause,
native acceptance of the path correction, workspace/core-E2E completion gaps,
and the separate local full-toolchain deadline remain open. The proposed
workspace/core-E2E budget changes remain unapplied pending the user's decision;
deployment and live SSH/Nexus acceptance remain with the operator.

## Live-dependency fixture path correction after `18e2235f`

The two live-dependency warmup assertions wait for an admitted compile while
filtering `shadow` decisions from their progress trail. A refused request can
therefore end in the same 600-second assertion as a genuinely unfinished
toolchain probe. The wrapper records its physical current directory, while the
fixture formerly put the lexical `/tmp` path in `CARGO_MANIFEST_DIR`. When that
temporary root is a symlink, these paths differ and the existing exact working
directory guard returns `LIVE_DEP_CWD`.

A controlled run at the published `18e2235f` source used the actual daemon,
wrapper and August compiler, with one owned package reached through a symlink
and through its physical path. The alias requests compiled locally but the
daemon logged `shadow` / `LIVE_DEP_CWD`; toolchain hashing had already completed
in about two seconds. Requests through the physical path reached `execute`,
`committed` and `verified` with unchanged binaries. The complete decision log,
request environments, compiler outputs and source/executable hashes are retained
under `rch-warmup-path-control-j9ltmujn/`. The native macOS ARM run at `18e2235f`
then reproduced exactly this refusal in both failing cases: the request's
`CARGO_HOME` and fixture roots used `/tmp`, while the daemon's `LIVE_DEP_CWD`
detail named the physical `/private/tmp` package directory. The diagnostics
establish the cause of those two current native failures.

Both fixtures now create their short temporary directory under the canonical
physical `/tmp` parent. The correction admits a fixture whose declared package
directory equals its actual compiler cwd. Requests with mismatched directories
remain refused by unchanged production code. Every compile, commit, verify,
serve, byte-equality and negative-result assertion remains, as do both 600-second
deadlines. The bounded failure diagnostics from `18e2235f` remain available.

Both complete live-dependency cases passed locally with no ignores or filters.
Locked workspace/all-target check, strict Clippy and formatting also passed,
with non-document source hashes held. UBS returned exit zero, zero critical,
78 warning and 59 informational records. Its four added-line informational
records concern literal path slashes and intentional filesystem setup; the
scanner and rules were unchanged. Evidence is under
`rch-warmup-physical-fixtures-fxqvegwz/` and active-worktree validation directory
`ubs-warmup-physical-report-approved.nhy6xh2j/`. Native Mac acceptance is pending.

The completed native workspace jobs at `18e2235f` establish the preceding
repair batch's results. Linux x64 completed 225 harnesses with 12,118 reported
passes, no failures, 31 ignored cases and zero filtered cases. macOS ARM
completed 216 harnesses with 10,021 passes, the two path-mismatch failures above,
23 ignored cases and zero filtered cases. All 13 socket regression cases passed
in the early selection and again in the full command, including the actual
Darwin peer-close result. The three native Mac TLS targets passed 23, five and
three cases. Canonical-root, prerequisite, shim and transfer fixes passed;
unsupported raw-filename operations retained explicit refusal/skip records,
and the non-Linux worker cleanup case proved refusal rather than reaping.
Repeated focused cases are not additional unique coverage. Evidence is in
`rch-ci-head18e223-monitor-y5hmt4mx/`. All five workflows at this revision are
now terminal; the additional results are recorded below.

The same revision's RABS release job passed its build and 443,456-byte wrapper
size gate, and 12 replay, compiler, Cargo-worktree and doctor cases. Its enforcing
wrapper timing case failed at 10,880 microseconds p95 against 10 ms. The log
contains only the aggregate percentile, so the shape and cause of this loss are
not established. The failed verdict remains in `rabs-release-outcomes.json`.
No performance improvement is claimed from earlier passing hosts or revisions.

The timing fixture now retains its existing 60 observations in original order
after all measurement intervals finish and includes them in the evidence JSON.
The five warm-ups, measured command, clock, percentile index, release/debug
policy and strict 10 ms assertion are unchanged. Local debug execution verified
that the JSON contains exactly 60 samples and that its reported p95 matches
those observations; debug remains measurement-only and does not establish
release acceptance. Workspace/all-target check, strict Clippy and formatting
passed with source hashes held. The final two-file UBS scan returned exit zero,
zero critical, 82 warning and 64 informational records; its four added-line
notices concern the canonical fixture paths, and no scanner rule changed.
Evidence is under `rch-wrapper-timing-evidence-zttpm8oo/` and
`ubs-wrapper-evidence-report-approved.jzg32_dn/` in the validation locations.

Core E2E at `18e2235f` passed 164 reported Cargo cases with 92 archived
capability-skip records. The shell phase completed three passes and one skip,
leaving nine outcomes absent when the job reached 30 minutes. GitHub's final
annotation confirms that limit, rather than an observed shell assertion failure.
The proposed core-E2E budget increase remains unapplied.

macOS Intel also recorded an independent failure:
`daemon_runtime::tests::boots_runs_and_shuts_down_obligation_clean` measured
109 ms against its unchanged strict 100 ms boot limit. Its clean-obligation
and four-subsystem checks passed before that assertion. The runtime code,
dependency manifest and lockfile are unchanged from the preceding revision;
the timing cause is not established. Failure messages now include the full
already-measured receipt and available parallelism. Those expressions run only
after a timing assertion fails, and neither boot nor shutdown measurement or
limit changes. Production runtime code remains byte-identical.

The final local follow-up passed all 106 Linux runtime-library cases at default
test parallelism, with no ignores or filters, plus workspace/all-target check,
strict Clippy and formatting. These Linux results do not resolve the native
Intel timing observation. The three-file UBS scan returned exit zero, zero
critical, 107 warning and 69 informational records. Its five added-line records
are the four fixture-path notices and the unchanged boot assertion with expanded
failure text. No scanner or gate change was made. Source-bound evidence is in
`rch-final-followup-validation-nhflxy_q/` and
`ubs-final-mac-followup-report-approved.u2uetg3z/`.

Final CI results at `18e2235f`:

- [Main CI](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38167414105)
  concluded cancelled; its aggregate check failed and release builds skipped.
  Both Linux architectures completed all 225 workspace harnesses with 12,118
  reported passes, no failures, 31 ignores and zero filters. Compiler, Clippy,
  documentation, formatting, workflow/manifest/security checks and benchmarks
  passed. Coverage passed LCOV, HTML/text reporting and the unchanged 65% gate:
  321,796/370,434 lines, or 86.87%, in the verified HTML artifact.
- macOS ARM's complete counts and two diagnosed fixture failures are above.
  macOS Intel completed the separate nine-case filesystem and 13-case socket
  selections, including the native peer-close assertion. Its full workspace
  command completed 12 harnesses with 174 passes and the 109 ms boot failure,
  then was cancelled during the next `rabs-cas` harness. Later targets have no
  verdict. GitHub's terminal annotation confirms the 60-minute job limit.
  The full Intel job-log download returned `BlobNotFound`; the verified artifact
  retains all three test logs, including the failure and partial workspace output.
- [Standalone E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38167414088)
  passed all 41 script statuses on both Linux and macOS. Each inner suite has
  11 passes and two skips; all 50 hook samples were retained, with means of
  1.474/6.954 ms below 10 ms. The formerly failing native saved-time scenario
  passed. Separate JSONL audits retain 20 Linux and 13 Mac skip records, including
  missing capabilities and pending remediation scenarios; duplicates are not
  unique test outcomes, and the script pass does not establish those behaviors.
- [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38167414112)
  passed its separate 42 protocol cases, but the release job failed the 10.880 ms
  wrapper gate described above after 12 other release cases passed.
- [Rsync](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38167414151)
  passed all 64 selected transfer cases. All 1,390 archived Git blobs and modes
  match this head.
- [Test Release](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38167414233)
  passed all five Unix package jobs. Uploaded digests, archive checksums, the
  three executable members and their modes were independently verified. Windows
  retained the deferred import/type failures; aggregate package verification
  skipped. Its jobs remain enabled.

The follow-up requires fresh CI. The two fixture paths are corrected, while the
Intel boot and RABS wrapper timing failures remain recorded and unexplained.
The workspace/core-E2E budget proposals remain unapplied, and the separate local
full-toolchain deadline plus operator deployment/SSH/Nexus acceptance remain open.
Keep PR #2 draft. No merge or deployment was performed.

## Native fixture repairs following the complete `8edaa0f0` results

All five validation workflows at `8edaa0f0` are terminal. The source archive
matches all 1,390 Git blobs and modes. Evidence is retained under
`rch-ci-head8edaa-monitor-sh76kdk6/` in the validation base.

- [Main CI](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38160004406)
  passed Linux x64's complete 225-harness workspace command: 12,112 reported
  passes, no failures, 31 ignored cases and zero filtered cases. Coverage passed
  LCOV, HTML, text reporting and the unchanged 65% gate; its archived HTML index
  reports 321,596/370,204 covered lines, or 86.87%. The separate nine-case
  filesystem selection is repeated coverage, not nine additional unique cases.
- macOS ARM completed 216 harnesses with 9,950 reported passes, 68 failures,
  23 ignored cases and zero filtered cases. Its 14 failed targets include 25
  canonical-parent fixture failures, four invalid-filename cases, 27 TLS fixture
  failures, ten RCH/worker-tool cases and two toolchain warmup deadlines. The
  intentionally failing T004 child appears inside one outer harness's output;
  it is not a 69th top-level failure.
- Linux ARM and macOS Intel reached their unchanged 60-minute ceilings during
  the next chaos harness's nested build. Their 95 completed harnesses reported
  2,163/2,013 passes, zero failures and seven/two ignored cases respectively.
  Final GitHub annotations establish the job timeouts. Later cases lack a
  verdict on those platforms.
- Core E2E completed 164 reported native cases with 92 archived capability-skip
  records, then all shell cases with 11 passes and two skips. Compiler, strict
  Clippy, docs, formatting, workflow/manifest/security guards and benchmarks
  passed. Main aggregate acceptance failed and release builds skipped.
- [Standalone E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38160004368)
  passed all 41 Linux script statuses. macOS passed 40 and failed the saved-time
  scenario described below. Both inner hook suites passed 11 cases with two
  skips; macOS's 50-sample hook mean was 7.6895 ms, below the 10 ms gate.
- [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38160004375)
  passed 42 protocol and 13 release-gate cases. Wrapper size was 443,456 bytes;
  enforced p95 was 8,498 microseconds under 10 ms. Rsync passed all 64 selected
  transfer cases. All five Unix package jobs passed and their uploaded archive
  hashes, internal checksums, executable members and permissions were verified.
  Windows retains the deferred compile failure and aggregate package verification
  remains skipped; its jobs are still enabled.

The fixture repair batch preserves the production path guards and test result
assertions. Canonicalizing owned temporary parents fixes `/var` versus
`/private/var` mismatches in the affected RABS fixtures and the real-Cargo artifact
fixture. Production ancestor rejection, source identity, output ownership,
recovery and delivery logic are unchanged.

TLS fixtures now supply an owned OpenSSL request configuration for CA and CSR
generation. The CA extensions occur once; the leaf certificate extensions and
all TLS assertions remain unchanged. A controlled reproduction built the
official OpenSSL 1.1.1w source archive after verifying its published SHA-256.
With the former recipe and a host configuration supplying CA extensions, the
certificate contained duplicate BasicConstraints and the actual RABS receiver
rejected it with `ExtensionValueInvalid`. The owned configuration produced one
extension and the same receiver reached listening. Both recipes succeeded with
the local OpenSSL 3.4.1 control. These are certificate/receiver controls, not
complete TLS delivery or historical-runner attribution. The original macOS
certificate was not archived. Evidence is in `rch-mac-tls-ca-vfg74tfa/`.

Four raw-filename cases now distinguish a native Darwin EILSEQ refusal from
successful raw-filename execution. Only that exact macOS error is admitted;
other errors still fail, and directory enumeration rejects a lossy alias.
The wrapper still executes raw, empty and UTF-8 arguments. OUT_DIR tests retain
distinct byte keys and real capture/replay; dep-info tests retain raw canonical
bytes, actual installation and unchanged CAS bytes through explicitly named
representable worktrees when the filesystem refuses raw names. Such a refusal
prints a capability-skip record and does not prove raw-filename support.

The shim fixture compares with a real process's PATH lookup, avoiding the host
shell's extra tilde expansion. The adaptive-compression argument fixture supplies
the existing pinned rsync capability description while retaining its exact zstd
and compression-level assertions. The real-tar fixture selects actual GNU find,
the worker's existing prerequisite; its production inventory script and archive
selection assertions remain unchanged. macOS CI installs Go 1.25.12, coreutils
and findutils, exposing only GNU timeout under the unprefixed worker command name.
Other BSD tools remain the host defaults. Parsed YAML and shell-syntax checks
confirm that the remaining job matrix, commands, gates and budgets are unchanged.
Native prerequisite installation still requires CI acceptance.

Two positive cleanup cases require the production Linux worker's `/proc` UUID
source and now run on Linux. Non-Linux Unix instead executes the real sweep and
job/pool claim paths, asserts refusal with zero reap counts, and verifies all
owned directories and artifact bytes remain. This changes native Mac collection
from two unsupported positive cases to one refusal case; it does not claim Mac
worker reaping or relax the production platform guard.

At `18e2235f`, the two 600-second toolchain warmup failures remained unexplained.
Their assertion messages added a bounded tail of projected daemon decisions and the
constructed toolchain environment. Full environments, cache keys and unparsed
log contents are omitted. Deadlines and decision assertions are unchanged. This
diagnostic change alone did not resolve or relabel either failed case. The
subsequent path control and fixture correction are described above.

Local validation used August nightly, default test parallelism and one compiler
job, with the non-document source held unchanged throughout. Workspace/all-target
check, strict Clippy and formatting passed. The RABS library and binary harnesses
reported 522 and 49 passes; their eight selected integration targets reported
69 passes and one intentionally ignored child entry. The raw-argument, shim
lookup, real-Cargo artifact, rsync argument and real-tar selections each passed
their one case, and all 54 selected Linux ownership-sweep cases passed. No native
Mac-only subcase or new warmup runtime verdict is included in these Linux counts.

The final unchanged UBS scan of all 21 changed Rust files returned exit 1:
12 critical, 2,943 warning and 2,159 informational records. All critical and 84
added-line records were reviewed. Seven critical records concern nonsecret file
identity comparisons; five concern deliberate test executable/script calls,
including the fixed-name GNU find prerequisite probe. No scanner suppression or
rule change was made, and no green UBS verdict is claimed. Exact source/scanner
hashes, the raw report and review are in
`ubs-mac-fixtures-report-approved.69miun4s/` under the active worktree's validation
directory. Native test evidence is under `rch-mac-native-fixtures-6s1bsr9_/`.

The proposed workspace and core-E2E budget increases remain unapplied pending
the user's decision. The current limits are 60 and 30 minutes respectively.
Keep PR #2 draft: this batch needs current-source native CI, the warmup failures
remain open, and deployment/live SSH/Nexus acceptance belongs to the user.

## Daemon response after peer close, following `8edaa0f0`

The macOS standalone run at `8edaa0f0` completed 40 passing scripts and one
failure in `e2e_bd-zked.sh`. Its first status request succeeded; the following
saved-time structure request returned `RCH-E504` with `Socket is not connected
(os error 57)`. The script did not archive its isolated daemon log. This run's
hook timing gate passed with all 50 samples averaging 7.6895 ms; that does not
explain the preceding revision's timing failure.

The status exchange writes and flushes its request, shuts down its write half,
then reads the response. The daemon can answer `GET /status` and close without
waiting for client EOF. Darwin can report `ENOTCONN` for the later shutdown
after the peer closes, leaving already-buffered response bytes unread. The
[XNU socket implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/uipc_socket.c)
and [Unix-domain disconnect handling](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/uipc_usrreq.c)
support that mechanism; the original CI log does not identify the failing I/O
phase or establish its kernel source version. Attribution of that historical
failure remains an inference until the native controlled case runs on macOS.

The exchange now reads the response after a shutdown-only `NotConnected`
error. It retains that error when the response is empty, propagates write,
flush, other shutdown and read errors, and leaves HTTP/JSON validation with
the existing callers. It never resends a command. Existing connect, send and
response deadlines are unchanged. Stage-specific error context identifies
which operation failed if another status request fails.

Before the behavior change, the regression selection reported five passes and
two failures: buffered valid data was discarded, and invalid UTF-8 was masked
by the shutdown error. The final selection passed 13 cases, including six new
cases and the existing request-EOF and timeout-policy cases. A separate status
consumer test passed. The native socket-pair case orders an actual peer close
before client shutdown using a channel rather than a sleep; it checks response
bytes and a single exact request on every Unix host and additionally requires
the observed `NotConnected` shutdown result on macOS. Its Linux execution
passed; the native macOS assertion is pending fresh CI.

Locked workspace/all-target check, strict Clippy and formatting passed with
the final helper source hash held. The unchanged official UBS scan returned
exit zero, zero critical, 89 warning and 43 informational records. The 21
added-line records were reviewed: 16 intentional test assertions, four
intentional `expect_err` checks and one division false positive on the request
string. No scanner change or suppression was made. The saved-time E2E scenario
passed against freshly built application binaries, including seven reported
saved-time behavior cases with no failures or ignores and 1,792 unrelated cases
filtered. Its daemon, compiler-check and behavior-test logs were exported from
the private temporary filesystem and retained with the executable hashes.
Evidence is retained
under `rch-daemon-half-close-y7y436gv/` and
`ubs-daemon-half-close-report-approved.sqs0uqhw/` in the validation locations.
The saved-time script, daemon API and lockfile remain byte-identical to the
published baseline.

CI now runs the socket regression selection before the complete workspace
command on all four native test platforms. It retains each command's exit
status, attempts the full workspace even after a focused failure and archives
the focused log on failure or cancellation. The new command uses the same
workspace/package selection as the full command. Linux-hosted Cargo unit graphs
for both selections, with and without the existing macOS `rchd` exclusion,
confirm an identical CLI test unit and all its transitive dependencies. The
focused graph omits only the `rabsd` doctest unit; that unit remains in the
unchanged full command. This graph comparison is not native macOS execution.
Repeated focused cases are not additional unique coverage. Every existing
test command, deadline, matrix entry and compiler
setting remains in place.

## Prepared wait and coverage reporting follow-up after `c5618ef6`

The Linux x64 workspace command completed all 225 harnesses: 12,111 reported
passes, one failure, 31 ignored cases and zero filtered cases. Both preceding
failures, the real-Cargo Clippy prerequisite and native force-stop fixture, now
pass. The remaining failure is
`actual_daemon_wait_never_reexecutes_uncertainty_or_reads_a_pre_start_cancelled_delivery`.

Its pre-startup record was compared byte-for-byte after the daemon and wait
client ran. The decoded failure differs in exactly four fields: attempt 1/2,
mode execute/local recovery, recovery origin null/1, and the recovery detail.
The daemon's existing prepared-job drivers independently attempt one local
recovery after acquiring coordinator authority. Missing delivery bytes leave
the operation uncertain; this path opens no worker connection and executes no
compiler. UDS availability does not establish that this startup work is done.

A native control seeded the same uncertain record and launched only the
unchanged daemon. It reproduced all four changes before any wait client ran.
Launching `--job-wait` afterwards returned the expected uncertainty error,
printed no stdout, and left the settled record byte-identical. Delivery and
installation directories remained absent. An initial synthetic control used
incorrect state permissions and was refused; its log is retained. The matching
control used the production store's private directory/file permissions.

The fixture now observes exactly the permitted startup transition within the
same ten-second readiness deadline used for socket setup. It checks the entire
decoded record against the original plus those four expected changes, then
retains the settled bytes for the existing post-client comparison. Child
liveness and the deadline are checked before accepting readiness. This admits
the legitimate independent startup transition that the old fixture rejected;
unexpected startup changes, later wait-side mutations, output exposure, altered
exit codes and unwanted delivery/installation still fail. The original client
and process deadlines and every result assertion remain. Production recovery
and wait code are unchanged.

All three native wait-harness cases passed without ignores or filters. Locked
workspace/all-target check, strict Clippy and formatting passed for the final
source hashes. A separate workspace-selected invocation passed the same three
cases with `serde_json` order preservation enabled, as confirmed by its Cargo
unit graph; these repeated cases are not additional unique coverage. The
unchanged official UBS scan returned exit zero, zero critical,
35 warning and ten informational records. Six added-line warnings are intentional
test assertions; no suppression or scanner change was made. Evidence is retained
under `rch-prepared-wait-startup-kcnssf8k/`, including the final candidate results
in `final/`, and `ubs-prepared-wait-final-report-approved.o1hrzmxw/` in the active
worktree's validation directory.

Coverage at `c5618ef6` completed 224 harness summaries with 12,110 reported
passes, no failures, 31 ignored cases and zero filtered cases, and generated
`lcov.info`. The next step failed because the installed `cargo-llvm-cov 0.9.1`
rejects `--features` on `report`. HTML, summary and threshold acceptance are
therefore absent. The three reporting commands now omit only that invalid
feature selector. Their `--workspace` selection, the unchanged 65% line gate,
and the original instrumented `--features rich-ui --workspace` test command
remain in place.

An independently downloaded official 0.9.1 binary was checked against its release
digest and the [versioned command parser](https://github.com/taiki-e/cargo-llvm-cov/blob/v0.9.1/src/cli.rs).
A two-member workspace control includes a member outside `default-members`
and code enabled by `rich-ui`. Both HTML/text reports retained both members;
LCOV records and counts matched the preceding test run, and profile/executable
hashes did not change during reporting. The exact 65% gate rejected the 40%
control and accepted the 97.14% control. Those are fixture coverage values, not
this repository's coverage. Controls use August nightly with `llvm-tools-preview`
installed and are retained under `rch-coverage-report-cli-swa1vu61/`. Parsed YAML
comparison establishes that only the three report command strings changed.

All five validation workflows at `c5618ef6` are terminal. Their evidence is under
`rch-ci-headc561-monitor-xahwz6dc/`:

- Linux ARM completed 225 workspace harnesses with 12,112 reported passes,
  no failures, 31 ignored cases and zero filtered cases. Both Macs completed
  95 harnesses with 2,013 passes, no failures and two ignored cases, then hit
  their 60-minute job limits during the nested daemon build in the next chaos
  harness. Final GitHub annotations confirm both timeouts. The separate
  nine-case filesystem fixture passed on all four platforms; those repeated
  cases are not additional unique coverage. Mac Intel's complete job-log
  download returned `BlobNotFound`; its verified test-log artifact and final
  timeout annotation are retained.
- Compiler, strict Clippy, docs, formatting, workflow/manifest/security guards
  and benchmarks passed. The main aggregate failed and release builds skipped.
- Core E2E passed 164 reported native cases with 92 archived capability-skip
  records. Its shell phase completed three passes and one skip before the
  separate 30-minute job limit, leaving nine outcomes absent. The unchanged
  advisory comparison flagged CMake 21/529 ms and clang/make 42/842 ms p95
  (stored baseline/current); its `|| true` remains advisory, and no cause or
  performance improvement is established.
- Standalone Linux E2E passed all 41 script statuses, with 11 inner passes and
  two skips. Its 50-sample hook mean was 1.451 ms under the 10 ms limit.
  macOS completed 40 passing scripts and one failed `e2e_test.sh`: its inner
  suite recorded ten passes, one failure and two skips. The failure is the
  unchanged hook timing gate, with all 50 retained samples averaging 12.1627 ms
  against 10 ms. This is a failed gate, not an attributed regression or a pass.
- The macOS output-validation script that failed at `352f025f` now passed all
  14 cases. All eight actual status replies, successful predicate outputs,
  daemon log and durable history were retained on both platforms. This proves
  evidence retention and this run's successful checks; the preceding failure's
  cause remains unestablished.
- RABS passed 42 protocol and 13 release-gate cases; wrapper size was 443,456
  bytes and enforced p95 was 8,078 microseconds under 10 ms. Rsync passed all
  64 selected transfer cases, with 3,303 unrelated cases filtered. All 1,390
  archived Git blobs and modes match the published head.
- All five Unix package jobs passed; uploaded ZIP digests, internal archive
  checksums, three executable members and permissions were independently
  verified. Windows reproduced the deferred import/type errors, and aggregate
  package verification skipped. Its jobs remain enabled.

At that checkpoint, the separate 90-minute workspace proposal remained unapplied
and core E2E retained its 30-minute limit. The complete `8edaa0f0` results above
supersede that checkpoint's pending CI: coverage now passed, and the hook timing
failure did not recur. Full native Mac acceptance, the separate local
full-toolchain deadline and operator acceptance remain open. Keep PR #2 draft.
No merge or deployment was performed.

## CI prerequisites and fixture evidence after `352f025f`

All five validation workflows at `352f025f` are terminal. Linux x64 completed
the entire workspace command: 225 harness summaries, 12,110 reported passes,
two failures, 31 ignored cases and zero filtered cases. The two failures are
addressed by this follow-up:

- The real-Cargo build-directory fixture executes both `cargo test` and
  `cargo clippy`. Workspace-test and coverage runners did not install Clippy
  for the pinned August toolchain. Both jobs now request that component; the
  fixture's actual compiler invocation, artifact checks and assertions remain.
- The force-stop helper correctly refuses a mismatched socket peer before
  sending request bytes. Its fake daemon treated that empty connection as an
  unexpected mutating request and exited. A controlled baseline reproduced
  exit 1 without a signal to either owned process. The fixture now tolerates
  the empty connection; a real status round trip verifies it remains responsive
  after refusal. Unknown nonempty requests still fail, and the production
  force-stop helper is byte-identical.

The 18 cases that failed on macOS ARM at `5352da59` now pass on both native Mac
platforms. Each Mac completed 95 workspace harnesses with 2,013 passes, no
failures and two ignored cases. Linux ARM completed the same 95 harness positions
with 2,163 passes, no failures and seven ignored cases. All three reached the
approved 60-minute ceiling while the following chaos harness compiled its nested
daemon dependency graph. Later harnesses, including wrapper singleflight and
supervision, have no native Mac verdict in this run. The separate nine-case
filesystem fixture passed on all four platforms; its repeated cases are not
additional unique coverage.

Coverage completed 149 harness summaries with 6,349 passes, one failure and
11 ignored cases. Its failure is the same missing Clippy component. LCOV and
the unchanged 65% coverage verdict were not produced. Compiler, strict Clippy,
docs, formatting, workflow/manifest/security guards and benchmarks passed.

The [standalone E2E run](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38147420738)
recorded 41 passing Linux script statuses and 40 passes/one failure on macOS.
Each platform's inner suite recorded 11 passes and two capability skips; the
50-sample hook means were 1.223/5.235 ms under the unchanged 10 ms limit.
The macOS failure is `e2e_bd-2ga8.sh`: the immediate status check after
`hook_no_ansi` did not establish the expected completed build. Its combined
failure message did not distinguish CLI failure from a rejected status value,
and its raw fixture files were outside the uploaded directory. Its cause is
not established. Ten unchanged Linux repetitions passed, as did replay of all
80 captured status checks through jq 1.8.2; this does not support the proposed
numeric-precision explanation or replace the failed native observation.

That script now retains status responses, predicate output, daemon logs and
durable history in the uploaded artifact directory, and reports the two failure
paths separately. Every completion, exit-code and output assertion remains.
This is a diagnostic repair, not a claim that the macOS failure is resolved.

Other terminal results at `352f025f`:

- [Core E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38147420717/job/114497565310)
  passed 164 reported native cases, including 92 archived capability-skip
  records. Its shell suite completed with 11 passes and two skips. The existing
  advisory comparison flagged CMake 21/840 ms and clang/make 42/792 ms p95
  (stored baseline/current). Its `|| true` behavior is unchanged; these are
  retained advisory failures, not an enforced timing pass or a causal diagnosis.
- [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38147420745)
  passed 42 protocol and 13 release-gate cases. Wrapper size was 443,456 bytes;
  enforced p95 was 8,476 microseconds under 10 ms.
- [Rsync](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38147420690)
  passed 64 selected transfer cases and strict checks. All 1,390 archived Git
  blobs and modes match the published head.
- All five Unix [package jobs](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38147420710)
  passed. ZIP digests, archive checksums, the three executable members and their
  modes were independently verified. Windows reproduced the deferred import
  and type errors; aggregate verification was skipped.

Local validation of this follow-up passed the real-Clippy fixture, locked
workspace/all-target check, strict Clippy and formatting. The force-stop suite
passed all 11 native tests against the built CLI, with source and binary hashes
held. Its refusal control now receives a subsequent status response without
signalling either process; a planted mutating request still crashes the fake
daemon. Direct attempts through the installed Python wrapper selected an
overlong temporary path and failed fixture startup. Those failed attempts are
retained; the successful run used `/usr/bin/python3` and verified `/tmp` as the
actual temporary directory, matching the CI context.

The output-validation script passed its 14 native cases. Injected status-command
and predicate failures each produced a failed script and retained all eight
status files plus durable history. A failed artifact-copy control also failed
the script. The unchanged official Bash UBS scan reported zero findings. The
Python scan returned exit 1 with two critical, two warning and 43 informational
records. Both critical records are reviewed false positives on the unchanged
test-driver executable argument; the new-line records are two informational
method notices. Reports, reviews and unchanged scanner identities are retained;
no suppression or all-scope scanner pass is claimed.

Evidence is under `rch-ci-head352f-monitor-61yn44rn/` and
`rch-macos-status-investigation-hqyqunwr/` in the validation base. Scanner reports
are in active-worktree `target/managed-storage-validation-20261009/ubs-status-evidence-report-approved.0tfsnz0t/`
and `ubs-force-peer-report-approved.z2wadirp/`. A one-line proposal to give the
workspace-test matrix 90 minutes passes `git apply --check` but remains unapplied
pending the user's decision. The previously approved limits remain 60 minutes
for workspace tests and coverage, and 90 for standalone E2E. Keep PR #2 draft:
complete cross-platform workspace/coverage acceptance, the unexplained macOS
status observation, the local full-toolchain deadline and operator acceptance
remain open. No merge or deployment has been performed.

## Native macOS fixture follow-up after `5352da59`

The prioritized filesystem fixture passed all nine cases on Linux x64, Linux
ARM64, macOS Intel and macOS ARM64. Both macOS logs explicitly record the
unavailable invalid-filename subcase, while the invalid symlink-target rejection
and long-TMPDIR fixture pass. These focused results are separate from the
incomplete workspace runs; repeated cases are not additional unique coverage.

The macOS ARM workspace run continued through 95 completed harnesses, recording
1,995 passes, 18 failures and two ignored cases before its 30-minute limit.
Those failures expose three fixture assumptions, addressed in this follow-up:

- The jobserver replacement test used invented client descriptor numbers that
  could equal the real worker's descriptors. It now keeps separate real client
  and worker pipes alive, verifies the exact client close list and worker
  installation, and requires that neither worker endpoint is closed.
- Reconnection, shutdown and wrapper-capture fixtures accepted sockets from
  nonblocking listeners and immediately used blocking I/O. Darwin inherits the
  listener's properties, as documented by [Apple's accept manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/accept.2.html).
  Five affected fixtures now explicitly restore blocking mode before their
  existing read/write timeouts. This includes the same boundary in the
  supervision and singleflight fixtures, which the interrupted macOS run had
  not yet reached. All listener deadlines and assertions remain unchanged.
- The wrapper framing helper dropped its peer entirely before inspecting EOF.
  Darwin's [socket-option shutdown guard](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/uipc_socket.c)
  can reject the subsequent timeout update with `EINVAL`. The helper now
  half-closes the reply direction and retains the peer until reading finishes.
  Every existing exact framing assertion remains; additional fully disconnected
  cases still require failure. Production transport behavior is unchanged.

All 173 locally selected cases passed across seven harnesses: 106 Asupersync
library cases, 12 worker reconnection/shutdown cases, and 55 wrapper cases.
There were no failures, ignored or filtered cases in these runs. Locked
workspace/all-target check, strict Clippy and formatting passed. The two edited
files containing production code retain byte-identical production prefixes;
the other five Rust files are integration tests. Native macOS validation of
this new fixture batch remains pending.

The unchanged official seven-file UBS scan returned exit 1: one critical,
307 warning and 129 informational findings. The critical record is unchanged
timestamp-based staging-filename code, not security-token generation; exclusive
creation refuses collisions. Its review and the seven added-line test-assertion
records are retained. No scanner suppression, threshold or source change was
used to silence the finding, and no all-scope scanner pass is claimed.

At `5352da59`, all four workspace jobs and coverage reached their original
30-minute limits. Linux ARM completed ten workspace harnesses with 189 passes;
Linux x64 and macOS Intel stopped during workspace compilation after their
separate nine-case fixture passes. Coverage completed 12 harnesses with 200
passes before interruption in `rabs-cas`; LCOV and the 65% verdict are absent.
The raw coverage job interleaves stdout/stderr, so its complete summary sequence
supersedes an earlier undercount from grouping summaries by adjacent headers.
Linux x64/ARM resource sampling recorded 60/59 samples, minimum available memory
of 5,182,912/9,667,652 KiB, and peak observed combined rustc RSS of
9,909,064/4,038,124 KiB. Both recorded zero host OOM kills. These observations
do not establish a causal performance improvement or diagnose earlier shutdowns.

Compiler, strict Clippy, docs, formatting, workflow/manifest/security guards and
benchmarks passed. [RABS](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38143211275)
passed 42 protocol and 13 release-gate cases, with wrapper size 443,456 bytes and
enforced p95 8,640 microseconds under 10 ms. [Rsync](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38143211273)
passed 64 selected transfer cases, and all 1,390 archived Git blobs and modes
match this head. All five Unix [package jobs](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38143211270)
passed; downloaded digests, archive checksums, executable membership and modes
were verified. Windows reproduced its deferred import/type errors; aggregate
package verification was skipped.

[Core E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38143211286/job/114484582073)
passed 164 reported native cases, with 92 archived capability-skip records.
Its shell phase recorded three passes and one skip, then reached the 30-minute
limit during path-dependency setup; nine shell outcomes and the advisory
comparison are absent. [Standalone Linux E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38143211279)
passed all 41 script statuses, with 11 inner passes/two skips and a 1.455 ms hook
mean under the unchanged 10 ms limit. Twelve structured skip records are retained
separately; they are not distinct-case counts. The macOS standalone script step
then reached its original 60-minute limit after seven passing script statuses.
The eighth script was still running; 34 outer outcomes are absent. Cache cleanup
was still active when the approved replacement configuration was prepared.

Evidence is retained under `rch-ci-head5352-monitor-jamixdg7/` and
`rch-darwin-fixture-repairs-ztlgowk5/` in the validation base. The scanner report
is in active-worktree `target/managed-storage-validation-20261009/ubs-darwin-fixtures-report-approved.n_qnlczb/`.
The user approved the prepared three-field CI budget change on October 11:
workspace tests and coverage now have 60-minute job ceilings, and standalone
E2E has 90 minutes. Assertions, per-test deadlines, compiler settings and the
65% coverage threshold retain their existing values. YAML comparison verifies
that only these three values changed. Earlier timeout results remain failures
of the old budgets, not passes under the new configuration. Keep the PR draft:
complete workspace/coverage, new native macOS fixture acceptance and operator
acceptance remain open. No merge or deployment has been performed.

## CI resource follow-up after `425dee6`

The Linux x64 resource probe recorded two concurrent `rustc` processes using
14.33 GiB of RSS together, with 722.6 MiB of host memory available in its last
sample. The runner later stopped with exit 143, before the 30-minute limit.
All 44 samples recorded zero host OOM kills; the last cgroup OOM counters were
also zero. This establishes memory pressure, not the termination mechanism.
The completed 95 harness summaries recorded 2,163 passes, zero failures and
seven ignored cases before the nested daemon build was interrupted.

Linux ARM's 59 samples also recorded zero host OOM kills. Its peak combined
`rustc` RSS was 14.43 GiB and minimum available memory was 786.4 MiB. That job
reached the explicit 30-minute limit during compilation, with no native harness
results. Both macOS matrix jobs also timed out during compilation, so the
filename-refusal branch below remains unverified on native macOS at this head.
Coverage completed 95 harnesses with 2,163 passes and seven ignored cases before
its own 30-minute limit; it did not produce a coverage-threshold verdict.

The follow-up workflow uses one compiler job during Linux matrix tests,
including inherited nested Cargo builds. Native test parallelism, compiler
profiles and every job/test limit remain unchanged. It runs the existing
nine-case filesystem fixture before the full workspace command, retaining a
separate log for each. The complete workspace is still attempted after a fixture
failure, and either failing stage fails the job. This prioritizes native evidence
for the changed fixture without replacing the full-workspace gate.

YAML comparison limits the change to the test-step body and the additional log
upload path. Nine shell controls verify both commands run, Linux receives one
compiler job, the macOS guard retains two, and failures 101/143 remain failures.
The resource child is reaped. These use fake Cargo on Linux, including the
macOS guard; native memory reduction and macOS fixture acceptance remain
pending. The unchanged official UBS scanner reports zero critical, warning or
informational findings in the exact extracted Bash body. UBS has no YAML
scanner; YAML parsing, semantic comparison and Bash syntax were checked
separately. No Rust source changed in this follow-up.

At `425dee6`, the [standalone E2E workflow](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38139131073)
passed all 41 script statuses on both Linux and macOS. Each inner suite recorded
11 passes and two skips. Their 50 hook samples averaged 1.371 ms and 4.708 ms
respectively, under the unchanged 10 ms limit. Separate JSONL files contain
19 Linux and 13 macOS skip records, including missing Bun/worker/daemon
capabilities and unrelated unimplemented scenarios; these are records, with
duplicates, not counts of distinct tests. Outer script success does not prove
those skipped paths.

The [rsync workflow](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38139131071)
passed 64 native transfer cases and strict checks. Its source archive's 1,390
Git blobs and modes match `425dee6`. All five Unix
[package jobs](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38139131078)
passed; downloaded ZIP digests, archive checksums, exact three executable
members and modes were verified. Windows reproduced its deferred import/type
failure and aggregate package verification was skipped. Compiler, strict Clippy,
docs, formatting, workflow/manifest/security guards and benchmarks passed.
[Core E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38139131176/job/114473168252)
passed with 164 reported native passes, 92 archived capability-skip records,
and 11 shell passes/two skips. Its advisory comparison flagged CMake 21/461 ms
and clang/make 42/662 ms (stored baseline/current p95); these remain
unattributed. The [RABS workflow](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38139131069)
passed all 42 accepted-hit cases and all 13 release-gate cases. Wrapper size was
443,456 bytes and enforced p95 was 5,236 microseconds under the unchanged 10 ms
limit. Its 41 release-build resource samples recorded at least 4,158,672 KiB
available memory and zero host OOM kills. This is acceptance of the existing
gate, not a measured speedup or an explanation of earlier runner shutdowns.
All five workflows at `425dee6` are now terminal.

Current CI evidence is retained under `rch-ci-head425-monitor-_l7941gr/` in the
validation base. The workflow source, controls and UBS evidence are under
`rch-linux-compiler-limit-WXHcMr0O/`. The separate job-budget proposal remains
unapplied pending the user's decision. No complete-workspace or coverage pass,
runtime speedup, merge, deployment or live-worker acceptance is claimed.

## Filesystem filename fixture follow-up after `4034fe2b`

Native macOS CI at `4034fe2b` confirms the canonical-root repair below: all 196
sandbox unit cases passed, including both formerly failing path assertions.
The run then reached `snapshot_capture_realfs` and failed its special-node test
and long-TMPDIR child because the filesystem refused creation of a filename
containing byte `0xff`. The error occurred before capture ran: Darwin
`EILSEQ` (92), defined in [Apple's errno header](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/sys/errno.h).
The 72 top-level harness summaries recorded 1,690 passes, two failures and two
ignored cases. The child failure's embedded summary is not counted again.

The fixture still requires `CaptureRejection::InvalidPath` when a non-UTF-8
filename can be created. Only the observed macOS `EILSEQ` refusal is recognized
as an unavailable filename representation; the directory must remain empty and
an explicit diagnostic records that application capture of this name was not
exercised. Other setup errors still fail. Every Unix run also creates a
non-UTF-8 symlink target, verifies its exact bytes and requires the capture
layer's `InvalidPath` rejection. Special-node and escaping-link checks remain
enforced. No production source or dependency changed.

The workspace matrix now uses Cargo's `--no-fail-fast` to continue to later
native harnesses after a failure while retaining Cargo's failing exit status.
Linux test jobs also record bounded memory, cgroup, OOM-counter and process-RSS
observations in the live job log, using the same approach already present in
the RABS release workflow. At `4034fe2b`, both Linux test runners shut down with
exit 143; ARM stopped after 27m 19s and x64 after 29m 39s. Neither annotation
reported the 30-minute timeout, and no assertion failure preceded those exits.
An OOM diagnosis is not established. The probe will provide evidence on the
actual runner; the CI budget proposal remains unapplied.

Workflow parsing confirms that only the matrix test step's shell body changed.
All job limits, compiler settings, matrix members and other YAML values remain
unchanged. Local shell controls preserved exits 0, 101 and 143 with the Linux
probe enabled and confirmed that its child was reaped. The non-Linux guard
controls preserved exits 0 and 101 without starting a probe. These controls
ran on Linux and are not native macOS or CI-host resource evidence.

All nine real-filesystem cases pass locally with no ignored or filtered cases,
including the original invalid filename, the added invalid symlink target and
the long-TMPDIR child. Locked workspace/all-target check, strict Clippy and
formatting passed. Native execution of the macOS refusal branch at the new
revision remains pending. The unchanged official single-file UBS scan returned
zero critical findings, 45 warnings and 67 informational records; its seven
added-line records are three test assertions and four intentional synchronous
filesystem operations in this real-filesystem fixture.

The source, test output and required-check receipts are retained under
`rch-nonutf8-fixture-izf4f5u6/` in the validation base. Scanner evidence is in
`target/managed-storage-validation-20261009/ubs-nonutf8-fixture-report-approved.97yzp9_v/`
in the active worktree. CI evidence at `4034fe2b` is retained separately under
`rch-ci-head4034-monitor-ti79i144/`; its archived 1,390 Git blobs and modes match
that head. Its [RABS workflow](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38135231371)
passed 42 accepted-hit cases and 13 release-gate cases, with wrapper size
443,456 bytes and enforced p95 8,429 microseconds. Its
[rsync workflow](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38135231401)
passed 64 transfer cases and strict checks. Compiler, Clippy, docs and the early
CI guards passed. Core E2E reported 164 native passes with 92 archived capability
skips, plus 11 shell passes and two capability skips. Its advisory comparison
flagged CMake 21/34 ms, Ninja 25/31 ms and clang/make 42/873 ms (stored
baseline/current p95); these flags remain unattributed. Linux standalone E2E
completed all 41 scripts with 11 inner passes and two capability skips; its
50 hook samples averaged 1.207 ms under the unchanged 10 ms limit. All five Unix
package jobs passed, and their archive checksums, exact executable membership
and modes were verified. Windows failed its deferred build and package
aggregation was skipped. Coverage and macOS Intel tests reached their original
30-minute limits. Full workspace, coverage and macOS standalone E2E acceptance
remain open. The macOS standalone job subsequently reached its 60-minute limit:
37 of 41 outer script statuses passed. The self-test inner log also completed
two native cases and three worker skips, but cancellation prevented its outer
status from being written. These partial results are not full-suite acceptance.

## Source-root fixture follow-up after `4e65b0a`

The cached macOS ARM retry at `4e65b0a` reached native tests and failed two
source-transfer path assertions. `SourceReceiver::create` deliberately freezes
the private parent's canonical path. On macOS, the tests supplied a path under
`/var/folders` but compared the returned `/private/var/folders` path with that
alias. The receiver's exact-byte and ownership behavior was not the mismatch.

The two fixtures now compare the returned root with the canonical private
parent. Input paths and byte, mode, incomplete-transfer, rejection and seal
idempotence assertions remain in place. The production portion of
`source_transfer.rs` is byte-identical to `4e65b0a`.

An unchanged Linux test binary reproduced both failures through a symlinked
temporary-directory ancestor while both tests passed through the direct path.
With the corrected expectations, both cases pass through each path. The full
sandbox unit suite also passes in both layouts: 209 passed, one explicitly
ignored, zero failed or filtered. This admits valid ancestor aliases while
retaining the existing invalid-input and ownership checks; it is not a runtime
performance change. Native macOS confirmation at `4034fe2b` is recorded above;
the later filesystem fixture failure remains a separate result.

Locked workspace/all-target check, strict Clippy and formatting passed after the
fixture change. The unchanged official single-file UBS scan returned zero
critical findings, 49 warnings and 55 informational records. Its three
added-line findings are the retained test assertion macros; all were reviewed.
The before/after binaries, source hashes, negative control, complete unit output
and required-check receipts are retained under
`rch-source-root-canonicalization-7xxau3f6/` in the validation base. The scanner
report is under
`target/managed-storage-validation-20261009/ubs-source-root-report-approved.r5tk64vl/`
in the active worktree.

A separate three-line proposal would increase only workspace-test/coverage job
ceilings from 30 to 60 minutes and standalone E2E from 60 to 90 minutes. It is
unapplied pending the user's decision under the repository's gate policy. The
current fixture repair retains all existing CI and native-test time limits.

## Published repair batch at `4e65b0a`

The published changes address the remaining core E2E failures and defects exposed
by the first complete local workspace runs. Current-source core E2E, rsync, RABS
and the five Unix packaging jobs pass. Full workspace, coverage and macOS E2E
acceptance remain open. The native macOS hook passed at the preceding `968f1b1`
revision; that result is historical, not a substitute for the current incomplete
macOS run. Local and CI failures are retained below; the PR stays draft.

- Shared logging now follows the configured console stream's terminal status,
  honors `FORCE_COLOR`, and lets any present `NO_COLOR` value disable color.
  JSON and file logging retain their plain output. The original core CI failure
  was colored debug output with `NO_COLOR`; the subsequent stream check also
  caught colored piped output without an override. Both conditions are covered.
- Source-cache writers explicitly unlock when their owner finishes. Durable
  toolchain caches explicitly unlock after their last execution lease ends.
  An unrelated child that briefly inherits a descriptor before `exec` no longer
  extends either lock's ownership. Regression cases retain duplicate descriptors
  across the owner transition and still enforce active-owner exclusion, verified
  bytes, and exclusion by a newly opened owner. Managed execution-storage leases
  intentionally inherited by descendants retain their existing behavior.
- The FIFO preload test runs its existing assertions in an isolated child, with
  both successful exit and a completion marker required. Counts 0, 1, 3 and 4096,
  token return, immediate `WouldBlock`, and immediate EOF remain checked. A
  failed `mkfifo` control also fails the parent test.
- The live Cargo fixture asks the recorded compiler for its sysroot before
  hashing the actual compiler and runtime files. It supports Cargo recording
  `rustc` or a rustup proxy, and verifies that the direct compiler's verbose
  version matches. Both real-Cargo cases pass with native-toolchain-first and
  rustup-first PATH configurations; hashes and contract assertions remain.
- Daemon-stop tests serialize only CLI-file preparation through successful
  process spawn. A parallel fork can otherwise inherit another fixture's writable
  executable handle and cause `ETXTBSY`. The lifecycle requests, socket identity,
  refusal, unsafe-command, and original timeout assertions remain concurrent.
- Nineteen Rust cache steps now include the root `Cargo.toml` hash in their key
  prefix. The pinned cache action derives package manifests from Cargo metadata,
  which omits this virtual-workspace root; the prior CI restored an exact cache
  key despite the changed development profile and then declined to save it.
  Core E2E also puts its built release binaries on PATH, which the two reliability
  scripts require. Workflow parsing and semantic comparison confirm these are
  the only workflow changes; test commands, budgets and enabled jobs are intact.

The preceding full run failed the full-toolchain acquisition deadline and the
daemon fixture's `ETXTBSY` launch. Its raw results remain separate from the final
run below, which uses August nightly, ordinary Cargo profiles, two compiler jobs,
the complete installed toolchain, and the original test parallelism and limits.

| Final local validation | Observed result |
| --- | --- |
| Full default-feature workspace run | All 12,143 collected cases match 225 harness summaries: 12,110 reported passes, two failures, 31 ignored, zero filtered. The failures are the toolchain acquisition deadline and hook timing against the retained older release CLI, detailed below. |
| Affected native regressions | All six daemon-stop cases passed, including the former `ETXTBSY` failure. FIFO, source-cache, durable-cache, real-Cargo compiler resolution, managed-storage integration, and both Go recovery cases passed in the full run. |
| Console output | All 46 logging tests passed, including 144 actual child-process format/stream/environment combinations. The regression failed before the production repair. All 48 rebuilt CLI terminal/pipe controls passed; the older implementation failed ten of them. |
| Affected shell checks | Stream isolation and both reliability scripts passed. The reliability scripts recorded 19 and 44 native passes respectively, with no failures or ignored cases; their filters overlap, so these are not 63 distinct cases. |
| Required compiler checks | Locked workspace/all-target check, Clippy with warnings denied, and formatting passed. |
| Rebuilt release CLI | The unchanged 13-case hook integration harness passed with no skips or filters. Its 20 timing samples averaged 2 ms under the unchanged 25 ms limit. |

Eight reported workspace passes took explicit capability-skip paths: two
delegated-cgroup cases, two signing-tool cases, three live-worker cases and one
infrastructure smoke case. These do not establish live acceptance. All Rust,
Cargo and workflow inputs remained unchanged throughout validation; only this
handoff changed while the full run was executing.

The full run's hook timing failure measured 130 ms against the 25 ms limit.
That harness resolves `release/rch`, independently of Cargo's freshly built
debug CLI. Its retained release executable was from October 9. The same binary
subsequently passed all 13 cases at a 6 ms mean, before rebuilding; the current
release CLI then passed at 2 ms. Both binaries, the unchanged harness identity,
and all timing records are retained. This establishes variation between runs,
not its cause or a performance improvement. The fresh release check does not
erase either failure from the complete workspace run.

The large-toolchain timeout remains open locally. The original three-case
artifact harness reproduces it with the complete installed August toolchain
and unchanged 30/60-second controls. Profiling now attributes most work to
filesystem operations. Neither a second NFS-backed ext4 image nor a direct
local-ext4 scratch directory resolved it in paired runs; the latter also had
one 60-second compiler case time out. Retain these failures. No dataset
exclusions, integrity-barrier changes or deadline increases were made.
CI summary passes do not close this local failure: the artifact isolation tests
can return early when canonical namespace support is unavailable, and successful
test output is captured by the ordinary Cargo test command.

The current seven-file UBS scan returned exit zero and zero critical findings;
322 warnings and 176 informational records are retained. Its eleven added-line
records were reviewed as test assertions, fixture-lock failure propagation,
bounded test labels, the fixed-argument child launch, and a path-string false
positive. Scanner inputs and the six workflow hashes match the current source.
Earlier broader-scope findings remain open; no all-scope scanner pass or scanner
Cargo/AST/ShellCheck phase is claimed.

| Current CI at `4e65b0a` | Observed result |
| --- | --- |
| [CI 38128170844](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38128170844) | All-target/all-feature check, strict Clippy, docs, format, workflow/manifest/security guards and benchmarks passed. The first attempt's four test jobs and coverage reached their original 30-minute limits. Linux x64 completed 95 harnesses with 2,163 reported passes and seven ignored cases; coverage completed ten harnesses with 189 passes. Neither is a complete suite or coverage verdict. The other three original test jobs stopped during compilation. |
| [Core CI E2E](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38128170844/job/114440189807) | Passed: 164 reported native passes include 92 archived capability-skip records. The shell suite completed with 11 passes and two capability skips. Stream isolation and both reliability scripts now pass at the published source. The existing advisory performance comparison flagged three timings, detailed below. |
| [Linux ARM retry](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38128170844/job/114447550770) | Restored the exact 1,937,327,214-byte cache, compiled the test targets in 16m 26s, then completed 95 harnesses with 2,163 reported passes, seven ignored cases and no reported failures. The original 30-minute job limit interrupted a nested `cargo build -p rabsd --bin rabsd` during `chaos_faultinjection`; the remaining harnesses did not run. |
| [macOS ARM retry](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38128170844/job/114453192163) | Restored the exact cache and compiled the test targets in 8m 51s. Sixty completed harness summaries recorded 1,670 reported passes, two failures and two ignored cases. Both failures are canonical-root path expectations in `rabs-sandbox`, addressed by the follow-up above. Cargo stopped with exit 101; this is an assertion failure, not a timeout or a complete suite. |
| [E2E 38128170826](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38128170826) | Linux completed all 41 discovered scripts; its inner suite recorded 11 passes and two capability skips. Its 50 hook samples averaged 1.361 ms under the unchanged 10 ms mean limit. macOS reached the original 60-minute job limit after five passing script statuses; the sixth script was compiling when interrupted. The other 36 statuses are absent, not passes or skips. The successful aggregate job only published the report. |
| [Test Release 38128170819](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38128170819) | All five Unix build/version/package jobs passed. Each downloaded ZIP digest, archive checksum, exact three executable members and executable modes were verified. Windows failed the deferred non-Unix `read_bounded_output_stream` import; aggregate verification was skipped. |
| [RABS 38128170889](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38128170889) | Both jobs passed: 42 accepted-hit cases and 13 release-gate cases, without ignored or filtered cases. Wrapper size was 443,456 bytes and enforced p95 was 6,785 microseconds under the unchanged 10 ms limit. |
| [Rsync 38128170880](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38128170880) | Both jobs passed, including 64 native transfer cases and strict checks. All 1,390 archived Git blobs and modes match `4e65b0a`; the CI merge archive is source-equivalent to that named head. |

The advisory comparison retained these p95 flags: CMake exit-code success
21/382 ms, clang/make exit-code success 42/678 ms, and all-workers-busy fail-open
527/720 ms (stored baseline/current). The existing workflow ends this comparison
with `|| true`; it is not an enforced gate. These results do not establish a
cause or a performance improvement, and the stored baselines were not changed.

Cache availability is a separate remaining limitation. A completed coverage
cache upload was already absent from the subsequent inventory; four caches
totalling 9,357,917,463 bytes remained. The macOS E2E cache archive failed to save.
The Linux ARM retry proves that restoring its retained cache is insufficient to
finish the complete suite within the existing limit. The nested daemon build
requests different dependency features from the workspace build, so substituting
an existing workspace executable would change the tested configuration. No such
substitution, test exclusion, deadline increase or paid cache setting was made.

Current CI receipts, original and retry job metadata, annotations, raw logs,
verified artifact ZIPs, source binding and outcome summaries are retained under
`rch-ci-head4e65-monitor-2pwf_pe_/` in the validation base. GitHub gives carried-over
jobs new IDs during a targeted retry; those copied successes are not new test
executions. The original job receipts above remain authoritative for those jobs.

| Terminal CI at `968f1b1` | Observed result |
| --- | --- |
| [CI 38001161692](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38001161692) | All-target/all-feature check, strict Clippy, docs, format, workflow/manifest/security guards and benchmarks passed. The four test jobs and coverage reached their existing 30-minute limits. No complete workspace or coverage verdict was produced. |
| Core CI E2E | 164 reported native passes included 92 archived capability-skip records. Shell checks recorded seven passes, two skips and three failures before timeout: the two missing-PATH reliability scripts and the debug-color stream check repaired above. |
| [E2E 38001161690](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38001161690) | Linux and macOS each completed all 41 discovered scripts. Each inner suite recorded 11 passes and two capability skips. The native macOS hook's 50 samples averaged 4.369 ms under the unchanged 10 ms mean limit. |
| [Test Release 38001161789](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38001161789) | All five Unix package jobs passed; archive checksums, the exact three executables and executable modes were verified. Windows failed the deferred non-Unix import; aggregate verification was skipped. |
| [RABS 38001161695](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38001161695) | Both jobs passed: 42 accepted-hit cases and 13 release-gate cases. Wrapper size was 443,456 bytes and enforced p95 was 8,325 microseconds under the unchanged 10 ms limit. |
| [Rsync 38001161709](https://github.com/tsunheimat/remote_compilation_helper/actions/runs/38001161709) | Both jobs passed, including 64 native transfer cases. All 1,390 archived Git blobs and modes match the named head. |

Dependabot automerge was skipped. Current evidence under the retained validation
base includes `rch-ci-head968-monitor-ZuwWCDey/`,
`rch-full-workspace-post968-repairs-lit9mkh6/`,
`rch-full-workspace-final-repairs-7674usti/`,
`rch-release-hook-final-control-9frhqqby/`,
`rch-logging-terminal-regression-k04bs_b5/`,
`rch-artifact-capture-profile-0rlbs9t9/`, and
`rch-local-ext4-capture-control-jgs_n3n_/`. The current scan is retained in the
active worktree under
`target/managed-storage-validation-20261009/ubs-final-source-report-approved.ievg0cbu/`.

## Toolchain and hook repairs at `968f1b1`

All six workflows at `1c56ce1d` are terminal. This published batch addressed the
remaining wrapper-fixture environment leak, development hashing cost, and
portable hook checks. Its completed CI results supersede the then-pending
native macOS and workflow status below; operator deployment acceptance remains
open.

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
  samples are retained and printed. Native macOS acceptance is recorded above.
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
2. Complete the ARM Mac whole-workspace release validation. Its single cached
   retry still exceeded the unchanged 30-minute limit; the separate 60-minute
   main-release proposal awaits approval. All four configured workspace test
   platforms, coverage, core and standalone E2E, RABS, Rsync and the five Unix
   Test Release packages passed at `1cbcf0e4`. Main release builds passed on all
   three Linux targets and Intel Mac. Preserve the original gates and distinguish
   incomplete jobs from passing checks. The old rustdoc, checksum, runtime-policy,
   watchdog, binary-preparation and failure-propagation defects are repaired.
3. Validate deployment, real SSH, and Nexus behavior in the user's environment
   when the user is ready. Privileged Linux mounting and native cancellation
   already passed locally; they do not prove acceptance on an operator's worker.
4. Leave Windows-specific repair deferred and keep PR #2 draft while required
   acceptance remains open. Existing Windows build failures remain visible.

When the user is ready to deploy, run the worker acceptance in this order:

1. Select a POSIX worker and provision the chosen persistent volume, package
   configuration and credentials. Follow the [configuration guide](guides/configuration.md#worker-cache-scratch-and-environment-placement).
   Keep source mirrors, tools and target pools outside `/tmp` if using private
   mounts. Start with `tmp_mode = "env"` and inspect the effective controller
   configuration with `rch config validate` and
   `rch config get execution.storage.root --sources`.
2. In a representative project, run
   `RCH_REQUIRE_REMOTE=1 rch exec -- cargo test`. Check its selected worker and
   durable completion receipt, compiler/test status and returned artifacts.
   Require the actual remote path to succeed; a local fallback is not evidence
   for worker storage placement.
3. On that worker, verify the configured cache paths and per-job scratch path.
   Repeat the build to check cache persistence. Exercise a failed build and a
   normal RCH cancellation of an owned job; verify the original status and
   receipt behavior, and that scratch is reclaimed only after its lease is
   released. Do not purge shared caches or unrelated jobs to create a fixture.
4. Use a project whose package manager is configured for the intended Nexus
   repository. Confirm a successful fetch in the Nexus access records, then
   repeat it to verify reuse of the worker cache. Environment defaults alone do
   not install registry configuration or credentials into a new `CARGO_HOME`.
5. If private `/tmp` is required, verify mount namespace privileges on that
   worker, switch to `tmp_mode = "private_mount"`, and repeat success, failure
   and cancellation checks. Confirm `/tmp` resolves to the job scratch while
   the watchdog record remains visible in the parent namespace. A privilege
   refusal is a valid refusal test, not proof that private mounting succeeded.

These are operator acceptance steps, not claims of deployment or live SSH/Nexus
validation by this development session.

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
prepared. CI builds the application release binaries and places `target/release`
on `PATH`. The separate `true_e2e` harness needs its expected debug daemon.
Build the release CLI before running `hook_integration`: that helper resolves
`release/rch` independently of Cargo's debug binary. Respect local RCH offloading
instructions and the user's chosen validation location.

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
