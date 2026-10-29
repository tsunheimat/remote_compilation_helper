# Migration Guide

This guide helps you transition from local compilation to RCH remote compilation.

## Overview

Migrating to RCH involves:
1. Setting up the infrastructure (workers, daemon)
2. Installing hooks for your AI coding agent
3. Validating behavior matches local compilation
4. Tuning for optimal performance

The goal is **transparent** operation - your workflow shouldn't change.

## Replacing remote-test on Linux

Stage RCH alongside remote-test first; activate its hooks/PATH explicitly and
keep the old installation for rollback until deployment tests pass. This section
describes current source interfaces, not an accepted deployment or a built release
candidate. Do not treat a draft packaging script, passing fixture, or successful
preview as release readiness.

### Invocation contract and prerequisites

RCH's native replacement entrypoint is `rch exec --verification`. It accepts
literal argv after `--`, explicit Bash source with `--shell`, `--cwd`, repeated
`--env KEY=VALUE` and `--forward-env KEY`, `--timeout`, `--interactive`, and
`--json-result FILE`. Explicit `--env` assignments override forwarded values;
unset forwarded variables are omitted. Remote environment inheritance still
follows RCH's configured allowlist, not the controller's entire environment.
A relative cwd resolves from the invoking directory; a relative result path also
resolves there, **not** from `--cwd`. The result's parent directory must exist.
Timeout accepts positive seconds or durations such as `20m` (fractional seconds
round up). Without `--interactive`, workload stdout/stderr remain separate native
byte streams. PTY mode deliberately merges terminal output.

`--json-result FILE` atomically writes RCH's terminal result without consuming
workload stdout. Typed invocations **refuse** `--json` / JSON output mode (including
environment-selected JSON) with exit 2; they do not silently merge streams.
Plain compiler `rch --json exec -- cargo ...` is a different, older contract:
result envelope on stdout and workload output on stderr. Do not alias the old
executable blindly: legacy SSH target flags migrate to configured RCH workers,
and result records use RCH's schema, not remote-test's.

The embedded structural planner needs controller `python3` (stdlib only, launched
with `-I`; no installed remote-test imports). Explicit shell execution needs Bash
on the executing host; guarded local shell execution uses `/bin/bash`. Local PTY
execution also needs `/usr/bin/python3`; remote PTY execution needs the worker's
`script` utility. Provision worker language runtimes, toolchains, SSH, and package
credentials separately. Planner failure or unsupported grammar can leave a
command local: this mechanism is not a security or permission boundary.

### Shared workspace configuration

General verification uses explicitly configured **shared workspace** mappings;
it does not upload a mirror and guess which arbitrary script outputs to copy
back. Supplying any explicit invocation option selects this contract, defaulting
to `--workspace shared`; `--workspace mirror` is currently rejected for typed
argv/shell, cwd, environment, timeout and native streams. Existing plain compiler
execution keeps its typed mirror/artifact policy. Shared invocations cannot use
mirror clean overlays or source-content receipts.

Configure a canonical enclosing shared volume (not the project directory) in
`~/.config/rch/config.toml` or the project's `.rch/config.toml`, so RCH's private
control state stays outside source inputs:

```toml
[jobs]
local_guard_root = "/mnt/hdd-vibe-coding-share/rch-local-jobs"
rtk_tee_root = "/mnt/vibe-coding-share/rch-tee"

[[jobs.shared_workspaces]]
worker = "builder-1"
controller_root = "/mnt/vibe-coding-share"
worker_root = "/mnt/vibe-coding-share"
```

Use your actual worker ID and canonical mounted paths. Both endpoints must see
the same backing tree; matching strings alone do not suffice. RCH verifies a
fresh control nonce in both directions and rejects nested mounts beneath a
mapping. `rtk_tee_root` is a private controller-side collection directory outside
source inputs; it does not have to be mounted on the worker. Worker output is
retrieved over SSH as described below. Cache and registry credentials/toolchains
must already be provisioned on the worker.
No mapping means an explicitly reported guarded-local fallback where policy
permits it, or refusal under strict-remote/dispatcher policy. This fallback is
limited to authorized no-start verification paths; a failed or uncertain remote
attempt is not permission to rerun locally. Adding `--job` disables this fallback.

```bash
# Literal arguments, separate workload streams, independent terminal result.
rch exec --verification --cwd /mnt/vibe-coding-share/project \
  --env CI=1 --timeout 20m --json-result result.json -- pytest -q

# Shell source is explicit; it is never inferred from a single argv argument.
rch exec --verification --shell 'npm test && npm run build' --timeout 30m
```

### Package the committed candidate

On a compatible Linux build host with the pinned toolchain installed, use
`bash scripts/package_linux_candidate.sh --output-dir /absolute/candidates/rch-COMMIT`
to build only `rch`, `rchd` and `rch-wkr`. It never activates binaries, hooks,
PATH or services. By default it refuses uncommitted source **before** building
or creating the candidate directory. Commit the replacement functionality and
its tests first; do not accidentally reproduce the earlier dirty-build dependency.

`--allow-dirty` is an explicit development-only exception. Such a candidate is
labeled as a dirty overlay, with its tracked diff and all relevant untracked
runtime sources fingerprinted. It is not a signed release. Both clean and dirty
candidates contain a source manifest, binary/archive digests and a completion
receipt published last. Missing `receipt.json` means incomplete, not deployable.
`--verify-source CANDIDATE_DIR` checks those source/archive identities without
building or installing; it does not replace signature or target-host verification.

The self-test uses synthetic tool binaries and real Git/archive operations to
check clean-source admission, explicit overlays, source drift, no-clobber output,
and isolated smoke environments. It is not compiler or deployment evidence.
Before operator activation, verify the actual candidate on code1 and the worker;
a successful build-host smoke test alone does not prove target-host compatibility.

### Stage first; activate explicitly

Keep the staged binary outside the active PATH and invoke it by absolute path
while checking its interfaces and worker compatibility. Do not use the README's
`--easy-mode` quick install as an inert staging operation. Do not replace active
binaries, migrate hooks, evaluate PATH activation, or wrap toolchains merely to
inspect a candidate. None of the commands below packages a release.

```bash
# Read-only plans; use the staged binary's absolute path instead of rch as needed.
rch shim install --tools all --dry-run
rch env --shell bash
rch agents migrate --agent codex-cli --dry-run
rch agents migrate --agent claude-code --dry-run
rch agents migrate --agent hermes --dry-run
```

After validation and the operator's activation decision:

```bash
# Install generic frontends; does not modify shell rc files, PATH or toolchains.
rch shim install --tools all

# Either limit activation to one CI process (installed shims must already exist):
rch ci -- ./scripts/test.sh

# Or activate a Bash shell explicitly after reviewing `rch env --shell bash`:
RCH_PATH_BEFORE_ACTIVATION=$PATH
eval "$(rch env --shell bash)"
hash -r
```

If `~/.rch/shims` is already on PATH, writing or refreshing shims there affects
new commands immediately: that is **activation**, not inert staging. `rch ci`
only prepends that directory to its child's PATH; it installs nothing, changes
no parent shell, and cannot intercept absolute executable paths or children
that replace PATH. Ordinary interpreter launches remain local while their nested
verification commands can still resolve through shims. Missing `rch` makes a
generic shim refuse rather than run an unclassified, unguarded workload.

`--tools all` installs only these generic frontends: `go`, `swift`, `npm`, `pnpm`,
`bun`, `pytest`, `python`, `python3`, `node`, `make`, `npx`, `bunx`, `vitest`, `jest`,
`tsc`, `eslint`, `prettier`, `playwright`, `vite`, and `rtk`. It does **not** include Cargo
or clippy. Cargo installation is separate: preview `rch shim install --dry-run`,
then explicitly run `rch shim install` (strict-remote by default). That operation
prints PATH setup advice rather than editing shell startup files, but wraps
rustup Cargo binaries immediately unless `--no-toolchains` is supplied. `--allow-local-fallback` changes Cargo shim policy,
not generic-shim policy; neither Cargo option configures `--tools all`.

### Agent migration and rollback

Run only the applicable agent commands, after previewing each change:

```bash
rch agents migrate --agent codex-cli
rch agents migrate --agent claude-code
rch agents migrate --agent hermes

# Roll back only agents migrated above; preview before applying.
rch agents rollback --agent codex-cli --dry-run
rch agents rollback --agent codex-cli
rch agents rollback --agent claude-code --dry-run
rch agents rollback --agent claude-code
rch agents rollback --agent hermes --dry-run
rch agents rollback --agent hermes
```

Claude and Codex migration recognizes exact single-executable legacy handlers
named `remote-test-codex-hook-bootstrap` or `remote-test-codex-hook`, not arbitrary
shell wrappers or commands mentioning those names. It updates Claude settings or
Codex `~/.codex/hooks.json`, journaling only owned entries for rollback while
preserving unrelated settings. Hermes migration requires the legacy
`remote-test-rewrite` plugin's managed marker and matching asset digests; it
disables the legacy callback before installing RCH routing. Changed or ambiguous
ownership is refused rather than overwritten. No recognized legacy installation
means migration may report unchanged; it is not a fresh hook install.

Codex trust approval and restarting/reloading agent sessions remain operator
actions. Status checks inspect files, not whether a running agent loaded them.
Do not run legacy and RCH rewrite handlers together. Rollback does not restore
PATH or a prior binary release. For the Bash activation example above, return
to its saved PATH with:

```bash
export PATH="$RCH_PATH_BEFORE_ACTIVATION"
hash -r
unset RCH_PATH_BEFORE_ACTIVATION
```

Restore any separately edited shell/CI activation explicitly and reload affected
sessions; retained generic shim files are inert only when no active PATH or
wrapper reaches them. `rch shim uninstall` is the separate Cargo/clippy and
rustup-wrapper rollback, **not** a generic-shim uninstall; it removes those
Cargo shim files and must be explicitly authorized. The current CLI has no
per-tool generic uninstall. Preserve migration journals, legacy files and active
jobs; select the retained known-good version separately rather than deleting
state as a shortcut.

### Mixed agent commands select a backend per component

The agent continues to issue ordinary shell commands. For supported compound
syntax the hook first parses the whole authored structure, then applies native
compilation classification and policy to each component. Both orders below
retain the same per-tool backend; pytest no longer stays bare merely because a
later Cargo command matched the native classifier:

```text
pytest -q && cargo test
  -> rch exec --verification -- pytest -q && rch exec -- cargo test
cargo test && pytest -q
  -> rch exec -- cargo test && rch exec --verification -- pytest -q
```

A standalone native compiler command keeps the existing fast path. Compound
planning uses the bounded embedded Python 3 helper; native configuration
confidence/allowlist exclusions are not retried as verification bypasses.
Ordinary local components, authored quoting, `cd`, conditional operators and
redirections are retained. Agent permission checking remains required for the
rewritten compound; the hook never auto-approves its unrelated prefix.

Pipeline participants and redirected commands use the native-stream shared
contract where supported. A subsequent independent Cargo stage after `;` can
still use its native mirror path. Prefix/export environment forwarding, nested
shell wrappers, groups and background scopes retain the conservative shared
contract rather than silently changing stdin/stdout or environment semantics.
A literal RTK shell remains one shared invocation, not independently scheduled
inner commands. General shared-workspace configuration is still required.

Unsupported control syntax or an unavailable/failed planner does not authorize
a partial native rewrite of the rest of the command. The original command is
left unchanged under the existing hook miss policy; that is not evidence of
remote execution. Existing shim coverage and strict execution policies are not
broadened by this structural fix. The generic-shim classifier and its wire
schema remain separate and unchanged.

### Stability backports and upgrade observations

Default socket provenance is recomputed for the calling environment instead of
reusing a systemd/agent shell's cached default. Explicit socket configuration is
preserved. Configuration cache schema 6 also retains the fork's prior source-root
and remote-base validation invalidation. Doctor reports a live daemon on another
known default socket rather than starting a duplicate.

Freshly admitted workers initially expose no new slots until finite filesystem
capacity is measured, with the upstream 120-second hold bound; this is not a
permanent refusal of unknown capacity. Existing leases remain accounted for.
Prepared Node/Bun tool directories use their physical absolute path across `cd`.
Explicit project rsync exclusions remain authoritative, while tracked source
under names such as `src/target_cache` is no longer mistaken for generated Cargo
output. Cancellation recovery requires durable records and complete process
identity witnesses before claiming the corresponding authority.

### Per-invocation remote requirement

`rch exec --require-remote -- COMMAND...` requires this particular invocation to
run remotely or refuse before local execution. It applies to native compilation,
explicit `--job`, and shared `--verification`; it also forbids guarded-local
verification fallback. The flag alone does not select the shared backend, so
`rch exec --require-remote -- cargo test` retains the native mirror/artifact path.
It does not make an ordinary command offloadable: use the existing verification
or job interface when that execution contract is intended.

An explicit per-call requirement wins over `force_local`, disabled configuration,
and a falsy inherited `RCH_REQUIRE_REMOTE`; those conflicts refuse rather than
starting locally or overriding policy to start remotely. The native path's
historical environment-only local-config opt-out is preserved; existing shared
fallback guards remain in force. No process-global environment is modified, and a subsequent invocation
without the flag retains its own policy. Options after the workload's `--` remain
literal workload arguments, not controller flags. Windows refuses this flag
before starting a local command because its client lacks Unix daemon transport.

Remote-failure incident records reflect the same effective fallback policy as
execution. A refusal caused only by `compilation.allow_local_fallback=false`
records that configuration cause, not a fabricated strict-remote flag, and emits
an explicit refused result. Socket and requested-worker diagnostics cannot claim
local fallback is allowed when the corresponding execution gate forbids it.

Agents still issue ordinary tool commands. Once the hook resolves an existing
strict policy (`RCH_REQUIRE_REMOTE=1` or the dispatcher default without an explicit
opt-out), it includes `--require-remote` in each delegated native/shared prefix.
Thus the execution step need not inherit the hook's entire environment to keep
that requirement. The hook does not inject it into authored strings, ordinary
local commands, unrecognized grammar, or commands explicitly excluded by project
policy. This is propagation of a recognized delegation's policy, not a claim that
every shell command has been intercepted. Mixed compounds retain permission
checking and exact authored command bytes.

### Worker selection inventory

`rch --json diagnose --dry-run 'cargo test'` includes a `worker_selection.inventory`
when a supporting daemon supplies selection diagnostics. Each configured worker
is reported as `selected`, `eligible`, `fallback_candidate`, or `excluded`, with
human-readable reasons and stable reason codes. The selected/fallback verdict is
from that same preview round, not reconstructed from a later fleet snapshot.
Older daemons lacking inventory evidence leave the field absent; the client does
not invent an empty successful inventory.

The fork's existing one-active-project-per-worker rule is reported as
`project.active_build`, not a fictitious configurable lane limit. Source ownership,
live checkout and source-busy fences have distinct codes. An explicit requested
worker set takes precedence in diagnostics: other configured workers report
`selection.not_requested`, even if another exclusion also applies. This does not
relax actual selection or import the upstream multi-lane scheduler.

Dry-run queries may contact the daemon and perform its existing capability
checks, but they do not create builds, reserve execution slots, spend half-open
probe slots or append dispatch-audit entries. Real selection audit records remain
intact, and actual selections still record their normal audit entries. Inventory
explains the existing native diagnose request; it does not replace the hook's
per-component planner or qualify a shared mapping for execution.

### RTK logs and local fallback limits

RTK 0.46.0 integration recognizes `rtk run -c SOURCE`, `rtk run SOURCE`,
positional run arguments, `rtk proxy COMMAND...` and proxy shell commands.
It preserves the original RTK argv/source, cwd, environment and shell sequencing.
`run` joins its positional arguments as shell source, as RTK itself does; proxy
preserves argv. Both are passthrough, not compression commands. Filtered forms
include `rtk go test/build`, `rtk cargo test/build/check/clippy`, and
`rtk test bun test` / `rtk err bun build`. RTK 0.46 has no dedicated filtered Bun
subcommand: `rtk bun test` uses RTK's ordinary external-command behavior.
The hook, `rch exec -- rtk ...`, and optional `rch shim install --tools rtk`
recognize supported build/test workloads. Informational/non-build RTK commands
remain ordinary local commands. Use `general.force_remote`, dispatcher policy,
or `RCH_REQUIRE_REMOTE=1` to forbid local build/test fallback.

For a literal nested shell such as
`rtk proxy sh -c 'export GOFLAGS=-mod=mod; go test ./... && node --test probe.test.cjs'`,
the hook prepends `rch exec --verification --` to the whole RTK invocation.
If invoked without a hook, the installed RTK shim makes the same decision.
One remote job owns the shell, exports, cwd changes, short-circuit evaluation,
side effects and both tools. Worker recursion bypass prevents nested shims from
creating additional jobs. A failed first command skips the second; the shell's
exit status and full passthrough streams remain the job's result. Missing remote
completion retains recovery ownership and never replays the shell.

This covers literal shell source with exports and flat `;`, `&&` and `||`
sequences. Export-containing groups, pipelines, background commands and
unsupported control/lookup syntax still decline automatic rewriting. Top-level
caller exports combined with conditionals also remain unchanged: inserting
separate remote commands there would require branch-sensitive environment
forwarding. An empty hook response means execute the original command through
the configured PATH; it is not evidence that any workload ran or that a shim
failed. Do not treat possible inner-tool offloads as proof that the whole RTK
compound has one remote owner or a retrievable outer output record.

Remote RTK jobs capture the raw input to supported tool filters as well as the
outer stdout/stderr. RTK still performs its own filtering. RCH's byte capture
does not depend on RTK's minimum tee size, truncation, rotation or UTF-8 conversion.
It records each child argv/cwd/exit in JSON and saves separate stdout/stderr files.
Absolute executables in direct generic filtered forms are executed at that exact
path. Dynamic/custom wrappers and filters that launch an unrecognized executable
are not a promise of inner raw capture; the outer streams are still recorded.

The worker output directory is `<execution.storage.tmp_root>/rch-output/<job UUID>`
(or `<transfer.remote_base>/rch-output/...` when tmp placement is unset), outside
the ephemeral scratch sweep. Each directory is private and bound to the durable
job identity. It is collected over bounded SSH into `jobs.rtk_tee_root/<UUID>` on
the controller, defaulting to `job-leases/rtk-output/<UUID>` in RCH state. The
controller location must be outside source inputs and have no symlink ancestors.
It does not need a shared mapping and does not require remote-test's legacy tee
mount. Source mapping nonce and nested-mount checks remain unchanged.

RCH prints `[RCH] full RTK output: PATH` and reports that path in result metadata.
The raw-stream budget is 64 MiB per job, with at most 256 collected files. Exceeding
it fails the job explicitly; incomplete output is never reported as a full log.
RTK tracking defaults are placed under the job output directory too. These
retained directories are not automatically deleted; the operator owns retention.
Do not pass `--env RTK_TEE_DIR=...` or embed your own tee-directory override.
Python 3 and the requested RTK/tools must be available on the worker.

Raw shared execution uploads its generated shell program on a separate,
source-fenced preparation channel to `<completion>.launch` beneath
`transfer.remote_base`. It then invokes a short `sh <path>` command, leaving
every byte of execution stdin for the workload, including interactive input.
Launch files are private (0600), created exclusively without symlink traversal,
and use the same unique identity as the completion records. Keep the configured
transport root on the intended worker data filesystem; it need not be shared
with the controller. Completed launch files stay with transport evidence;
retirement cleanup includes them. Failed preparation cleanup may leave inert
private evidence if the worker cannot remove it, but cannot authorize replay.

The generated program is limited to 120 KiB, so nested shell arguments remain
below the Linux per-argument boundary. This limit applies after environment,
storage, capture and watchdog wrappers are generated, not to the authored
command length. Oversized programs fail before execution and retire the owned
preparation/reservation. There is no automatic fallback or retry. A definitive
local transport-spawn failure also retires preparation and records failed,
acknowledged ownership after source fencing and daemon release. It remains a
transport error, never a successful workload result. Once the transport child
exists, an exit or missing completion cannot prove that the workload never
started: RCH retains uncertain ownership for `jobs recover`, without replay.

After verified command completion, a log-transfer failure returns a delivery
failure and releases the job's slots; `rch jobs recover <wrapper ID>` retries
collection without rerunning the workload. Uncertain completion/cleanup retains
ownership and the existing no-replay rules. Timeout/SIGINT captures include bytes
already emitted before cancellation. Inspect actual retrieved files rather than
treating filtered console output as retention proof.

Authorized guarded-local RTK fallback uses the same wrapper/override validation,
but places managed `RTK_TEE_DIR` in the private local lease's `rtk-tee` directory,
not the shared tee root. That directory is exposed through the execution result's
`logs` field and keeps the lease retained for log access. RTK must also be
available locally when using this fallback.

The Linux local guard redirects managed home/cache/temp locations into a private
lease under `jobs.local_guard_root` (or the user's `rch/local-jobs` cache default).
Its current default checks are a 2 GiB job high-water mark and a 4 GiB free-space
floor, monitored periodically. These are cooperative limits, **not hard quotas**:
a command can write outside managed paths, overshoot between checks, or block in
filesystem I/O. Explicit overrides of guard-owned home/cache/temp variables are
refused; this fallback does not simply reuse every remote cache setting.

The general verification fallback currently supplies no concurrency cap and no
default per-invocation timeout; pass `--timeout` when a deadline is required.
Do not infer bounded CPU, memory, filesystem containment, or total retained-disk
usage from the word "guarded". The implementation uses a dedicated Linux child
subreaper and requires its kernel `ECHILD` observation plus successful supervisor
exit before reclaiming identity-checked scratch. A successful workload leader
exit or group signal alone is not proof of descendant quiescence. Requested
retention, captured logs, managed RTK logs, unproven quiescence, or cleanup failure
retain the lease rather than treating it as disposable. Unavailable `/proc`,
unreapable D-state processes, exhausted supervision budgets, a dead supervisor,
or truncated control messages leave quiescence unproven: the controller reports
a control failure (exit 125) and retains scratch. Abrupt controller loss closes
the control channel and may still trigger reaping, but without a received receipt
scratch remains retained. Raw PTY input has no stream half-close on stdin EOF;
the workload must exit through its terminal protocol or an explicit deadline.

Deployment acceptance of cleanup after normal exit, signals, timeout,
cancellation and disconnected controllers remains the operator's responsibility
on the actual workers and deployed revision. This implementation description is
not a complete migration or release-readiness claim. Preserve retained leases
until ownership and safe cleanup are established.

### Deployment validation (not a readiness claim)

For an actual release installation, use the fork's installer with an explicit
version and `RCH_SIGNATURE_POLICY=required`. Keep the prior installed release
available for rollback. Check controller, daemon and worker versions together;
a checksum-only fixture is not proof of a deployed fork release or its source
commit. No built/signed replacement candidate or completed live migration is
asserted by this guide.

For one actual workload, the opt-in acceptance entrypoint rejects local fallback,
mock environments, wrong workers, infrastructure failures and mismatched exit
codes. It does not install hooks or provision workers. Create an evidence root
outside the project once, then use an already configured worker:

```bash
python3 -I scripts/validate_remote_execution.py \
  --rch /absolute/path/to/rch --project /absolute/path/to/project \
  --worker WORKER_ID --verification \
  --evidence-root /absolute/path/to/existing/evidence -- pytest -q
```

Repeat for warm-cache execution. For an intentionally failing test use
`--expected-exit 101` (or the workload's actual expected code). Each invocation
gets a new evidence directory holding `result.json`, process status, and separate
`stdout.bin`/`stderr.bin` captures with `--verification`. Omitting that flag tests
the older compiler/mirror contract using a merged `workload.log` instead. A
refusal, missing binary, or missing worker is a failure, never an accepted skip.
This validates one completed invocation, not byte-for-byte stream equivalence,
cancellation, descendant cleanup, cache identity, storage quotas or the entire
migration.

Before retiring remote-test, separately verify:

- All authored command families and their produced files, including Python,
  Swift, npm/pnpm/Bun scripts and Go output modes. The semantic hook routes
  recognized verification components through the shared-workspace invocation
  path; writes within the mapped tree are shared, not downloaded artifacts.
  Arbitrary external paths are not automatically mapped or sandboxed. Script
  recognition is bounded (test/build/lint/typecheck/check/verify/vet names and
  supported suffixes), not blanket recognition of every package script.
  Unsupported shell grammar stays unchanged rather than being guessed.
- Codex `~/.codex/hooks.json`, Claude settings and the Hermes plugin are loaded
  by actual runtimes; unrelated hooks survive. Preview changes first. Do not
  run both legacy and replacement rewrite handlers on the same command.
- Direct shell/CI and nested subprocess PATH resolution. Install the generic
  set explicitly with `rch shim install --tools all`, not only Cargo shims.
  `rch ci` changes the child process environment, never an existing parent shell.
- Literal argv, working directories, env overrides, shell operators, native
  stream separation, PTYs, signals, timeout, cancellation and disconnect. Use
  `--json-result FILE` for typed invocations and explicitly check JSON-mode
  refusal; do not substitute the older compiler machine-output contract.
- RTK filtered output and actual raw log access through RCH's job-owned output
  collection, including recovery without replay; console output alone is not proof.
- Persistent package caches/Nexus credentials, per-job scratch isolation and
  cleanup after failure/cancel, short Unix sockets, disk-pressure handling and
  bounded local fallback. `execution.storage` configures placement, not quotas.
- A rollback that restores the original hooks, PATH and versioned release
  without discarding active jobs or other agents' settings.

For this deployment, keep shared package/build caches under
`/mnt/hdd-vibe-coding-share`. Reuse the existing project/user cache directories
without clearing them. Verify the executing user's write access to the cache
and its existing shard directories: an old root-owned Go cache can make module
index writes fail with `EACCES`, surfaced misleadingly as `package fmt is not in
std`. Choose an existing user-owned cache on the same mount instead of deleting
or recursively changing ownership of someone else's cache. For example, keep
`CARGO_HOME`, `CARGO_TARGET_DIR` and `GOCACHE` on that mount; use separate per-job
scratch directories and report
`TMPDIR` explicitly in validation results. A local `/tmp` comparison is diagnostic
only, not acceptance for shared scratch. Do not change NFS mount settings or
weaken cancellation deadlines to make a test pass. Direct group signaling avoids
spawning `kill(1)` during cancellation, but cannot force blocked filesystem I/O
to finish or make a D-state process immediately reapable.

The optional RABS worker toolchain pool publishes a synced tree with an
exclusive `.complete` marker, rather than relying on `RENAME_NOREPLACE`, which
this NFS mount rejects. Lookup still verifies the complete tree and its identity;
the marker alone never makes a hit. Unmarked captures, including older unmarked
entries, remain charged against retention limits and are not repaired, overwritten
or deleted automatically. Inspect retained entries with the worker stopped before
any operator-approved cleanup. This pool is separate from Cargo/Go package caches.

Mocked SSH, local process regressions and this validator's `--self-test` are
repository evidence only. Linux acceptance does not require fixing unrelated
macOS/Windows releases.

## Pre-upgrade working-tree reconciliation — 2026-10-22

The old custom checkout contained 70 modified tracked files and 13 untracked
files. The verified 83-file recovery snapshot is retained privately; the original
checkout is not a deployable replacement release and is not overwritten during
integration. Preservation does not mean blindly importing its older files.

| Original snapshot group | Files | Disposition |
| --- | ---: | --- |
| RCH runtime, agents, configuration, daemon and invocation modules/tests | 58 | Reconcile onto the newer main in PR #4, including the already committed source-root port. Combined runtime compilation and execution acceptance remain separate gates. |
| Installer, release workflow, documentation, packaging and acceptance tools | 12 | Retain useful behavior with main's stronger source/tag/checksum checks. Support checkpoint `538cdd28` supplies these changes for the same replacement PR. |
| RABS process/stream control, jobserver and persistent toolchain cache | 7 | Retain after reconciling newer ownership/EOF tests. Direct safe kernel signals avoid launching a cancellation helper; completion markers avoid a directory-rename primitive unsupported by the tested NFS mount. |
| Old CI, durable-execution and retired source-execution workflow edits | 3 | Do not restore the old graph. Move useful checks into the retained CI/E2E owners, preserving the 12-workflow layout and the newer read-only recovery owner. |
| Old `Cargo.lock` | 1 | Do not copy old package versions. Resolve only the necessary dependency edges against the security-patched merged lockfile. |
| `scripts/tests/test_durable_execution.py` | 1 | Reconcile with the runtime formatter, retaining both raw stdin/interactive cases and main's cancellation/saturated-writer regressions. |
| `scripts/tests/test_recovery_workflows.py` | 1 | Exclude: it requires the retired `source-execution-recovery.yml` and would incorrectly require reintroducing obsolete infrastructure. |

Discarded changes are excluded from the new implementation, not erased from the
recovery archive. In particular, the old lockfile and stale workflow requirements
must never be used to overwrite the newer security fixes or validation structure.

The RABS support review preserved main's unreaped-child formation checks and
isolated retained-writer EOF assertions. On `u25-agent-test`, pinned nightly
validation passed 21 process-group, 12 stream-drain, 10 jobserver, 17 durable-cache
and 3 dependency-budget tests. The 17 durable-cache and 10 jobserver tests also
passed with fixtures on the actual NFS shared mount. A same-filesystem comparison
observed the prior no-replace directory publication succeed under ordinary temp
storage but return EINVAL on that NFS mount; the new cache path succeeded and
reopened the exact verified objects in both cases. This is evidence for those
fixtures and that mount, not power-loss, fleet performance or deployment proof.

The initial aggregate validation reached successful strict Clippy and process/
stream tests, then exhausted its command budget during worker-test compilation.
The remaining worker suites completed separately without changing product
limits. Evidence is retained under `target/overlay-rabs-validation-M7VUq8JU/`
and `target/overlay-rabs-worker-GPPmJ6fY/`; worker job
`rt-20261022-050224-jmql8lzx` completed with exit zero. Installer/packaging and
source-policy evidence is recorded in PR #4. Synthetic packaging tools and source
contract checks must not be described as actual release or cryptographic tests.

Final pinned formatting, full-workspace/all-target `cargo check --locked` and
full-workspace/all-target/all-feature Clippy with `-D warnings` passed in
`rt-20261022-051210-t1hz69nq`; exact source hashes are retained in
`target/overlay-workspace-final-2CBSLDLm/source.sha256`. The adapter adds one
explicit, already-locked `rustix` process dependency to use safe kernel signal
APIs; its direct-dependency budget changes from four to five for that concrete
addition. No package version, other crate budget, timeout or warning gate changes.

This review does not authorize deployment. The operator owns activation after
the complete committed replacement is validated; support-only checkpoints are
not substitutes for the existing code1 tool shims, agent hooks and worker paths.

## Pre-Migration Checklist

Before starting:

- [ ] Identify worker machines (existing servers, cloud VMs, spare desktops)
- [ ] Ensure SSH access to workers
- [ ] Verify workers have sufficient resources (CPU, RAM, disk)
- [ ] Document current build times as baseline

## Step 1: Infrastructure Setup

### Install RCH on Workstation

```bash
# From source
git clone https://github.com/tsunheimat/remote_compilation_helper.git
cd remote_compilation_helper
cargo build --release
cp target/release/rch target/release/rchd ~/.local/bin/

# Or via cargo
cargo install --git https://github.com/tsunheimat/remote_compilation_helper.git
```

### Configure Workers

```bash
mkdir -p ~/.config/rch

cat > ~/.config/rch/workers.toml << 'EOF'
[[workers]]
id = "worker1"
host = "your-worker-ip"
user = "your-username"
identity_file = "~/.ssh/id_rsa"
total_slots = 8
priority = 100
EOF
```

### Deploy to Workers

```bash
# Install RCH on workers
rch fleet deploy

# Verify
rch fleet verify
```

### Start the Daemon

```bash
rch daemon start
```

## Step 2: Install Hooks

### Claude Code

```bash
rch agents install-hook claude-code --dry-run
rch agents install-hook claude-code
```

This adds RCH to Claude Code's PreToolUse hooks for a fresh installation. If a
legacy remote-test handler is present, use the explicit `agents migrate` procedure
above instead of adding a second handler.

### Verify Hook Installation

```bash
rch hook status
```

This reports installed configuration, not whether a running agent has loaded it.
Reload the agent as needed and verify a real tool invocation before declaring
activation complete.

## Step 3: Validation

### Test Classification

Verify commands are classified correctly:

```bash
# Should be REMOTE
rch classify "cargo build --release"
rch classify "cargo test"
rch classify "cargo check"

# Should be LOCAL
rch classify "cargo fmt"
rch classify "cargo install foo"
rch classify "cargo clean"
rch classify "git status"
```

### Test End-to-End

Run a build with verbose output:

```bash
cd /path/to/rust/project
RCH_VERBOSE=1 cargo build
```

You should see:
1. Command classification
2. Worker selection
3. Transfer to worker
4. Remote execution
5. Artifact retrieval

### Verify Artifacts

After remote build:

```bash
# Check artifacts are present
ls -la target/release/

# Run the binary
./target/release/your-app --version
```

## Step 4: Canonical Topology Rollout Plan (`/data/projects` + `/dp`)

This phase is specifically for migrating existing workers to the canonical
filesystem topology required by reliability workstreams:

- `/data/projects` must exist as a directory
- `/dp` must be a symlink resolving to `/data/projects`
- workers failing topology preflight must be excluded from remote scheduling

### Phase 0: Fleet Inventory and Segmentation

1. Capture a baseline before making any changes:
```bash
rch workers list
rch workers probe --all
rch status --workers
```
2. Segment workers into rollout cohorts (for example by region or host group):
- `canary` (5-15% of fleet)
- `batch-a` (next 30-40%)
- `batch-b` (remainder)
3. Record cohort membership in your operations notes before execution.

### Phase 1: Canary Cohort

Run migration only on canary workers first:

```bash
# Run bootstrap/enforcement for a single canary worker or a small set.
rch workers setup --worker <canary-id>

# Re-verify health and topology eligibility.
rch workers probe <canary-id>
rch status --workers
```

Go/no-go rule for Phase 1:
- Proceed only if all canary workers are healthy or have explicit, understood,
  non-topology issues unrelated to migration.

### Phase 2: Rolling Batch Migration

Migrate the remaining cohorts in bounded batches. After each batch:

1. Run worker setup for that cohort.
2. Re-check worker probe + status.
3. Stop immediately on repeated topology integrity failures.

### Phase 3: Full-Fleet Validation and Stabilization

After all cohorts are migrated:

- Run full worker health checks.
- Run representative compile/test workloads.
- Confirm no persistent preflight exclusion for topology reasons.

## Mixed-State Worker Handling (Critical)

During migration, mixed states are expected temporarily. Treat them explicitly:

| State Detected | Meaning | Required Action |
|---|---|---|
| `projects_root_ok=true` | Topology healthy | Keep worker eligible |
| `alias_missing` | `/dp` missing | Re-run setup, verify symlink creation |
| `alias_wrong_target:*` | `/dp` points elsewhere | Repoint alias, re-run probe |
| `alias_not_symlink` | `/dp` exists but wrong type | Manual repair (safe remove/recreate symlink) |
| `canonical_not_directory` | `/data/projects` exists but invalid type | Manual integrity remediation before retry |
| `canonical_missing` | canonical root absent | Re-run setup, confirm directory creation |

Workers with topology failures must remain excluded from remote scheduling until
explicit revalidation confirms recovery.

## Validation Checklist (Per Phase Gate)

Use this checklist before promoting to the next phase:

- [ ] `rch workers probe` succeeds for target cohort
- [ ] No unresolved topology-preflight failure reasons in `rch status --workers`
- [ ] At least one representative `cargo build` and `cargo test` run succeeds
- [ ] No unexpected fail-open spikes during the phase window
- [ ] Operator notes include worker IDs, timestamps, and remediation actions

## Dry-Run Procedure + Evidence Template

Before each production phase, run and archive a dry-run evidence bundle:

```bash
# Example evidence capture commands
date -Iseconds
rch workers list
rch workers probe --all
rch status --workers
```

Evidence template (store in your rollout notes):

```text
Phase: <canary|batch-a|batch-b|final>
Timestamp: <ISO8601>
Cohort Workers: <comma-separated ids>
Preflight Summary: <healthy/degraded counts>
Topology Failures: <none | list of worker->reason>
Actions Taken: <commands + remediation>
Go/No-Go Decision: <go|no-go>
Approved By: <operator>
```

## Step 5: Performance Tuning

### Baseline Comparison

Compare build times:

```bash
# Local build (RCH disabled)
RCH_ENABLED=false time cargo build --release

# Remote build
RCH_ENABLED=true time cargo build --release
```

### Initial Sync

First sync to a worker is slow (full project transfer). Subsequent syncs are incremental.

Tips to minimize initial sync:
- Ensure `.gitignore` includes `target/`
- Add large data directories to excludes
- Consider pre-warming worker caches

### Optimize Excludes

```toml
# ~/.config/rch/config.toml
[transfer]
exclude_patterns = [
    "target/",
    ".git/",
    "node_modules/",
    "benches/data/",
    "test_fixtures/large/",
]
```

### Adjust Compression

For different network conditions:

```toml
# Fast local network (100Mbps+)
compression_level = 1

# Moderate network (10-100Mbps)
compression_level = 3

# Slow network (<10Mbps)
compression_level = 9
```

## Common Migration Issues

### "Build runs locally instead of remote"

1. Check daemon is running: `rch daemon status`
2. Check hook is installed: `rch hook status`
3. Check command classification: `rch classify "your command"`
4. Check confidence threshold: lower if needed

### "Build slower than local"

Expected for:
- Small projects (overhead exceeds benefit)
- First build (initial sync is slow)
- Poor network to workers

Solutions:
- Increase `min_local_time_ms` to skip small builds
- Add more workers
- Improve network connectivity

### "Build fails remotely but works locally"

Common causes:
1. Missing tools on worker (install via `rch fleet deploy`)
2. Different Rust version (sync toolchains)
3. Platform-specific code (workers must be same platform)
4. Missing environment variables

Debug with:
```bash
RCH_LOG_LEVEL=debug cargo build 2>&1 | tee build.log
```

### "Artifacts missing after build"

Check transfer patterns include your artifacts:

```toml
# .rch/config.toml
[transfer]
include_artifacts = [
    "target/release/*",
    "target/debug/*",
]
```

## Rollback Plan

If migration causes instability or inconsistent topology states, roll back in
reverse cohort order.

### Rollback Triggers

Initiate rollback when any of the following are true:

- sustained topology preflight failures after remediation attempts
- repeated `alias_wrong_target` / `alias_not_symlink` reoccurrence in the same cohort
- materially elevated fail-open rate attributable to topology drift
- operator cannot prove canonical invariants for current rollout segment

### Rollback Steps

1. Freeze rollout immediately (no new cohorts).
2. Remove affected workers from routing (drain/disable) until repaired:
```bash
rch workers drain <worker-id>
```
3. Restore previously known-good worker topology configuration for the affected cohort.
4. Re-run probe/status validation:
```bash
rch workers probe <worker-id>
rch status --workers
```
5. Re-enable workers only after explicit verification:
```bash
rch workers enable <worker-id>
```
6. If fleet-wide instability persists, temporarily disable RCH for local fallback:
```bash
export RCH_ENABLED=false
```

### Rollback Completion Criteria

- All affected workers are either healthy and revalidated, or intentionally drained.
- No unknown topology-preflight failures remain.
- Incident notes include root cause, corrective action, and follow-up owner.

## Post-Migration

### Document the Setup

Record:
- Worker configurations
- Tuning decisions made
- Known issues and workarounds

### Set Up Monitoring

- Add health checks (see Monitoring Guide)
- Configure alerts for daemon/worker issues
- Track build performance over time

### Team Onboarding

Share with team:
1. RCH overview and benefits
2. How to check if RCH is working
3. How to bypass RCH if needed
4. Who to contact for issues

## Comparison: Before vs After

| Aspect | Before (Local) | After (RCH) |
|--------|---------------|-------------|
| Build CPU | Workstation | Workers |
| Build parallelism | Limited by local cores | Limited by fleet cores |
| Concurrent agents | ~2-3 before throttling | Many (worker-limited) |
| Workstation responsiveness | Degraded during builds | Maintained |
| Setup complexity | None | Moderate (one-time) |
| Dependencies | Local tools | Tools on workers |

## Success Criteria

Migration is successful when:
- [ ] Builds complete successfully via RCH
- [ ] Build times are acceptable (≤ 15% overhead typical)
- [ ] Multiple agents can build concurrently
- [ ] Workstation remains responsive during builds
- [ ] Team is trained on RCH operation
- [ ] Monitoring and alerts are in place
