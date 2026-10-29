# Fork CI/CD audit — 2026-10-16

Baseline: `a582ac1d46f80ca19be2fc6c28627c54c819e1bf`, PR #3,
`feat/upstream-v2.1.34-integration`.

The user requested removal of CI/CD that does not provide useful evidence for
this fork. The criterion is unique coverage, not whether a job currently passes:
keep a job when it checks a distinct platform, execution boundary or shipping
artifact; consolidate duplicated compilation; remove upstream-only publication
and historical candidate-landing automation.

## Result and normal execution

Workflow files: **20 → 12**. Eight workflow files and the validator that existed
only to inspect the deleted Dependabot automerge workflow are removed. No product
Rust/Python/shell tests or application code are deleted or weakened.

- Main CI performs formatting and compiler-backed Clippy together, on the full
  workspace/all-target/all-feature graph, with `--locked` and `-D warnings`.
- The four native test rows are unchanged: Linux x64/ARM64 and macOS Intel/ARM64.
  Existing macOS daemon exclusions are not expanded. Workspace tests still
  include fallback, rsync, source selection, installer and live-protocol tests.
- CI calls one reusable E2E workflow. Linux Cargo `true-e2e`, explicit mock-SSH
  shell tests and the complete Linux/macOS shell scenario suite are retained.
  Linux harnesses share one release application build. Mock/Cargo logs are kept
  separate from the native scenario aggregator to avoid mixing evidence.
- CI calls one six-platform packaging workflow instead of also running a second
  six-platform generic release matrix. GNU x64, musl x64, Linux ARM64, macOS
  Intel/ARM64 and Windows retain real shipping profiles, version smoke tests,
  archive creation and aggregate checksum/package verification.
- RABS keeps native installer tests and its distinct release-only size, overhead,
  handoff and live-Cargo gates. Only its duplicate debug wrapper job is removed.
  Relevant source, manifests, lockfile and toolchain changes trigger this lane.
- The `ci-success` aggregate includes bundle/workflow integrity and rejects any
  failure, cancellation, skipped required job or missing required result.

Documentation, classifier/remediation benchmarks and instrumented coverage are
available through **CI → Run workflow → extended = true**, rather than running on
every PR. Existing documentation warnings, benchmark budgets and the 65% coverage
threshold are unchanged and become required by the aggregate when requested.

The CI weekly schedule runs the dependency security scan and its aggregate only.
A separate weekly/manual wrapper-contract probe compares stable, beta and nightly
using one harness build, replacing three daily builds. PR workspace tests continue
to install and exercise all three channels. This deliberately trades daily drift
notification for weekly drift notification; no channel is removed.

## Removed workflow owners

| File | Reason | Retained coverage or replacement |
| --- | --- | --- |
| `dependabot-automerge.yml` | Automatic merging is not a correctness check; the current main branch has no required-check protection. | Weekly grouped Dependabot PR configuration remains unchanged; merging is explicit. |
| `notify-acfs.yml` | Dispatches this fork's installer checksum to an upstream-owned repository. | Installer ShellCheck moved into CI bundle/workflow integrity. |
| `release-automation.yml` | Automatically tags a fork on imported version changes without waiting for CI. | Explicit version tags or manual invocation still use the real release workflow. |
| `source-admission.yml` | Replays hard-coded candidate blobs and writes prepared Git objects, rather than testing committed HEAD. | Both real source-registry scripts moved to read-only durable-execution validation; current Rust tests remain in the workspace suite. |
| `source-execution-recovery.yml` | Historical candidate staging; its referenced Python helper/test do not exist in the committed tree. | Current committed ownership/recovery tests remain; no nonexistent candidate test is represented as passing. |
| `windows-source-sync.yml` | Hard-coded upstream repository/issue gate and patch-to-main landing operation. | Committed source archive, receipt and exclusion tests remain in workspace CI. |
| `remote-only-policy.yml` | Repeats the full-workspace formatter/compiler/linter and fallback test coverage. | `hook::fallback_policy_tests` remains registered and runs in the workspace suite. |
| `rsync-transport.yml` | Repeats full-workspace checks; produces a source archive with no in-repository consumer. | Workspace transfer tests plus the separate real native/legacy rsync lane remain. |

## Retained workflow owners

| File | Distinct purpose / trigger |
| --- | --- |
| `ci.yml` | PR/main validation, single aggregate; optional manual extended checks; weekly security only. |
| `e2e.yml` | Reusable/manual Cargo, mock-SSH and complete Linux/macOS shell scenarios, with existing per-scenario verdict aggregation. |
| `test-release.yml` | Reusable/manual six-platform shipping build, smoke test and archive verification. No independent PR trigger. |
| `rabs-binaries.yml` | Source-filtered PR/main or manual native installer and release-only RABS gates. |
| `release.yml` | Explicit tag/manual release, artifact integrity/provenance and publication. Intel runner aligned to `macos-15-intel`; upstream crate publication is skipped for the fork. |
| `durable-execution.yml` | Source-filtered main/manual exact formatter, supervisor-loss and real source-registry tests; read-only, no patch-to-main job. |
| `source-content-ssh.yml` | Source-filtered main/manual real SSH/rsync integrity tests; historical publication job removed. |
| `remote-process-platform.yml` | Source-filtered main/manual native Linux/macOS identity, timeout and owned-process lifecycle; duplicate workspace lint/check removed. |
| `rsync-paths.yml` | Source-filtered main/manual real modern Linux and stock legacy macOS rsync argument behavior. |
| `managed-pool.yml` | Source-filtered main/manual real policy under checked and optimized arithmetic, including Windows. |
| `update-lock.yml` | Source-filtered PR/main or manual native cross-process ownership on Linux, Windows and macOS; the independently generated stable manifest is checked before merge, while full-client validation remains with the workspace owner. |
| `wrapper-contract.yml` | Weekly/manual external Rust channel drift, all three channels in one build. |

The updater ownership lane copies exact production modules into a separate
stable-toolchain crate. Its generated manifest must declare `rustix` for Unix,
not just macOS: the Linux process observer also uses it, and a transitive copy
cannot satisfy a direct import. Main-push run 40802227637 exposed that omission
before Linux tests could run; macOS and Windows passed. The same existing lane
now checks only relevant PR paths as well, so generated-manifest drift is not
first discovered after merge. Platform coverage, assertions and the 15-minute
job limit are unchanged; this adds no workflow file or full-client rebuild.

### Intel profile packaging and qualification caches

The combined Intel shipping job exceeded its 90-minute limit in CI run
40651533671. `test-release.yml` now builds the unchanged wrapper and daemon
profiles on separate native Intel runners, each with that same per-job limit.
This raises possible aggregate runner allocation; it is not a measured compiler
speedup or a relaxation of the wrapper's runtime SLO. Both profile jobs, native
binary smoke tests, and the same-revision final archive are still required.

`scripts/package_ci_components.py` is consumed by that workflow to bind each
component's source commit, target, profile, member names and byte digests before
assembly. The final archive is checked and executed on Intel macOS, then joins
the other five targets in the existing all-platform verifier. The component
self-test uses synthetic bytes, not real compiler outputs. This helper can be
removed when neither shipping nor E2E needs a split-build handoff; do not
retain an unused manifest/certificate path. The workflow count remains twelve, not twelve jobs.
The separate signed-release publication workflow is unchanged and is not
qualified merely by this candidate-packaging change.

### E2E application preparation is separate from test execution

CI run 40681529196's macOS E2E job exceeded its unchanged 90-minute cap:
release preparation took 47m42s, then the first debug application build took
17m55s and the cancellation scenario's test graph took 14m17s. Only 31 of the
41 scripts had completed when the job was cancelled; that is a failed run,
not an E2E pass. Its retained artifact and job annotation establish the split
between build and test time, not a product workload timeout.

`e2e.yml` now runs native release preparation in a separate required job per OS.
Each preparation and test job retains the prior OS-specific 120/90-minute cap;
possible aggregate runner allocation increases. The compiler profile/flags,
all 41 discovered scripts, Linux Cargo/mock-SSH paths, and all product budgets
are unchanged. Build/test cache keys are separate. The same-run artifact must
match commit, native target, release profile, exact three applications and
hashes before any tests can consume it. The existing component helper restores
executable permissions and replaces only those named cached applications;
missing/mismatched/tampered inputs fail rather than silently invoking stale
cached binaries. Native source-stamped version checks run on both sides.

The helper's added nine cases use synthetic payloads and reject mismatched
source/target/profile, missing/extra members, changed bytes and unsafe links.
They do not establish that native hosted E2E has completed. Application
artifacts have a separate name prefix and are not ingested as scenario results.
The reusable E2E workflow must succeed as a whole; an informational aggregate
cannot override a failed build or cancelled test job.

For local cross-worktree qualification, do not share one Cargo output directory
across different source roots. A native reproduction switched from source B to
A but retained B's integration-test executable, including when checksum mode
was enabled on the existing cache. Separate output roots passed both directions.
Use a fresh, source-owned `CARGO_TARGET_DIR`, record the actual executable from
Cargo's JSON output, and check its test inventory and the compiled client's
source stamp. Source-file hashes alone cannot prove which cached binary ran.
Keep this distinct from the earlier intermittent hook misses: this cache defect
does not establish the cause of those historical observations.

All non-release workflows have read-only repository contents permissions. Only
the dependency audit gets check-write permission. Candidate blob injection,
source patching, direct pushes to main and PR-comment write permissions are not
part of test workflows. No branch protection setting is changed by this cleanup.

## Validation boundary

Actionlint 1.7.12 validates workflow syntax, expressions, dependencies and reusable
calls. Embedded script lint is separate; installer ShellCheck is retained. The
actual aggregate Python body is exercised in 93 subprocess cases covering PR,
main push, ordinary/extended manual runs and the weekly schedule, including
failure, cancellation, skipped and missing required results. All 93 passed.
The 12-workflow dependency graph and read-only validation permissions also passed
static checks. Product source and test files remain unchanged against the baseline.

These workflow-only checks do not establish that all product Rust tests, native
workers or release gates pass. PR #3 remains draft pending exact-revision CI and
its pre-existing integration acceptance. Main and deployed workers are unchanged.
