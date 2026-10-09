#!/usr/bin/env bash
#
# e2e_bd-c7xr.sh - Daemon hot-reload configuration tests
#
# Verifies:
# - workers.toml changes are detected and applied without restart
# - rch daemon reload CLI command triggers reload
# - SIGHUP triggers configuration reload
# - Invalid config is rejected, daemon keeps old config
# - JSONL logging with required fields

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="${RCH_E2E_LOG:-$PROJECT_ROOT/target/e2e_bd-c7xr.jsonl}"

# shellcheck source=lib/e2e_common.sh
source "$SCRIPT_DIR/lib/e2e_common.sh"

passed_tests=0
failed_tests=0
run_start_ms="$(e2e_now_ms)"

daemon_pid=""
tmp_root=""
rch_bin=""
rchd_bin=""
socket_path=""
workers_toml=""

log_json() {
    local phase="$1"
    local message="$2"
    local worker="${3:-local}"
    local command="${4:-}"
    local bytes="${5:-0}"
    local duration="${6:-0}"
    local result="${7:-}"
    local error="${8:-}"
    local ts
    ts="$(e2e_timestamp)"
    printf '{"ts":"%s","test":"bd-c7xr","phase":"%s","worker":"%s","command":"%s","bytes_transferred":%s,"duration_ms":%s,"result":"%s","error":"%s","message":"%s"}\n' \
        "$ts" "$phase" "$worker" "$command" "$bytes" "$duration" "$result" "$error" "$message" | tee -a "$LOG_FILE"
}

record_pass() {
    passed_tests=$((passed_tests + 1))
}

record_fail() {
    failed_tests=$((failed_tests + 1))
}

cleanup() {
    if [[ -n "$daemon_pid" ]]; then
        kill "$daemon_pid" >/dev/null 2>&1 || true
        wait "$daemon_pid" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

check_dependencies() {
    log_json "setup" "Checking dependencies" "local" "dependency check" 0 0 "start"
    for cmd in cargo jq; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            log_json "setup" "Missing dependency" "local" "$cmd" 0 0 "fail" "missing $cmd"
            record_fail
            return 1
        fi
    done
    log_json "setup" "Dependencies ok" "local" "dependency check" 0 0 "pass"
    record_pass
}

build_binaries() {
    local rch="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rch"
    local rchd="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rchd"

    if [[ -x "$rch" && -x "$rchd" ]]; then
        log_json "setup" "Using existing rch/rchd binaries" "local" "cargo build" 0 0 "pass"
        record_pass
        rch_bin="$rch"
        rchd_bin="$rchd"
        return
    fi

    log_json "setup" "Building rch + rchd (debug)" "local" "cargo build -p rch -p rchd" 0 0 "start"
    if (cd "$PROJECT_ROOT" && cargo build -p rch -p rchd >/dev/null 2>&1); then
        log_json "setup" "Build completed" "local" "cargo build" 0 0 "pass"
        record_pass
        rch_bin="$rch"
        rchd_bin="$rchd"
    else
        log_json "setup" "Build failed" "local" "cargo build" 0 0 "fail" "cargo build failed"
        record_fail
        return 1
    fi
}

start_daemon() {
    tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/rch-bd-c7xr-XXXXXX")"
    workers_toml="$tmp_root/workers.toml"
    local runtime_root
    runtime_root="$(e2e_runtime_dir)"
    socket_path="$runtime_root/rch.sock"
    export RCH_CONFIG_DIR="$tmp_root" RCH_SOCKET_PATH="$socket_path"
    export XDG_CACHE_HOME="$tmp_root/cache" XDG_STATE_HOME="$tmp_root/state"
    export XDG_DATA_HOME="$tmp_root/data"

    # Initial config with one worker
    cat > "$workers_toml" <<'WORKERS'
[[workers]]
id = "worker-1"
host = "127.0.0.1"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 4
enabled = true
WORKERS

    log_json "setup" "Starting daemon with hot-reload enabled" "local" "rchd --socket" 0 0 "start"
    RCH_CONFIG_DIR="$tmp_root" RCH_DAEMON_INSTALLS_HOOKS=0 \
        XDG_CACHE_HOME="$tmp_root/cache" XDG_STATE_HOME="$tmp_root/state" \
        RCH_LOG_LEVEL=info RUST_LOG=info RCH_TEST_MODE=1 RCH_MOCK_SSH=1 \
        "$rchd_bin" --socket "$socket_path" --workers-config "$workers_toml" --foreground \
        >"$tmp_root/rchd.log" 2>&1 &
    daemon_pid=$!

    for _ in {1..50}; do
        if [[ -S "$socket_path" ]]; then
            log_json "setup" "Daemon socket ready" "local" "$socket_path" 0 0 "pass"
            record_pass
            return 0
        fi
        sleep 0.1
    done

    log_json "setup" "Daemon socket not ready" "local" "$socket_path" 0 0 "fail" "socket timeout"
    record_fail
    return 1
}

get_worker_count() {
    local output
    output=$(RCH_CONFIG_DIR="$tmp_root" RCH_SOCKET_PATH="$socket_path" RCH_TEST_MODE=1 RCH_MOCK_SSH=1 \
        "$rch_bin" status --json 2>/dev/null) || return 1
    echo "$output" | jq -er '.data.daemon.workers | arrays | length'
}

test_initial_worker_count() {
    log_json "verify" "Checking initial worker count" "local" "rch status" 0 0 "start"
    local count
    count=$(get_worker_count)

    if [[ "$count" == "1" ]]; then
        log_json "verify" "Initial worker count correct" "local" "count=$count" 0 0 "pass"
        record_pass
        return 0
    else
        log_json "verify" "Wrong initial worker count" "local" "count=$count expected=1" 0 0 "fail" "expected 1 got $count"
        record_fail
        return 1
    fi
}

test_cli_reload_add_worker() {
    log_json "execute" "Adding worker to config" "local" "modify workers.toml" 0 0 "start"

    # Add a second worker
    cat > "$workers_toml" <<'WORKERS'
[[workers]]
id = "worker-1"
host = "127.0.0.1"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 4
enabled = true

[[workers]]
id = "worker-2"
host = "127.0.0.2"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 8
enabled = true
WORKERS

    log_json "execute" "Triggering reload via CLI" "local" "rch daemon reload" 0 0 "start"
    local start_ms
    start_ms="$(e2e_now_ms)"

    local output
    if ! output=$(RCH_CONFIG_DIR="$tmp_root" RCH_SOCKET_PATH="$socket_path" RCH_TEST_MODE=1 RCH_MOCK_SSH=1 \
        "$rch_bin" daemon reload --json 2>/dev/null); then
        local duration_ms
        duration_ms=$(( $(e2e_now_ms) - start_ms ))
        log_json "execute" "CLI reload command failed" "local" "rch daemon reload" 0 "$duration_ms" "fail" "command failed"
        record_fail
        return 1
    fi

    local duration_ms
    duration_ms=$(( $(e2e_now_ms) - start_ms ))

    # Check reload response
    local added
    added=$(echo "$output" | jq -er '.data.added')

    if [[ "$added" == "1" ]]; then
        log_json "execute" "Reload added worker" "local" "added=$added" 0 "$duration_ms" "pass"
    else
        log_json "execute" "Reload did not add worker" "local" "added=$added" 0 "$duration_ms" "fail" "expected 1"
        record_fail
        return 1
    fi

    # Verify worker count increased
    sleep 0.5
    local count
    count=$(get_worker_count)

    if [[ "$count" == "2" ]]; then
        log_json "verify" "Worker count increased to 2" "local" "count=$count" 0 0 "pass"
        record_pass
        return 0
    else
        log_json "verify" "Worker count not updated" "local" "count=$count expected=2" 0 0 "fail" "expected 2"
        record_fail
        return 1
    fi
}

test_sighup_reload() {
    log_json "execute" "Adding third worker to config" "local" "modify workers.toml" 0 0 "start"

    # Add a third worker
    cat > "$workers_toml" <<'WORKERS'
[[workers]]
id = "worker-1"
host = "127.0.0.1"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 4
enabled = true

[[workers]]
id = "worker-2"
host = "127.0.0.2"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 8
enabled = true

[[workers]]
id = "worker-3"
host = "127.0.0.3"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 2
enabled = true
WORKERS

    # Check if daemon is still running before sending SIGHUP
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
        log_json "verify" "Daemon exited before SIGHUP reload" "local" "kill -0" 0 0 "fail" "daemon stopped"
        record_fail
        return 1
    fi

    log_json "execute" "Sending SIGHUP to daemon" "local" "kill -HUP $daemon_pid" 0 0 "start"
    if ! kill -HUP "$daemon_pid" 2>/dev/null; then
        log_json "verify" "Failed to send SIGHUP" "local" "kill -HUP" 0 0 "fail" "signal delivery failed"
        record_fail
        return 1
    fi

    # Wait for reload to complete
    sleep 1.0

    # Check if daemon still running after SIGHUP
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
        log_json "verify" "SIGHUP caused daemon to exit" "local" "kill -0" 0 0 "fail" "daemon terminated by SIGHUP"
        record_fail
        return 1
    fi

    local count
    count=$(get_worker_count)

    if [[ "$count" == "3" ]] \
        && grep -Fq 'SIGHUP received, triggering configuration reload' "$tmp_root/rchd.log"; then
        log_json "verify" "SIGHUP reload added worker" "local" "count=$count" 0 0 "pass"
        record_pass
        return 0
    else
        log_json "verify" "SIGHUP reload did not add worker" "local" "count=$count expected=3" 0 0 "fail" "expected 3"
        record_fail
        return 1
    fi
}

test_invalid_config_rejected() {
    log_json "execute" "Writing invalid config" "local" "modify workers.toml" 0 0 "start"

    # Save current worker count
    local count_before
    count_before=$(get_worker_count)

    # Write invalid TOML
    cat > "$workers_toml" <<'INVALID'
this is not valid toml {{{
INVALID

    log_json "execute" "Triggering reload with invalid config" "local" "rch daemon reload" 0 0 "start"

    # Reload should fail but daemon keeps running with old config
    local reload_exit=0
    RCH_CONFIG_DIR="$tmp_root" RCH_SOCKET_PATH="$socket_path" RCH_TEST_MODE=1 RCH_MOCK_SSH=1 \
        "$rch_bin" daemon reload --json >"$tmp_root/invalid-reload.json" \
        2>"$tmp_root/invalid-reload.stderr" || reload_exit=$?
    if [[ "$reload_exit" != 1 ]] || ! jq -e \
        '.success == false and (.error.code | startswith("RCH-E"))' \
        "$tmp_root/invalid-reload.json" >/dev/null; then
        log_json "verify" "Invalid config reload did not return its expected error" "local" "rch daemon reload" 0 0 "fail" "unexpected reload exit or envelope"
        record_fail
        return 1
    fi

    sleep 0.5

    # Verify daemon still running and old config preserved
    local count_after
    count_after=$(get_worker_count)

    if [[ "$count_after" == "$count_before" ]]; then
        log_json "verify" "Invalid config rejected, old config preserved" "local" "count=$count_after" 0 0 "pass"
        record_pass

        # Restore valid config for cleanup
        cat > "$workers_toml" <<'WORKERS'
[[workers]]
id = "worker-1"
host = "127.0.0.1"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 4
enabled = true
WORKERS
        return 0
    else
        log_json "verify" "Config changed despite being invalid" "local" "before=$count_before after=$count_after" 0 0 "fail"
        record_fail
        return 1
    fi
}

test_file_watcher_reload() {
    log_json "execute" "Testing file watcher auto-reload" "local" "modify workers.toml" 0 0 "start"

    # Get current count
    local count_before
    count_before=$(get_worker_count)

    # Add a worker via file modification (no explicit reload)
    cat > "$workers_toml" <<'WORKERS'
[[workers]]
id = "worker-1"
host = "127.0.0.1"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 4
enabled = true

[[workers]]
id = "worker-2"
host = "127.0.0.2"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 8
enabled = true

[[workers]]
id = "worker-3"
host = "127.0.0.3"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 2
enabled = true

[[workers]]
id = "watcher-test"
host = "127.0.0.100"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 1
enabled = true
WORKERS

    # Wait for file watcher debounce + reload
    sleep 1.5

    local count_after
    count_after=$(get_worker_count)

    if [[ "$count_after" -gt "$count_before" ]]; then
        log_json "verify" "File watcher detected change and reloaded" "local" "before=$count_before after=$count_after" 0 0 "pass"
        record_pass
        return 0
    else
        log_json "verify" "File watcher did not auto-reload" "local" "before=$count_before after=$count_after" 0 0 "fail" "no change"
        record_fail
        return 1
    fi
}

main() {
    mkdir -p "$(dirname "$LOG_FILE")"
    : > "$LOG_FILE"

    if ! check_dependencies; then
        return 1
    fi

    if ! build_binaries; then
        return 1
    fi

    if ! start_daemon; then
        return 1
    fi

    # Run tests
    test_initial_worker_count || true
    test_cli_reload_add_worker || true
    test_sighup_reload || true
    test_invalid_config_rejected || true
    test_file_watcher_reload || true

    # Summary
    local elapsed_ms
    elapsed_ms=$(( $(e2e_now_ms) - run_start_ms ))
    local total_count
    total_count=$((passed_tests + failed_tests))

    if [[ $failed_tests -eq 0 ]]; then
        log_json \
            "summary" \
            "bd-c7xr hot-reload tests complete (pass=${passed_tests} fail=${failed_tests} total=${total_count})" \
            "local" \
            "summary" \
            0 \
            "$elapsed_ms" \
            "pass"
        return 0
    else
        log_json \
            "summary" \
            "bd-c7xr hot-reload tests complete (pass=${passed_tests} fail=${failed_tests} total=${total_count})" \
            "local" \
            "summary" \
            0 \
            "$elapsed_ms" \
            "fail" \
            "$failed_tests tests failed"
        return 1
    fi
}

main "$@"
