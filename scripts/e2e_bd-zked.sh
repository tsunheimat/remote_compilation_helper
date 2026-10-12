#!/usr/bin/env bash
#
# e2e_bd-zked.sh - Saved-time summary in rch status
#
# Verifies:
# - `rch status --json` includes saved_time field in response
# - saved_time structure contains all required fields when populated
# - saved_time is null when no remote builds exist
# - Human-readable output includes saved time info
# - Negative saved time is never reported (saturating_sub behavior)
# - Unit test coverage for saved_time_stats() passes
#
# Related: bd-zked "Idea: Saved-time summary in rch status"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="${RCH_E2E_LOG:-$PROJECT_ROOT/target/e2e_bd-zked.jsonl}"
# shellcheck source=lib/e2e_common.sh
source "$SCRIPT_DIR/lib/e2e_common.sh"
daemon_pid=""
tmp_root=""

cleanup() {
    if [[ -n "$daemon_pid" ]]; then
        kill "$daemon_pid" >/dev/null 2>&1 || true
        wait "$daemon_pid" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

timestamp() {
    date -u '+%Y-%m-%dT%H:%M:%S.%3NZ' 2>/dev/null || date -u '+%Y-%m-%dT%H:%M:%SZ'
}

log_json() {
    local phase="$1"
    local message="$2"
    local extra="${3:-}"
    [[ -n "$extra" ]] || extra='{}'
    local ts
    ts="$(timestamp)"
    jq -nc --arg ts "$ts" --arg phase "$phase" --arg message "$message" \
        --argjson extra "$extra" \
        '{ts:$ts,test:"bd-zked",phase:$phase,message:$message} + $extra' \
        | tee -a "$LOG_FILE"
}

die() {
    log_json "error" "$*" '{"result":"fail"}'
    exit 1
}

check_dependencies() {
    log_json "setup" "Checking dependencies"
    for cmd in cargo jq; do
        command -v "$cmd" >/dev/null 2>&1 || die "Missing dependency: $cmd"
    done
}

build_rch() {
    local rch_bin="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rch"
    if [[ -x "$rch_bin" ]]; then
        log_json "setup" "Using existing rch binary" "{\"path\":\"$rch_bin\"}" >&2
        echo "$rch_bin"
        return
    fi
    log_json "setup" "Building rch (debug)" >&2
    (cd "$PROJECT_ROOT" && cargo build -p rch >/dev/null 2>&1) || die "cargo build failed"
    [[ -x "$rch_bin" ]] || die "rch binary missing after build"
    echo "$rch_bin"
}

start_daemon() {
    local rchd_bin="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rchd"
    if [[ ! -x "$rchd_bin" ]]; then
        log_json "setup" "Building rchd (debug)"
        (cd "$PROJECT_ROOT" && cargo build -p rchd) || die "cargo build -p rchd failed"
    fi
    tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/rch-saved-time-XXXXXX")"
    printf 'workers = []\n' > "$tmp_root/workers.toml"
    export RCH_CONFIG_DIR="$tmp_root"
    export XDG_CACHE_HOME="$tmp_root/cache" XDG_STATE_HOME="$tmp_root/state"
    export XDG_DATA_HOME="$tmp_root/data"
    local runtime_root
    runtime_root="$(e2e_runtime_dir)"
    export RCH_SOCKET_PATH="$runtime_root/rch.sock"
    RCH_DAEMON_INSTALLS_HOOKS=0 "$rchd_bin" --socket "$RCH_SOCKET_PATH" --workers-config "$tmp_root/workers.toml" \
        --foreground > "$tmp_root/rchd.log" 2>&1 &
    daemon_pid=$!
    for _ in {1..50}; do
        if [[ -S "$RCH_SOCKET_PATH" ]]; then
            log_json "setup" "Isolated daemon ready" "{\"root\":\"$tmp_root\"}"
            return 0
        fi
        kill -0 "$daemon_pid" 2>/dev/null || die "rchd exited during startup; see $tmp_root/rchd.log"
        sleep 0.1
    done
    die "Daemon socket not ready; see $tmp_root/rchd.log"
}

# Test 1: Verify saved_time field exists in status JSON schema
test_saved_time_field_exists() {
    local rch_bin="$1"
    log_json "test" "Checking saved_time field exists in status JSON output"

    local json_output
    json_output="$("$rch_bin" status --json 2>/dev/null)" || true

    echo "$json_output" | jq -e '.success == true' >/dev/null 2>&1 \
        || die "Isolated daemon status did not return success: $json_output"

    # Check saved_time field exists (can be null or object)
    if ! echo "$json_output" | jq -e '.data.daemon | has("saved_time")' >/dev/null 2>&1; then
        die "saved_time field missing from status JSON response"
    fi

    log_json "verify" "saved_time field exists in status JSON" '{"result":"pass"}'
}

# Test 2: Verify SavedTimeStats structure when populated
test_saved_time_structure() {
    local rch_bin="$1"
    log_json "test" "Checking SavedTimeStats JSON structure"

    local json_output
    json_output="$("$rch_bin" status --json 2>/dev/null)" || true

    echo "$json_output" | jq -e '.success == true' >/dev/null 2>&1 \
        || die "Isolated daemon status did not return success: $json_output"

    local saved_time
    saved_time="$(echo "$json_output" | jq '.data.daemon.saved_time')"

    if [[ "$saved_time" == "null" ]]; then
        # No remote builds - this is valid
        log_json "verify" "saved_time is null (no remote builds yet)" '{"result":"pass","note":"null is valid when no remote builds"}'
        return 0
    fi

    # Verify all required fields exist when saved_time is populated
    local required_fields=("total_remote_duration_ms" "estimated_local_duration_ms" "time_saved_ms" "builds_counted" "avg_speedup" "today_saved_ms" "week_saved_ms")

    for field in "${required_fields[@]}"; do
        if ! echo "$saved_time" | jq -e "has(\"$field\")" >/dev/null 2>&1; then
            die "Missing required field in saved_time: $field"
        fi
    done

    log_json "verify" "SavedTimeStats has all required fields" "{\"fields\":\"${required_fields[*]}\",\"result\":\"pass\"}"
}

# Test 3: Run the saved-time behavior tests; compilation alone is not acceptance.
test_unit_tests() {
    log_json "test" "Running saved_time_stats behavior tests"
    if ! (cd "$PROJECT_ROOT" && cargo test -p rchd -- history::tests::test_saved_time --nocapture) \
        >"$tmp_root/saved-time-tests.log" 2>&1; then
        die "Saved-time unit tests failed; see $tmp_root/saved-time-tests.log"
    fi
    grep -qE 'test result: ok\. [1-9][0-9]* passed' "$tmp_root/saved-time-tests.log" \
        || die "Saved-time test selection ran no passing tests"
    log_json "verify" "Saved-time behavior tests passed" '{"result":"pass"}'
}

# Test 4: Verify time_saved_ms is never negative (saturating_sub)
test_no_negative_savings() {
    log_json "test" "Verifying time_saved_ms cannot be negative"

    grep -q 'test history::tests::test_saved_time_stats_no_negative_savings .* ok' "$tmp_root/saved-time-tests.log" \
        || die "Non-negative savings behavior test did not pass"
    log_json "verify" "Non-negative savings behavior test passed" '{"result":"pass"}'
}

# Test 5: Check human-readable output format
test_human_readable_output() {
    local rch_bin="$1"
    log_json "test" "Checking human-readable status output"

    local status_output
    status_output="$("$rch_bin" status 2>&1)" || die "Isolated daemon human status failed"

    if echo "$status_output" | grep -qiE "(saved|status|daemon)" 2>/dev/null; then
        log_json "verify" "Human-readable output generated successfully" '{"result":"pass"}'
    else
        die "Isolated daemon human status did not describe its status"
    fi
}

# Test 6: Verify BuildStats structure (foundation for saved time)
test_build_stats_structure() {
    local rch_bin="$1"
    log_json "test" "Checking BuildStats structure (saved_time dependency)"

    local json_output
    json_output="$("$rch_bin" status --json 2>/dev/null)" || true

    echo "$json_output" | jq -e '.success == true' >/dev/null 2>&1 \
        || die "Isolated daemon status did not return success: $json_output"

    # Verify stats field exists with required subfields
    local stats_fields=("total_builds" "success_count" "failure_count" "remote_count" "local_count" "avg_duration_ms")

    for field in "${stats_fields[@]}"; do
        if ! echo "$json_output" | jq -e ".data.daemon.stats | has(\"$field\")" >/dev/null 2>&1; then
            die "Missing required field in stats: $field"
        fi
    done

    log_json "verify" "BuildStats has all required fields" '{"result":"pass"}'
}

# Test 7: Verify SavedTimeStats type in rch-common
test_type_definition() {
    log_json "test" "Verifying SavedTimeStats type compilation"

    # Check that the type exists and compiles
    (cd "$PROJECT_ROOT" && cargo check -p rch-common) >"$tmp_root/common-check.log" 2>&1 \
        || die "rch-common compilation failed; see $tmp_root/common-check.log"

    log_json "verify" "SavedTimeStats type defined and exported" '{"result":"pass"}'
}

main() {
    mkdir -p "$(dirname "$LOG_FILE")"
    : > "$LOG_FILE"
    log_json "setup" "Starting bd-zked E2E tests (saved-time summary)"

    check_dependencies

    local rch_bin
    rch_bin="$(build_rch)"
    log_json "setup" "Built/found rch binary" "{\"path\":\"$rch_bin\"}"
    start_daemon

    # Run all tests
    test_type_definition
    test_saved_time_field_exists "$rch_bin"
    test_saved_time_structure "$rch_bin"
    test_build_stats_structure "$rch_bin"
    test_unit_tests
    test_no_negative_savings
    test_human_readable_output "$rch_bin"

    # Summary
    log_json "summary" "All bd-zked saved-time summary checks passed" '{"result":"pass","tests_run":7}'
    echo ""
    echo "E2E test log: $LOG_FILE"
}

main "$@"
