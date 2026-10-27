# Configuration Guide

This guide consolidates how RCH configuration works, where files live, and how
values are resolved when multiple layers exist.

## Overview

RCH has three primary configuration surfaces:

1. **Hook/CLI config** (`config.toml`) for interception, transfer, and circuit
   breaker behavior.
2. **Worker definitions** (`workers.toml`) for daemon-managed worker inventory.
3. **Daemon settings** (`daemon.toml`) for health checks and socket behavior.

## Precedence (Hook/CLI Config)

For `rch` (the hook/CLI), the effective configuration is resolved in this order
(lowest to highest):

1. Built-in defaults
2. User config: `~/.config/rch/config.toml`
3. Project config: `.rch/config.toml`
4. Environment variables (RCH_*)
5. Command-line flags (when a command supports them)

Tip: use `rch config show --sources` to see where each value came from.

Note: the config library includes support for `.env` / `.rch.env` files and
profile presets (via `RCH_PROFILE`), but the current `rch` loader only applies
user/project config + env overrides. Verify your effective values with
`rch config show --sources`.

## Hook/CLI Config File (`config.toml`)

Location:
- User: `~/.config/rch/config.toml`
- Project: `.rch/config.toml`

Sections and fields:

### Worker cache, scratch and environment placement

Set these once in the controller's `~/.config/rch/config.toml`; project config
may override individual keys. They apply to the remote execution path used by
`rch exec`, `rch exec --job` and the hook. They do not configure standalone
`rch-wkr execute`, canary self-tests, source transfer staging or the RABS sidecar.

```toml
[execution.storage]
root = "/srv/rch"                 # worker path, preferably on the worker SSD
# cache_root = "/srv/rch/cache"  # overrides root/cache
# tmp_root = "/srv/rch/tmp/jobs" # overrides root/tmp/jobs
# home_root = "/srv/rch/home"    # optional; HOME otherwise stays unchanged
tmp_mode = "env"                 # or "private_mount" on Linux
tmp_retention_hours = 24         # abandoned-job sweep; 0 disables it

[environment.remote]
GOPROXY = "https://nexus.example/repository/go-proxy/"
NPM_CONFIG_REGISTRY = "https://nexus.example/repository/npm-group/"
PIP_INDEX_URL = "https://nexus.example/repository/pypi-group/simple"

[remediation.pooled_target]
store_base = "/srv/rch/targets"  # existing Cargo target-pool configuration
```

With all storage paths unset, native placements remain unchanged. `cache_root` and
`tmp_root` can also be used independently. Paths are absolute POSIX worker
paths; no controller-side tilde, variable or shell expansion is performed.
Configured storage is refused on Windows workers. For a mixed fleet, scope
this configuration to projects routed to POSIX workers.

| Variable | Under the configured cache root |
|---|---|
| `XDG_CACHE_HOME` | `xdg` |
| `CARGO_HOME` | `cargo-home` |
| `GOCACHE`, `GOMODCACHE`, `GOPATH` | `go-build`, `go-mod`, `go-path` |
| `NPM_CONFIG_CACHE`, `BUN_INSTALL_CACHE_DIR` | `npm`, `bun` |
| `UV_CACHE_DIR`, `PIP_CACHE_DIR` | `uv`, `pip` |
| `PLAYWRIGHT_BROWSERS_PATH` | `playwright` |

Package caches persist across jobs. Cargo target keys, target-pool reuse and
artifact retrieval still use RCH's native target policy. A new `CARGO_HOME`
does not copy the old registry credentials/configuration: provision the new
directory if those are required. Changing `HOME` is opt-in because it affects
tool discovery and credentials; explicitly set worker `RUSTUP_HOME` in
`environment.remote` if rustup toolchains remain under the original home.

Remote environment values are literal defaults. Allowlisted controller values
override those defaults; RCH-managed cache/tmp/target paths override both.
Managed placement also overrides the corresponding `--env`/`--forward-env`
values on typed/shared invocations. Other explicit variables retain their values.
Explicit assignments *inside the authored command* retain normal shell
semantics. This is placement policy, not a security sandbox for arbitrary code.
`CARGO_TARGET_DIR`, `RCH_CH_BASE` and the recursion-bypass variable cannot be
set through `environment.remote`. Profile values may contain credentials, so
`config show/get` reports their names and `(set)` rather than their values.
Use worker-side credential provisioning instead of committing secrets.

For a shared-source controller with HDD worker storage, setting only `TMPDIR`
does not relocate language caches. Use the existing placement settings on the
controller (paths below are worker paths):

```toml
[execution.storage]
cache_root = "/mnt/hdd-vibe-coding-share/rch-worker/agnet-test/cache"
tmp_root = "/mnt/hdd-vibe-coding-share/rct"
tmp_mode = "private_mount"

[jobs]
# Controller destination for explicitly retrieved RTK output; no shared mapping.
rtk_tee_root = "/mnt/hdd-vibe-coding-share/rch-controller/rtk-output"
```

Keep the worker tool `PATH` in `[environment.remote]`. `home_root` is optional;
cache placement does not require changing HOME, moving sources or importing
existing user caches. Both native mirrors and shared jobs use `cache_root` for
Go build/module caches, Cargo package cache and Bun's install cache. Go's
resolved `go env GOCACHE GOMODCACHE GOPATH` and actual writes establish placement;
the absence of a particular environment variable does not. Native Cargo target
pools and shared Cargo's authored target outputs retain their existing contracts.

Controller-side native Cargo planning invokes `cargo metadata --locked`. Cargo
may update its package index/cache there even though compilation is remote.
Worker storage settings deliberately do not select controller paths. To keep
that cache off the controller root disk, launch RCH/hooks/shims with the normal
Cargo setting `CARGO_HOME=/mnt/hdd-vibe-coding-share/rch-controller/cargo-home`.
Use a separate controller directory; no existing cache migration is necessary.
RCH still loads Cargo configuration from that home, so operators must preserve
any required registry configuration/credentials when choosing it. Verify actual
controller and worker writes separately. `RCH_STATE_HOME` likewise selects the
controller's RCH journals; it is independent of worker cache placement.

Managed tmp creates a unique `rch-job-<UUID>/tmp` outside the source mirror,
exports `TMPDIR`, `TMP` and `TEMP`, and inherits a lease into the existing RCH
watchdog/process group. The worker needs `flock`, `find`, `mktemp`, hard-link
support on the scratch filesystem, and the `/dev/fd` descriptor view. Cleanup after
success or failure only removes that job directory after its lease is free;
descendants retaining the inherited lease keep it. A killed supervisor or
interrupted connection may leave the directory for the next age-and-lock
sweep. The sweep never kills processes or infers a build's completion. Recovery receipts and result
directories remain in their original locations.

`private_mount` additionally uses `unshare --mount --propagation private` and
binds the job scratch directory over `/tmp`. It requires Linux mount namespace
privileges and the `unshare`/`mount` tools. Missing privileges fail before the
workload runs; there is no silent downgrade or sudo invocation. Storage paths,
source mirrors, target pools and worker tools must resolve outside `/tmp`,
since the private mount hides its original contents. RCH checks the physical
source, target and managed storage paths before starting the workload, including
symlink aliases. Only the workload enters the namespace; the native watchdog
and its `/tmp/rch-run` process record remain in the worker's normal namespace
for cancellation and orphan recovery. Cancellation still targets the same
process group.
If only `tmp_root` is configured, the existing native Cargo cache must also
resolve outside `/tmp`. RCH refuses a hidden native or explicit `CARGO_HOME`
before Cargo runs; configure `cache_root` or a persistent
`environment.remote.CARGO_HOME` when the worker's native cache base is `/tmp`.
`env` mode does not redirect programs that hardcode `/tmp`.

Source mirrors, cache-warm staging and controller storage retain their existing
configuration. Package caches are not swept by the job-tmp cleaner. Existing
disk-pressure telemetry must still observe the volume you use: place the
worker canonical mirror and these paths on that monitored volume, and use
the tool's own cache cleanup for persistent package caches. This setting
does not install a filesystem, impose a quota or guarantee that `/srv/rch`
is disk-backed.

Inspect or change individual keys with, for example:

```bash
rch config set execution.storage.root /srv/rch
rch config set environment.remote.GOPROXY https://nexus.example/repository/go-proxy/
rch config get execution.storage.root --sources
rch config validate
```

### `[general]`
- `enabled` (bool, default `true`) — Master on/off switch for the hook.
- `log_level` (string, default `"info"`) — `trace|debug|info|warn|error`.
- `socket_path` (string, default `"$XDG_RUNTIME_DIR/rch.sock"` if set, otherwise
  `"~/.cache/rch/rch.sock"`; falls back to `"/tmp/rch.sock"`) — Unix socket path
  used to communicate with the daemon.

### `[compilation]`
- `confidence_threshold` (float, default `0.85`) — Minimum classifier confidence
  to intercept a command.
- `min_local_time_ms` (u64, default `2000`) — Skip interception if the estimated
  local runtime is shorter than this. Uses timing history from past builds.
- `remote_speedup_threshold` (float, default `1.2`) — Minimum predicted speedup
  ratio (local/remote) required for offloading. Set to `1.0` to always offload
  when other criteria are met. Set higher (e.g., `1.5`) to only offload builds
  predicted to be significantly faster remotely.

### `[transfer]`
- `compression_level` (u32, default `3`) — zstd compression level.
- `exclude_patterns` (list) — Patterns excluded from transfer. Defaults include:
  `target/`, `.git/`, `node_modules/`, common build caches, and
  coverage output. Use `rch config show` to see the full effective list.
- `ssh_server_alive_interval_secs` (u64, optional) — Sets `ssh -o ServerAliveInterval`
  for remote execution and rsync transfers to reduce dropped connections on flaky networks.
- `ssh_control_persist_secs` (u64, optional) — Sets `ssh -o ControlPersist=<N>s` for
  remote execution (ControlMaster). Use `0` to disable persistence (`ControlPersist=no`).
- `sync_timeout_ms` (u64, optional; `1000..=3600000`) — Per-attempt timeout for
  uploading source to the worker. When unset, RCH uses a payload-aware default:
  30 seconds plus one second per MiB at a conservative 1 MiB/s floor, capped at
  one hour. Each retry receives this full timeout and resumes the same partial
  immutable-base upload. This setting does not affect remote Cargo execution or
  artifact-return timeouts.
- `source_sync_silence_timeout_secs` (u64, default `120`) — Abort a source sync
  after this many seconds with no rsync output at all (no progress refresh,
  stats, or itemized line). This detects a dead channel or wedged rsync far
  faster than the wall-clock `sync_timeout_ms` cap, while a large but
  progressing transfer is never affected: every `--info=progress2` refresh
  counts as forward progress. On a stall, the hook releases the worker's
  reservation and retries the build on another admissible worker. `0` disables
  silence detection.
- `rsync_bin` (string, optional) — Explicit rsync binary for every local
  transfer. Unset (the default) lets RCH resolve one: the `PATH` rsync when it
  is rsync 3.1 or newer, otherwise a modern rsync from
  `/opt/homebrew/bin`, `/usr/local/bin`, or `/opt/local/bin`, otherwise the
  `PATH` binary driven with an openrsync/rsync 2.6.9-compatible flag set (no
  `--info=*`, zlib instead of zstd, no `--append-verify`; the zero-build-output
  detector fails open). `~` is expanded and a bare name is looked up on `PATH`.
  The `RCH_RSYNC_BIN` environment variable overrides this setting. `rch doctor`
  shows the resolved binary and flavour.

### `[circuit]`
- `failure_threshold` (u32, default `3`) — Consecutive failures to open.
- `success_threshold` (u32, default `2`) — Consecutive successes to close.
- `error_rate_threshold` (float, default `0.5`) — Error rate to open in window.
- `window_secs` (u64, default `60`) — Rolling error window size.
- `open_cooldown_secs` (u64, default `30`) — Cooldown before half-open.
- `half_open_max_probes` (u32, default `1`) — Concurrent probes allowed.

Example:

```toml
[general]
enabled = true
log_level = "info"
socket_path = "~/.cache/rch/rch.sock"

[compilation]
confidence_threshold = 0.85
min_local_time_ms = 2000
remote_speedup_threshold = 1.2

[transfer]
compression_level = 3
exclude_patterns = ["target/", "node_modules/"]
ssh_server_alive_interval_secs = 30
ssh_control_persist_secs = 60

[circuit]
failure_threshold = 3
success_threshold = 2
error_rate_threshold = 0.5
window_secs = 60
open_cooldown_secs = 30
half_open_max_probes = 1
```

### `[path_topology]`
- `source_root` (string, optional) — Controller source tree when sources and
  worker mirrors occupy different namespaces. Projects and relative path
  dependencies keep their paths relative to this root under `canonical_root`.
  Source and mirror roots must be disjoint, including resolved symlinks.
- `canonical_root` (string, optional, default `"/data/projects"`) — Canonical
  project root directory. Override this on systems where `/data/projects` is
  unavailable (e.g., macOS with SIP).
- `alias_root` (string, optional, default `"/dp"`) — Symlink alias root that
  points at the canonical root.

Environment variable overrides:
- `RCH_CANONICAL_PROJECT_ROOT`
- `RCH_ALIAS_PROJECT_ROOT`
- `RCH_SOURCE_PROJECT_ROOT`

Example (macOS):

```toml
[path_topology]
canonical_root = "/Users/me/Projects"
alias_root = "/Users/me/p"
```

For repositories on a shared source mount and a separate native worker mirror:

```toml
[path_topology]
source_root = "/mnt/vibe-coding-share"
canonical_root = "/mnt/hdd-vibe-coding-share/rch-worker/agnet-test/projects"
alias_root = "/mnt/hdd-vibe-coding-share/rch-worker/agnet-test/projects-alias"

[compilation]
native_toolchain = "1.93.1"

[environment.remote]
PATH = "/opt/rch/toolchains/clippy-1.93.1/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
```

The ordinary Cargo/clippy PATH shims and `rch exec -- cargo test` use this
mapping. Sources remain on the source mount; rsync writes only to the separate
mirror, builds use worker output directories, and the existing native artifact
contract returns outputs to the originating repository. Worker preflight also
rejects mirror destinations that resolve into source or outside the mirror.
Use relative `--manifest-path`, workspace-member and path-dependency paths with
this mapping. Absolute authored Cargo source paths are refused because copying
them unchanged would select the original shared source on the worker. Optional
repository-updater convergence is skipped for this mapping; rsync updates the
owned mirror directly.

`compilation.native_toolchain` explicitly chooses an exact native Rust version
for unpinned projects on workers without rustup. Both admission and execution
check `rustc --version`, Cargo and clippy availability; this option does not
install tools. Set `environment.remote.PATH` to expose private native tools;
the ordinary SSH login PATH need not contain them. Use a complete literal PATH
(including system utilities), without `$PATH` expansion. Native Cargo/clippy
and explicit shared execution both apply these remote defaults.

Rust admission carries the caller's resolved remote environment, including
project overrides, allowlisted controller values and configured cache/home
placement. The daemon applies it to the entire toolchain probe. Probe caches
and queued recovery are bound to that environment, so one PATH cannot reuse
another PATH's verdict. With a configured native PATH, the exact-version probe
is authoritative instead of the ordinary-PATH compiler inventory.
Install matching `rch` and `rchd` builds for this
contract. Older callers use the daemon's startup `environment.remote` defaults;
changing those defaults for older callers requires a daemon restart.
Admission does not allocate job scratch: managed tmp/target placement still
occurs only after ownership is established.
Command/project toolchain pins and `RUSTUP_TOOLCHAIN` are refused
under this policy. Leave it unset for the existing rustup toolchain contract.
This does not establish compatibility with projects requiring nightly Rust.

Keep execution scratch separate from both source and mirror. On the existing
shared-mount installation, retain the short worker `tmp_root` of
`/mnt/hdd-vibe-coding-share/rct`, `tmp_mode = "private_mount"`, and the managed
`TMPDIR` rewriting. Changing source topology does not require moving caches
or lengthening Unix socket paths.

For explicit shared jobs, a requested `--timeout` that finishes verified owned
process cleanup, durable completion and byte-stream delivery returns exit 124
with `outcome = "deadline_exceeded"`. SIGINT/SIGTERM uses exit 130 and
`outcome = "interrupted"`. The daemon receives that same terminal status and
releases this job's slots before the CLI reports completion. Ordinary workload
exit 137 is still a completed nonzero command. Missing completion, unverified
cleanup, output delivery failure or an unacknowledged release remains a control
failure with retained recovery ownership and no automatic replay.

### `[api]` — daemon status API over the tailnet

`rchd`'s rich state (`/status`, `/workers/capabilities`) lives on a `0600` Unix
socket that only the local user can read. This section serves the same JSON on
a TCP listener so agents and the fleet dashboard on *other* machines can ask a
dispatcher "why are you local-only right now?" without ssh. It is designed for
a Tailscale tailnet and is **off** unless `bind` is set.

- `bind` (string, default `""` = off) — `"tailscale"` resolves this machine's
  Tailscale IPv4 at daemon start (port 9101); `"tailscale:PORT"`, `"IP:PORT"`
  or `"[v6]:PORT"` are taken literally. The address must be loopback or inside
  Tailscale's ranges (`100.64.0.0/10`, `fd7a:115c:a1e0::/48`); anything else is
  refused at startup — the payload carries worker hosts and IPs.
- `token` (string, optional) — bearer token required on `/status`,
  `/workers/capabilities`, `/workers/config`,
  `/repo-convergence/status[?worker=<id>]`. Sent as
  `Authorization: Bearer <token>` or `X-Rch-Token: <token>`.
- `token_file` (string, optional) — file holding the token (trimmed, `~`
  expanded). Wins over `token`. Keep it `0600`.
- `no_token` (bool, default `false`) — serve the status routes with no token.
  Honoured only when no token is configured and must be set explicitly, so an
  empty token is never an accidental open door.
- `allow_any_addr` (bool, default `false`) — permit a bind outside loopback and
  the Tailscale ranges. You almost never want this.

`/health`, `/ready`, `/metrics` and `/budget` are served on the same listener
without a token, exactly as on loopback `:9100`. A bad `[api]` section is
logged with its fix and the daemon keeps running without the API — a dashboard
knob must never stop builds. CLI overrides: `rchd --api-bind`,
`rchd --api-token-file`. Set from the CLI:

```bash
head -c 32 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=' > ~/.config/rch/api.token
chmod 600 ~/.config/rch/api.token
rch config set api.bind tailscale
rch config set api.token_file ~/.config/rch/api.token
# then restart rchd; verify from another tailnet box:
curl -H "Authorization: Bearer $(cat ~/.config/rch/api.token)" http://100.x.y.z:9101/status | jq .daemon
```

### `[dashboard]`
- `url` (string, optional) — where the fleet dashboard is served, e.g.
  `https://rch-fleet.vercel.app`. `rch web` opens it (`--url` and
  `RCH_DASHBOARD_URL` override). Agents use `<url>/api/fleet?view=help`.

## Workers Config (`workers.toml`)

Location: `~/.config/rch/workers.toml`

Each worker entry:
- `id` (string, required) — Unique identifier.
- `host` (string, required) — Hostname or IP.
- `user` (string, default `"ubuntu"`) — SSH user.
- `identity_file` (string, default `"~/.ssh/id_rsa"`) — SSH key path.
- `total_slots` (u32, default `8`) — CPU slots available.
- `priority` (u32, default `100`) — Higher = preferred.
- `tags` (list, default `[]`) — Optional selection tags.
- `tools` (list, default `[]`) — Verified named tool probes; see below.
- `enabled` (bool, default `true`) — Skip worker if false.

Example:

```toml
[[workers]]
id = "worker-1"
host = "203.0.113.20"
user = "ubuntu"
identity_file = "~/.ssh/id_rsa"
total_slots = 16
priority = 100
tags = ["ssd", "fast"]
enabled = true
tools = [
  { name = "clang",  command = ["clang", "--version"] },
  { name = "ld.lld", command = ["/usr/bin/ld.lld", "--version"] },
]
```

### `tools` — verified named tool probes

Each entry declares a **fixed argv** that the worker runs as its configured
user. The probe is judged by **exit status alone** — zero means present,
anything else (missing binary, error exit, signal) means absent — and its
output is discarded so a chatty tool cannot corrupt the fact stream.

- `name` (string, required) — ASCII letters, digits, `-`, `_`, `.` or `+`.
  Anything else (a space, an `=`, a control character) is **refused when the
  config loads**: such a name could not survive the probe's `RCH_FACT
  tool=<name>` format and would be read back as a different name, which is a
  silent capability lie rather than a loud config error.
- `command` (list of strings, required, non-empty) — the exact argv. It is
  never caller-supplied; RCH exposes no general remote-shell probe API,
  because a caller that could ask a worker to run an arbitrary command "to
  check for a tool" would be a remote execution primitive wearing a
  capability-probe hat.

Results appear in `rch workers capabilities --refresh` under **Named tools**,
split into `verified:` and `failed:`. The split matters: a worker that
declared a tool whose probe fails is broken, while a name nothing declared is
usually a typo or a fleet that was never configured for it.

Declaring no `tools` sends the worker the byte-identical capability command it
has always received, so adding probes to one worker cannot disturb the rest of
a fleet — including workers still running an older `rch-wkr`.

Jobs require them with `rch exec --job --require-tool NAME`, or per project:

```toml
# .rch/config.toml or ~/.config/rch/config.toml
[jobs]
required_tools = ["clang"]
```

Project defaults add to `--require-tool` in **job mode only**. Explicit
`--require-tool NAME` also applies to normal compilations without bypassing
their classifier or Rust target/toolchain requirements. Required tools demand
strict remote execution; forced-local or disabled configuration refuses them.
Both client and daemon need the verified-tools endpoint; older daemons reject
it before worker reservation instead of ignoring the requirement.

A required tool no worker has verified admits no
worker at all; selection reports
`capability_missing:tool:<name>:probe_failed` or `...:not_declared`.
These cached presence probes do not identify the actual compiler or SDK bytes
used in a particular build. `rch admit --require-tool NAME -- <command>` is a
static requirement preview, not live worker or release qualification.

## Daemon Config (`daemon.toml`)

Location: `~/.config/rch/daemon.toml`

Fields:
- `socket_path` (default `$XDG_RUNTIME_DIR/rch.sock` or `~/.cache/rch/rch.sock`)
  — Unix socket path.
- `health_check_interval_secs` (default `30`) — Health check cadence.
- `worker_timeout_secs` (default `10`) — Mark worker unreachable after timeout.
- `max_jobs_per_slot` (default `1`) — Max concurrent jobs per slot.
- `connection_pooling` (default `true`) — Reuse SSH connections.
- `log_level` (default `"info"`) — Daemon logging level.

## Environment Variables (RCH_*)

RCH uses environment variables for overrides, tooling, and testing. The list
below groups variables by intent. If you are unsure whether a variable is
applied in your version, verify with `rch config show --sources` or `rch --help`.

### Core runtime overrides (hook/CLI)
These are read by the hook configuration loader:

- `RCH_ENABLED`
- `RCH_LOG_LEVEL`
- `RCH_SOCKET_PATH`
- `RCH_CONFIDENCE_THRESHOLD`
- `RCH_MIN_LOCAL_TIME_MS`
- `RCH_REMOTE_SPEEDUP_THRESHOLD`
- `RCH_COMPRESSION_LEVEL`
- `RCH_COMPRESSION` (legacy alias for `RCH_COMPRESSION_LEVEL`)
- `RCH_SYNC_TIMEOUT_MS` (per-attempt source-upload timeout; `1000..=3600000`)
- `RCH_CANONICAL_PROJECT_ROOT`
- `RCH_ALIAS_PROJECT_ROOT`

`rch config export` emits the loader-consumed names above so exported shell or
`.env` output can be sourced directly. Use `rch config show --sources` to verify
which layer supplied each value.

### CLI/daemon options (documented in `rch --help` / validators)

- `RCH_PROFILE`
- `RCH_LOG_FORMAT`
- `RCH_DAEMON_SOCKET` (legacy alias for `RCH_SOCKET_PATH`; the canonical name wins when both are set)
- `RCH_DAEMON_TIMEOUT_MS` (daemon socket connect/read timeout in milliseconds, `100..=600000`, default `5000`; the select-worker wait keeps `RCH_DAEMON_RESPONSE_TIMEOUT_SECS` / `RCH_DAEMON_WAIT_RESPONSE_TIMEOUT_SECS`)
- `RCH_TRANSFER_ZSTD_LEVEL` (legacy alias for `RCH_COMPRESSION_LEVEL`, lowest precedence)
- `RCH_ENABLE_METRICS`
- `RCH_TEST_MODE`
- `RCH_WORKER` (preferred worker override)
- `RCH_WORKERS` (comma-separated preferred worker override list)

SSH identities are per worker (`identity_file` in `workers.toml`); there is no
global `RCH_SSH_KEY` override.

### Hook integration variables
Used by hook integration scripts (see `docs/extending/integration-hooks.md`):

- `RCH_COMMAND`
- `RCH_PROJECT`
- `RCH_ESTIMATED_CORES`
- `RCH_WORKER`
- `RCH_LAST_BUILD_WORKER`
- `RCH_BUILD_SUCCESS`
- `RCH_BUILD_ERROR`
- `RCH_AVAILABLE_WORKERS`
- `RCH_TEST_HOOK`

### Fleet dashboard

- `RCH_DASHBOARD_URL` — the URL `rch web` opens (overrides `[dashboard] url`).

### Installer / setup helpers

- `RCH_CONFIG_DIR`
- `RCH_INSTALL_DIR`
- `RCH_NO_COLOR`
- `RCH_NO_HOOK`
- `RCH_SKIP_DOCTOR`
- `RCH_INIT_HOST`
- `RCH_VERBOSE`

### Mocking, test, and CI helpers

- `RCH_MOCK_SSH`
- `RCH_MOCK_CIRCUIT_OPEN`
- `RCH_MOCK_NO_RUSTUP`
- `RCH_MOCK_RSYNC_BYTES`
- `RCH_MOCK_RSYNC_FAIL_ARTIFACTS`
- `RCH_MOCK_RSYNC_FAIL_SYNC`
- `RCH_MOCK_RSYNC_FILES`
- `RCH_MOCK_SSH_DELAY_MS`
- `RCH_MOCK_SSH_EXIT_CODE`
- `RCH_MOCK_SSH_FAIL_CONNECT`
- `RCH_MOCK_SSH_FAIL_EXECUTE`
- `RCH_MOCK_SSH_STDERR`
- `RCH_MOCK_SSH_STDOUT`
- `RCH_MOCK_TOOLCHAIN_INSTALL_FAIL`
- `RCH_E2E_VERBOSE`
- `RCH_E2E_WORKERS_FILE`
- `RCH_E2E_WORKER_HOST`
- `RCH_E2E_WORKER_ID`
- `RCH_E2E_WORKER_KEY`
- `RCH_E2E_WORKER_SLOTS`
- `RCH_E2E_WORKER_USER`
- `RCH_CIRCUIT_FAILURE_THRESHOLD`
- `RCH_CIRCUIT_RESET_TIMEOUT_SEC`
- `RCH_OTEL_ENABLED`
- `RCH_PRESET`
- `RCH_BAD_BOOL`
- `RCH_TEST_`
- `RCH_TEST_1`
- `RCH_TEST_123`
- `RCH_TEST_12345`
- `RCH_TEST_99999`
- `RCH_TEST_BOOL_FALSE`
- `RCH_TEST_BOOL_TRUE`
- `RCH_TEST_LIST`
- `RCH_TEST_MARKER_UNIQUE_12345_XYZ`
- `RCH_TEST_OK`
- `RCH_TEST_OPT`
- `RCH_TEST_SRC`
- `RCH_TEST_U64`
- `RCH_TEST_U64_OOR`
- `RCH_TEST_WORKER_HOST`
- `RCH_TEST_WORKER_KEY`
- `RCH_TEST_WORKER_USER`

### Documented but not yet wired in code

These appear in docs/README but are not currently used by the config loader:

- `RCH_BYPASS`
- `RCH_DRY_RUN`
- `RCH_LOCAL_ONLY`
- `RCH_STREAM_MODE`
- `RCH_NO_CACHE`

## Per-Project Configuration

Use `.rch/config.toml` when you need project-specific behavior (e.g. disable
RCH for a repo or change thresholds). Only the sections above are recognized;
unknown keys are ignored.

## Debugging Configuration

- `rch config show` — Show effective config
- `rch config show --sources` — Show value origins
- `rch config export` — Export config to shell/.env format
- `rch doctor` — Diagnose common misconfigurations

## Related Docs

- `docs/QUICKSTART.md`
- `docs/TROUBLESHOOTING.md`
- `docs/runbooks/configuration-troubleshooting.md`
