#!/usr/bin/env bash
#
# e2e_bd-x1ek.sh - Retry/backoff for rsync + SSH exec
#
# Executes the hook's delegated command with mock transport:
# 1) Baseline sync succeeds in one attempt
# 2) Two transient sync failures are followed by one successful attempt
# 3) force_local bypasses remote admission
# Native unit cases also cover retry bounds and transport error classification.
#
# Output is logged in JSONL format.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# Use CARGO_TARGET_DIR if set, otherwise default to $PROJECT_ROOT/target
RCH_TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
LOG_FILE="${RCH_E2E_LOG:-$RCH_TARGET_DIR/e2e_bd-x1ek.jsonl}"

# shellcheck source=scripts/lib/e2e_common.sh
source "$SCRIPT_DIR/lib/e2e_common.sh"
LAST_BUILD_ID=0

timestamp() {
    date -u '+%Y-%m-%dT%H:%M:%S.%3NZ' 2>/dev/null || date -u '+%Y-%m-%dT%H:%M:%SZ'
}

log_json() {
    local phase="$1"
    local message="$2"
    local extra="${3:-}"
    local ts
    ts="$(timestamp)"
    if [[ -z "$extra" || "$extra" == "{}" ]]; then
        printf '{"ts":"%s","test":"bd-x1ek","phase":"%s","message":"%s"}\n' \
            "$ts" "$phase" "$message" | tee -a "$LOG_FILE"
        return
    fi

    local payload="$extra"
    payload="${payload#\{}"
    payload="${payload%\}}"
    if [[ -z "$payload" ]]; then
        printf '{"ts":"%s","test":"bd-x1ek","phase":"%s","message":"%s"}\n' \
            "$ts" "$phase" "$message" | tee -a "$LOG_FILE"
        return
    fi

    printf '{"ts":"%s","test":"bd-x1ek","phase":"%s","message":"%s",%s}\n' \
        "$ts" "$phase" "$message" "$payload" | tee -a "$LOG_FILE"
}

die() {
    log_json "error" "$*" '{"result":"fail"}'
    exit 1
}

check_dependencies() {
    log_json "setup" "Checking dependencies"
    for cmd in cargo jq awk; do
        command -v "$cmd" >/dev/null 2>&1 || die "Missing dependency: $cmd"
    done
}

build_binaries() {
    log_json "build" "Building rch + rchd (debug)"
    (cd "$PROJECT_ROOT" && cargo build -p rch -p rchd >/dev/null 2>&1) || die "cargo build failed"
    [[ -x "$RCH_TARGET_DIR/debug/rch" ]] || die "rch binary missing after build"
    [[ -x "$RCH_TARGET_DIR/debug/rchd" ]] || die "rchd binary missing after build"
}

make_test_project() {
    TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/rch-bd-x1ek-XXXXXX")"
    TEST_ROOT="$(cd "$TEST_ROOT" && pwd -P)"
    PROJECT_DIR="$TEST_ROOT/project"
    LOG_DIR="$TEST_ROOT/logs"
    mkdir -p "$PROJECT_DIR/src" "$LOG_DIR" "$TEST_ROOT/config"

    cat >"$TEST_ROOT/config/config.toml" <<EOF
[path_topology]
canonical_root = "$TEST_ROOT"
alias_root = "${TEST_ROOT}__alias"

[output]
visibility = "verbose"
first_run_complete = true
EOF

    cat >"$PROJECT_DIR/Cargo.toml" <<'EOF'
[package]
name = "rch_e2e_bd_x1ek"
version = "0.1.0"
edition = "2024"

[dependencies]
EOF

    cat >"$PROJECT_DIR/src/main.rs" <<'EOF'
fn main() {
    println!("rch bd-x1ek e2e ok");
}
EOF

    log_json "setup" "Created test project" "{\"root\":\"$TEST_ROOT\"}"
}

write_workers_config() {
    WORKERS_FILE="$TEST_ROOT/workers.toml"
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
    cp "$WORKERS_FILE" "$TEST_ROOT/config/workers.toml"
}

start_daemon() {
    SOCKET_PATH="$TEST_ROOT/rch.sock"
    DAEMON_LOG="$LOG_DIR/rchd.log"

    log_json "daemon" "Starting rchd (mock transport)" "{\"socket\":\"$SOCKET_PATH\"}"
    env RCH_DAEMON_INSTALLS_HOOKS=0 RCH_MOCK_SSH=1 RCH_MOCK_SSH_STDOUT=health_check \
        "RCH_CONFIG_DIR=$TEST_ROOT/config" "RCH_STATE_HOME=$LOG_DIR/state/rch" "RCH_SOCKET_PATH=$SOCKET_PATH" \
        "$RCH_TARGET_DIR/debug/rchd" \
        --socket "$SOCKET_PATH" \
        --workers-config "$WORKERS_FILE" \
        --foreground \
        >>"$DAEMON_LOG" 2>&1 &
    RCHD_PID=$!

    local waited=0
    while [[ ! -S "$SOCKET_PATH" && $waited -lt 50 ]]; do
        sleep 0.1
        waited=$((waited + 1))
    done
    [[ -S "$SOCKET_PATH" ]] || die "Daemon socket not found after startup (log: $DAEMON_LOG)"
    log_json "daemon" "Daemon ready" "{\"pid\":$RCHD_PID}"
}

stop_daemon() {
    if [[ -n "${RCHD_PID:-}" ]]; then
        log_json "daemon" "Stopping daemon" "{\"pid\":$RCHD_PID}"
        kill "$RCHD_PID" >/dev/null 2>&1 || true
        wait "$RCHD_PID" >/dev/null 2>&1 || true
        RCHD_PID=""
    fi
}

hook_json() {
    cat <<'JSON'
{
  "tool_name": "Bash",
  "tool_input": {
    "command": "cargo build",
    "description": "bd-x1ek e2e build"
  }
}
JSON
}

write_project_config() {
    local config_body="$1"
    mkdir -p "$PROJECT_DIR/.rch"
    cat >"$PROJECT_DIR/.rch/config.toml" <<EOF
$config_body
EOF
}

run_hook() {
    local scenario="$1"
    local extra_env="${2:-}"
    local hook_out="$LOG_DIR/hook_${scenario}.out"
    local hook_err="$LOG_DIR/hook_${scenario}.err"

    (
        cd "$PROJECT_DIR" || exit 1
        # shellcheck disable=SC2086
        printf '%s\n' "$(hook_json)" | env RCH_SOCKET_PATH="$SOCKET_PATH" RCH_MOCK_SSH=1 \
            "RCH_CONFIG_DIR=$TEST_ROOT/config" "RCH_STATE_HOME=$LOG_DIR/state/rch" \
            $extra_env "$RCH_TARGET_DIR/debug/rch" \
            >"$hook_out" 2>"$hook_err"
    )

    # Check for RCH interception:
    # - "updatedInput" is an executable rewrite, not proof of remote execution
    # - "permissionDecision":"deny" means blocked (legacy, still used for actual denials)
    if command grep -q '"updatedInput"' "$hook_out"; then
        echo "intercepted"
    elif command grep -q '"permissionDecision":"deny"' "$hook_out"; then
        echo "deny"  # Blocked
    else
        echo "allow"  # Pass-through to local
    fi
}

expect_decision() {
    local scenario="$1"
    local expected="$2"
    local extra_env="${3:-}"
    local got
    got="$(run_hook "$scenario" "$extra_env")"
    log_json "verify" "Hook decision" "{\"scenario\":\"$scenario\",\"expected\":\"$expected\",\"got\":\"$got\"}"
    [[ "$got" == "$expected" ]] || die "Scenario $scenario expected $expected, got $got"
}

expect_remote_sync() {
    local scenario="$1" expected_failures="$2"
    local prefix="$LOG_DIR/$scenario" attempts failures successes
    e2e_run_delegated "$RCH_TARGET_DIR/debug/rch" "$PROJECT_DIR" "cargo build" "$prefix" \
        "RCH_CONFIG_DIR=$TEST_ROOT/config" "RCH_SOCKET_PATH=$SOCKET_PATH" \
        "CARGO_HOME=$TEST_ROOT/cargo-home" RCH_REQUIRE_REMOTE=1 RCH_MOCK_SSH=1 \
        RCH_MOCK_SSH_EXIT_CODE=0 "RCH_MOCK_RSYNC_FAIL_SYNC_ATTEMPTS=$expected_failures" \
        RUST_LOG=info || die "$scenario did not execute the native hook rewrite"
    [[ "$E2E_EXEC_EXIT" -eq 0 ]] && jq -se \
        'length == 1 and (.[0] | .location == "remote" and .outcome == "completed"
            and .worker_id == "mock-worker" and .command == "cargo build" and .remote_exit_code == 0)' \
        "$prefix.exec.json" >/dev/null || die "$scenario did not complete remotely"

    # Count actual mock transport calls, not the hook's admission decision.
    attempts="$(awk '/Mock sync: / {n++} END {print n+0}' "$prefix.exec.err")"
    failures="$(awk '/Mock transient sync failure/ {n++} END {print n+0}' "$prefix.exec.err")"
    successes="$(awk '/Mock sync complete:/ {n++} END {print n+0}' "$prefix.exec.err")"
    log_json "verify" "Observed source sync attempts" \
        "{\"scenario\":\"$scenario\",\"attempts\":$attempts,\"transient_failures\":$failures,\"successes\":$successes}"
    [[ "$attempts" -eq $((expected_failures + 1)) && "$failures" -eq "$expected_failures" && "$successes" -eq 1 ]] \
        || die "$scenario did not preserve the exact retry attempt history"

    env "RCH_CONFIG_DIR=$TEST_ROOT/config" "RCH_SOCKET_PATH=$SOCKET_PATH" \
        "RCH_STATE_HOME=$LOG_DIR/state/rch" "$RCH_TARGET_DIR/debug/rch" status --json \
        >"$prefix.status.json" 2>"$prefix.status.err" || die "$scenario daemon status failed"
    jq -e --argjson previous "$LAST_BUILD_ID" \
        '.success == true and (.data.daemon.active_builds | length) == 0 and
         (.data.daemon.recent_builds[0] | .id > $previous and .exit_code == 0 and
          .worker_id == "mock-worker" and .command == "cargo build")' \
        "$prefix.status.json" >/dev/null || die "$scenario has no new durable completion"
    LAST_BUILD_ID="$(jq -er '.data.daemon.recent_builds[0].id' "$prefix.status.json")"
}

# =============================================================================
# Unit-style tests for RetryConfig (run via cargo test, not here)
# =============================================================================
# The retry logic is also tested via unit tests in rch-common/src/types.rs:
# - test_retry_config_defaults
# - test_retry_config_delay_calculation
# - test_retry_config_should_retry
# - test_retry_config_no_retry
# - etc.

# =============================================================================
# E2E Scenarios
# =============================================================================

test_baseline_remote_works() {
    log_json "test" "Baseline: execute remotely with one source sync attempt"
    write_project_config $'[general]\nenabled = true\n'
    expect_remote_sync "baseline" 0
}

test_transient_failures_succeed() {
    log_json "test" "Two transient source sync failures must be followed by remote success"
    write_project_config $'[general]\nenabled = true\n'
    # Mock rsync will fail first 2 attempts, then succeed
    # The mock has built-in transient failure support that decrements on each call
    expect_remote_sync "transient_2_attempts" 2
}

test_force_local_bypasses_retry() {
    log_json "test" "force_local: bypasses remote entirely, no retry needed"
    write_project_config $'[general]\nenabled = true\nforce_local = true\n'
    expect_decision "force_local" "allow"
}

test_is_retryable_transport_error() {
    log_json "test" "Verifying is_retryable_transport_error classifications"

    # Run Rust unit tests for transport error classification
    (cd "$PROJECT_ROOT" && e2e_cargo_test -p rch-common retryable_transport_error_text -- --nocapture) \
        || die "is_retryable_transport_error unit tests failed"

    log_json "verify" "Transport error classification tests passed"
}

test_retry_config_unit_tests() {
    log_json "test" "Running RetryConfig unit tests"

    # Run Rust unit tests for RetryConfig
    (cd "$PROJECT_ROOT" && e2e_cargo_test -p rch-common retry_config -- --nocapture) \
        || die "RetryConfig unit tests failed"

    log_json "verify" "RetryConfig unit tests passed"
}

main() {
    : > "$LOG_FILE"
    check_dependencies
    build_binaries
    make_test_project
    write_workers_config

    trap stop_daemon EXIT
    start_daemon

    # Run E2E scenarios
    test_baseline_remote_works
    test_transient_failures_succeed
    test_force_local_bypasses_retry

    # Run unit tests that exercise retry logic
    test_is_retryable_transport_error
    test_retry_config_unit_tests

    log_json "summary" "All bd-x1ek checks passed" '{"result":"pass"}'
}

main "$@"
