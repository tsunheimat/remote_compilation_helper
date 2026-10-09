#!/usr/bin/env bash
#
# e2e_pipeline.sh - Legacy pipeline test for Remote Compilation Helper (RCH)
#
# Usage:
#   ./scripts/e2e_pipeline.sh [OPTIONS]
#
# Options:
#   --mock                 Run with mock SSH/rsync (default)
#   --real                 Run with real workers (requires env below)
#   --fail MODE            Inject failure: sync|exec|artifacts|worker-down|remote-exit|toolchain-install|no-rustup|circuit-open
#   --run-all              In mock mode, run success + failure scenarios
#   --unit                 Also run `cargo test --workspace`
#   --verbose              Enable verbose output
#   --help                 Show this help message
#
# Environment (real mode):
#   RCH_E2E_WORKERS_FILE    Path to workers.toml (preferred)
#   RCH_E2E_WORKER_HOST     Worker host
#   RCH_E2E_WORKER_USER     SSH user (default: ubuntu)
#   RCH_E2E_WORKER_KEY      SSH key path (default: ~/.ssh/id_rsa)
#   RCH_E2E_WORKER_ID       Worker id (default: e2e-worker)
#   RCH_E2E_WORKER_SLOTS    Total slots (default: 8)
#
# Notes:
# - Mock mode uses RCH_MOCK_SSH=1 and does NOT create real artifacts.
# - The hook rewrite is executed through rch exec; mock runs assert its result,
#   transfer diagnostics, and daemon ownership without compiling locally.
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
export PROJECT_ROOT
# Use CARGO_TARGET_DIR if set, otherwise default to $PROJECT_ROOT/target
RCH_TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
export RCH_TARGET_DIR
MODE="mock"
FAIL_MODE=""
RUN_ALL="0"
RUN_UNIT="0"
VERBOSE="${RCH_E2E_VERBOSE:-0}"

# Structured JSONL logging
# shellcheck disable=SC1091
source "$SCRIPT_DIR/test_lib.sh"
init_test_log "$(basename "${BASH_SOURCE[0]}" .sh)"
# shellcheck source=lib/e2e_common.sh
source "$SCRIPT_DIR/lib/e2e_common.sh"

timestamp() { date -u '+%Y-%m-%dT%H:%M:%S.%3NZ'; }

fail_with_code() {
    local exit_code="$1"
    shift
    local reason="$*"
    log_json verify "TEST FAIL" "{\"reason\":\"$reason\"}"
    exit "$exit_code"
}

log() {
    local level="$1" phase="$2"; shift 2
    local ts; ts="$(timestamp)"
    local msg="[$ts] [$level] [$phase] $*"
    echo "$msg"

    local json_phase="execute"
    case "$phase" in
        SETUP|ARGS|PREFLIGHT) json_phase="setup" ;;
        VERIFY) json_phase="verify" ;;
        CLEANUP|TEARDOWN) json_phase="teardown" ;;
        *) json_phase="execute" ;;
    esac
    log_json "$json_phase" "$msg"
}

die() { log "FAIL" "SETUP" "$*"; fail_with_code 2 "$*"; }

usage() {
    sed -n '1,40p' "$0" | sed 's/^# \{0,1\}//'
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --mock) MODE="mock"; shift ;;
            --real) MODE="real"; shift ;;
            --fail) FAIL_MODE="${2:-}"; shift 2 ;;
            --run-all) RUN_ALL="1"; shift ;;
            --unit) RUN_UNIT="1"; shift ;;
            --verbose|-v) VERBOSE="1"; shift ;;
            --help|-h) usage; exit 0 ;;
            *) log "FAIL" "ARGS" "Unknown option: $1"; fail_with_code 3 "Unknown option: $1" ;;
        esac
    done
    [[ "${RCH_MOCK_SSH:-}" == "1" ]] && MODE="mock" || true
    if [[ "$MODE" == "mock" && "$RUN_ALL" == "0" ]]; then
        RUN_ALL="1"
    fi
}

check_dependencies() {
    log "INFO" "SETUP" "Checking dependencies..."
    for cmd in cargo rustc jq; do
        command -v "$cmd" >/dev/null 2>&1 || die "Missing: $cmd"
    done
    log "INFO" "SETUP" "Dependencies OK"
}

build_binaries() {
    if [[ -x "$RCH_TARGET_DIR/debug/rch" && -x "$RCH_TARGET_DIR/debug/rchd" ]]; then
        log "INFO" "BUILD" "Using existing rch + rchd: $RCH_TARGET_DIR/debug"
        return 0
    fi
    log "INFO" "BUILD" "Building rch + rchd (debug)..."
    cd "$PROJECT_ROOT"
    cargo build -p rch -p rchd >/dev/null 2>&1 || die "Build failed"
    [[ -x "$RCH_TARGET_DIR/debug/rch" ]] || die "Binary missing: rch"
    [[ -x "$RCH_TARGET_DIR/debug/rchd" ]] || die "Binary missing: rchd"
    log "INFO" "BUILD" "Build OK"
}

make_test_project() {
    TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/rch-e2e-XXXXXX")"
    TEST_ROOT="$(cd "$TEST_ROOT" && pwd -P)"
    PROJECT_DIR="$TEST_ROOT/project"
    LOG_DIR="$TEST_ROOT/logs"
    RUNTIME_DIR="$(e2e_runtime_dir)"
    mkdir -p "$PROJECT_DIR/src" "$LOG_DIR"
    mkdir -p "$TEST_ROOT/config" "$TEST_ROOT/bin" "$TEST_ROOT/cache" "$TEST_ROOT/state"
    cat >"$TEST_ROOT/config/config.toml" <<EOF
[path_topology]
canonical_root = "$TEST_ROOT"
alias_root = "${TEST_ROOT}__alias"

[output]
visibility = "verbose"
first_run_complete = true
EOF
    export RCH_CONFIG_DIR="$TEST_ROOT/config"
    export XDG_CACHE_HOME="$TEST_ROOT/cache"
    export XDG_STATE_HOME="$TEST_ROOT/state"
    export XDG_DATA_HOME="$TEST_ROOT/data"

    # Local fallback must be observable without accidentally starting a real
    # compilation. Metadata still comes from Cargo for the actual fixture tree.
    export RCH_E2E_REAL_CARGO
    RCH_E2E_REAL_CARGO="$(command -v cargo)"
    export RCH_E2E_METADATA_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-}"
    cat >"$TEST_ROOT/bin/cargo" <<'EOF'
#!/usr/bin/env bash
if [[ "${1:-}" == +* ]]; then shift; fi
case "${1:-}" in
    metadata|locate-project|--version|-V)
        if [[ -n "${RCH_E2E_METADATA_TOOLCHAIN:-}" ]]; then
            exec env RUSTUP_TOOLCHAIN="$RCH_E2E_METADATA_TOOLCHAIN" "$RCH_E2E_REAL_CARGO" "$@"
        fi
        exec env -u RUSTUP_TOOLCHAIN "$RCH_E2E_REAL_CARGO" "$@"
        ;;
esac
printf 'RCH E2E local Cargo fixture: %s\n' "$*" >&2
printf '%s\n' "$*" >>"$RCH_E2E_LOCAL_MARKER"
exit 79
EOF
    chmod +x "$TEST_ROOT/bin/cargo"

    cat >"$PROJECT_DIR/Cargo.toml" <<'EOF'
[package]
name = "rch_e2e_app"
version = "0.1.0"
edition = "2024"

[dependencies]
EOF

    cat >"$PROJECT_DIR/src/main.rs" <<'EOF'
fn main() {
    println!("rch e2e ok");
}
EOF
    CARGO_HOME="$TEST_ROOT/cargo-home" "$RCH_E2E_REAL_CARGO" metadata --manifest-path "$PROJECT_DIR/Cargo.toml" \
        --format-version 1 --no-deps >"$LOG_DIR/fixture-metadata.json" \
        || die "Fixture Cargo metadata preparation failed"

    log "INFO" "SETUP" "Test project: $PROJECT_DIR"
    log "INFO" "SETUP" "Logs: $LOG_DIR"
}

write_workers_config() {
    WORKERS_FILE="$TEST_ROOT/workers.toml"

    if [[ "$MODE" == "mock" ]]; then
        cat >"$WORKERS_FILE" <<'EOF'
[[workers]]
id = "mock-worker"
host = "mock.host"
user = "mockuser"
identity_file = "~/.ssh/mock"
total_slots = 64
priority = 100
enabled = true
EOF
        return
    fi

    if [[ -n "${RCH_E2E_WORKERS_FILE:-}" ]]; then
        if [[ ! -f "$RCH_E2E_WORKERS_FILE" ]]; then
            die "RCH_E2E_WORKERS_FILE not found: $RCH_E2E_WORKERS_FILE"
        fi
        WORKERS_FILE="$RCH_E2E_WORKERS_FILE"
        return
    fi

    local host="${RCH_E2E_WORKER_HOST:-}"
    local user="${RCH_E2E_WORKER_USER:-ubuntu}"
    local key="${RCH_E2E_WORKER_KEY:-~/.ssh/id_rsa}"
    local wid="${RCH_E2E_WORKER_ID:-e2e-worker}"
    local slots="${RCH_E2E_WORKER_SLOTS:-8}"

    [[ -n "$host" ]] || die "RCH_E2E_WORKER_HOST is required for --real"

    cat >"$WORKERS_FILE" <<EOF
[[workers]]
id = "$wid"
host = "$host"
user = "$user"
identity_file = "$key"
total_slots = $slots
priority = 100
enabled = true
EOF
}

start_daemon() {
    SOCKET_PATH="$RUNTIME_DIR/${1:-default}.sock"
    DAEMON_LOG="$LOG_DIR/rchd_${1:-default}.log"

    log "INFO" "DAEMON" "Starting rchd (socket: $SOCKET_PATH)"
    if [[ "$MODE" == "mock" ]]; then
        env RCH_DAEMON_INSTALLS_HOOKS=0 RCH_MOCK_SSH=1 RCH_MOCK_SSH_STDOUT=health_check \
            "$RCH_TARGET_DIR/debug/rchd" \
            --socket "$SOCKET_PATH" \
            --workers-config "$WORKERS_FILE" \
            --foreground \
            >>"$DAEMON_LOG" 2>&1 &
    else
        RCH_DAEMON_INSTALLS_HOOKS=0 "$RCH_TARGET_DIR/debug/rchd" \
            --socket "$SOCKET_PATH" \
            --workers-config "$WORKERS_FILE" \
            --foreground \
            >>"$DAEMON_LOG" 2>&1 &
    fi
    RCHD_PID=$!

    local waited=0
    while [[ ! -S "$SOCKET_PATH" && $waited -lt 50 ]]; do
        sleep 0.1
        waited=$((waited + 1))
    done

    if [[ ! -S "$SOCKET_PATH" ]]; then
        die "Daemon socket not found after startup (log: $DAEMON_LOG)"
    fi
    log "INFO" "DAEMON" "Daemon ready (pid: $RCHD_PID)"
}

stop_daemon() {
    if [[ -n "${RCHD_PID:-}" ]]; then
        log "INFO" "DAEMON" "Stopping rchd (pid: $RCHD_PID)"
        kill "$RCHD_PID" >/dev/null 2>&1 || true
        wait "$RCHD_PID" >/dev/null 2>&1 || true
        RCHD_PID=""
    fi
}

run_pipeline_command() {
    local scenario="$1" command="$2" expect="$3" fail="$4"
    shift 4
    stop_daemon
    start_daemon "$scenario"
    local prefix="$LOG_DIR/$scenario" marker="$LOG_DIR/$scenario.local-cargo"
    local require_remote=1 expected_exit=0 expected_location="remote" expected_outcome="completed"
    if [[ "$expect" == "allow" ]]; then
        require_remote=0
        expected_exit=79
        expected_location="local"
    fi
    case "$fail" in
        artifacts) expected_exit=102 ;;
        remote-exit) expected_exit=2 ;;
        exec|worker-down)
            # Once execution may have started, uncertainty retains ownership
            # and refuses replay, including local replay.
            expected_exit=1
            expected_location="remote"
            expected_outcome="transport_error"
            ;;
    esac

    log "INFO" "HOOK" "Executing hook rewrite ($scenario)"
    e2e_run_delegated "$RCH_TARGET_DIR/debug/rch" "$PROJECT_DIR" "$command" "$prefix" \
        "PATH=$TEST_ROOT/bin:$RCH_TARGET_DIR/debug:$PATH" \
        "CARGO_HOME=$TEST_ROOT/cargo-home" \
        "RCH_CONFIG_DIR=$TEST_ROOT/config" "RCH_E2E_REAL_CARGO=$RCH_E2E_REAL_CARGO" \
        "RCH_E2E_METADATA_TOOLCHAIN=$RCH_E2E_METADATA_TOOLCHAIN" \
        "RCH_E2E_LOCAL_MARKER=$marker" "RCH_SOCKET_PATH=$SOCKET_PATH" \
        "RCH_REQUIRE_REMOTE=$require_remote" "$@" \
        || die "$scenario did not produce an executable hook rewrite; see $prefix.hook.json"

    if [[ "$E2E_EXEC_EXIT" -ne "$expected_exit" ]] || ! jq -e \
        --arg location "$expected_location" --arg outcome "$expected_outcome" \
        --argjson code "$expected_exit" \
        '.location == $location and .outcome == $outcome and
         (if $outcome == "completed" then .remote_exit_code == $code else .remote_exit_code == null end)' \
        "$prefix.exec.json" >/dev/null; then
        log "FAIL" "SCENARIO" "$scenario expected $expected_location/$expected_outcome exit $expected_exit, got exit $E2E_EXEC_EXIT (see $prefix.exec.json and .err)"
        return 1
    fi
    if [[ "$expected_location" == "local" ]]; then
        [[ -s "$marker" ]] || die "$scenario did not execute the observable local fallback fixture"
    elif [[ -e "$marker" ]]; then
        die "$scenario unexpectedly replayed locally"
    fi

    RCH_SOCKET_PATH="$SOCKET_PATH" "$RCH_TARGET_DIR/debug/rch" status --json \
        >"$prefix.status.json" 2>"$prefix.status.err" || die "$scenario status query failed"
    local active_count=0
    [[ "$fail" != "exec" && "$fail" != "worker-down" ]] || active_count=1
    jq -e --argjson count "$active_count" \
        '.success == true and (.data.daemon.active_builds | length) == $count' \
        "$prefix.status.json" >/dev/null || die "$scenario daemon ownership does not match the execution outcome"

    if [[ "$expected_location" == "remote" && "$expected_outcome" == "completed" ]]; then
        jq -e --argjson code "$expected_exit" \
            '.data.daemon.recent_builds[0].exit_code == $code' "$prefix.status.json" >/dev/null \
            || die "$scenario durable completion did not record exit $expected_exit"
    fi
    if [[ "$MODE" == "mock" && -z "$fail" ]]; then
        check_artifacts_mock "$prefix.exec.err" || die "$scenario artifact retrieval phase missing"
    elif [[ "$MODE" == "mock" && "$fail" == "artifacts" ]]; then
        check_artifacts_mock_failure "$prefix.exec.err" || die "$scenario artifact retrieval failure missing"
        grep -q 'RCH-E309' "$prefix.exec.err" || die "$scenario artifact failure lost its error code"
    elif [[ "$MODE" == "real" && "$expected_exit" == 0 ]]; then
        check_artifacts_real || die "$scenario did not retrieve the real binary"
    fi
    log "INFO" "SCENARIO" "$scenario OK ($expected_location/$expected_outcome exit $expected_exit)"
}

check_artifacts_real() {
    local bin_path="$PROJECT_DIR/target/debug/rch_e2e_app"
    [[ -x "$bin_path" ]]
}

check_artifacts_mock() {
    local hook_err="$1"
    /bin/grep -Eq 'Mock artifact retrieval complete: [1-9][0-9]* files, [1-9][0-9]* bytes' "$hook_err" \
        && jq -e '.timing.sync_down != null and .timing.sync_down > 0' \
            "${hook_err%.exec.err}.exec.json" >/dev/null
}

check_artifacts_mock_failure() {
    local hook_err="$1"
    /bin/grep -Fq 'Mock artifact retrieval failed' "$hook_err"
}

run_scenario() {
    local scenario="$1"
    local expect="$2"
    local fail="$3"
    local envs=()

    if [[ "$MODE" == "mock" ]]; then
        envs+=("RCH_MOCK_SSH=1")
    fi

    case "$fail" in
        sync) envs+=("RCH_MOCK_RSYNC_FAIL_SYNC=1") ;;
        exec) envs+=("RCH_MOCK_SSH_FAIL_EXECUTE=1") ;;
        artifacts) envs+=("RCH_MOCK_RSYNC_FAIL_ARTIFACTS=1") ;;
        worker-down) envs+=("RCH_MOCK_SSH_FAIL_CONNECT=1") ;;
        remote-exit) envs+=("RCH_MOCK_SSH_EXIT_CODE=2") ;;
        toolchain-install) envs+=("RCH_MOCK_TOOLCHAIN_INSTALL_FAIL=1") ;;
        no-rustup) envs+=("RCH_MOCK_NO_RUSTUP=1") ;;
        circuit-open) envs+=("RCH_MOCK_CIRCUIT_OPEN=1") ;;
        "") ;;
        *) die "Unknown failure mode: $fail" ;;
    esac

    run_pipeline_command "$scenario" "cargo build" "$expect" "$fail" "${envs[@]}"
}

# Run a scenario with explicit toolchain specification
run_toolchain_scenario() {
    local scenario="$1"
    local toolchain="$2"
    local expect="$3"
    local fail="$4"
    local envs=()

    if [[ "$MODE" == "mock" ]]; then
        envs+=("RCH_MOCK_SSH=1")
    fi

    case "$fail" in
        toolchain-install) envs+=("RCH_MOCK_TOOLCHAIN_INSTALL_FAIL=1") ;;
        no-rustup) envs+=("RCH_MOCK_NO_RUSTUP=1") ;;
        "") ;;
        *) die "Unknown toolchain failure mode: $fail" ;;
    esac

    run_pipeline_command "$scenario" "cargo +$toolchain build" "$expect" "$fail" "${envs[@]}"
    grep -qiE 'toolchain|rustup' "$LOG_DIR/$scenario.exec.err" \
        || die "$scenario lost its toolchain failure diagnostics"
}

run_e2e() {
    log "INFO" "E2E" "Mode: $MODE"
    log "INFO" "E2E" "Scenario: ${FAIL_MODE:-success}"

    if [[ "$RUN_ALL" == "1" && "$MODE" == "mock" ]]; then
        run_scenario "success" "intercepted" ""
        run_scenario "sync_fail" "allow" "sync"
        run_scenario "exec_fail" "allow" "exec"
        run_scenario "worker_down" "allow" "worker-down"
        run_scenario "artifact_fail" "intercepted" "artifacts"
        run_scenario "remote_exit" "intercepted" "remote-exit"

        # Toolchain synchronization scenarios with explicit toolchain specification
        log "INFO" "E2E" "Running toolchain synchronization scenarios..."

        # Test 1: Nightly toolchain with date - install failure should fall back
        run_toolchain_scenario "tc_nightly_install_fail" "nightly-2024-01-15" "allow" "toolchain-install"

        # Test 2: Stable toolchain - no rustup should fall back
        run_toolchain_scenario "tc_stable_no_rustup" "stable" "allow" "no-rustup"

        # Test 3: Beta with date - install failure should fall back
        run_toolchain_scenario "tc_beta_install_fail" "beta-2024-02-01" "allow" "toolchain-install"

        # Test 4: Specific version - no rustup should fall back
        run_toolchain_scenario "tc_version_no_rustup" "1.75.0" "allow" "no-rustup"

        # Legacy tests without explicit toolchain (backward compatibility)
        run_scenario "toolchain_install_fail" "allow" "toolchain-install"
        run_scenario "no_rustup" "allow" "no-rustup"

        log "INFO" "E2E" "Toolchain scenarios complete"

        # Circuit breaker scenarios
        log "INFO" "E2E" "Running circuit breaker scenarios..."

        # Test: All circuits open triggers local fallback
        # When RCH_MOCK_CIRCUIT_OPEN is set, daemon returns AllCircuitsOpen
        run_scenario "circuit_open" "allow" "circuit-open"

        log "INFO" "E2E" "Circuit breaker scenarios complete"
        return
    fi

    if [[ -n "$FAIL_MODE" ]]; then
        case "$FAIL_MODE" in
            sync|exec|worker-down|toolchain-install|no-rustup|circuit-open) run_scenario "$FAIL_MODE" "allow" "$FAIL_MODE" ;;
            artifacts|remote-exit) run_scenario "$FAIL_MODE" "intercepted" "$FAIL_MODE" ;;
            *) die "Unknown failure mode: $FAIL_MODE" ;;
        esac
    else
        run_scenario "success" "intercepted" ""
    fi
}

run_unit_tests() {
    log "INFO" "UNIT" "Running cargo test --workspace"
    cd "$PROJECT_ROOT"
    cargo test --workspace
}

main() {
    parse_args "$@"
    check_dependencies
    build_binaries
    make_test_project
    write_workers_config
    trap '_test_lib_cleanup; stop_daemon' EXIT

    if [[ "$MODE" == "mock" ]]; then
        export RUST_LOG="${RUST_LOG:-info}"
    fi

    run_e2e

    if [[ "$RUN_UNIT" == "1" ]]; then
        run_unit_tests
    fi

    log "INFO" "DONE" "E2E complete. Logs in $LOG_DIR"
    log "INFO" "DONE" "Temp project kept at $TEST_ROOT"
    test_pass
}

main "$@"
