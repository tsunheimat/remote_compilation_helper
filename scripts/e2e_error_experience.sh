#!/usr/bin/env bash
#
# e2e_error_experience.sh - E2E Test: Error Experience Phase 4
#
# Tests that errors are displayed beautifully and are actionable.
# Validates RCH error messages follow the error experience guidelines.
#
# Usage:
#   ./scripts/e2e_error_experience.sh [OPTIONS]
#
# Options:
#   --verbose          Enable verbose output
#   --help             Show this help message
#
# Exit codes:
#   0 - All tests passed
#   1 - Test failure
#   2 - Setup/dependency error
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
export PROJECT_ROOT
VERBOSE="${RCH_E2E_VERBOSE:-0}"
LOG_FILE="${RCH_E2E_LOG:-/tmp/rch_e2e_error_experience_$(date +%Y%m%d_%H%M%S).jsonl}"
LOG_FILE="${LOG_FILE%.jsonl}.diagnostics.log"

# Structured JSONL logging
# shellcheck disable=SC1091
source "$SCRIPT_DIR/test_lib.sh"
init_test_log "$(basename "${BASH_SOURCE[0]}" .sh)"

# Counters
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0

timestamp() { date -u '+%Y-%m-%dT%H:%M:%S.%3NZ'; }

fail_with_code() {
    local exit_code="$1"
    shift
    local reason="$*"
    log_json verify "TEST FAIL" "{\"reason\":\"$reason\"}"
    exit "$exit_code"
}

log() {
    local level="$1"; shift
    local ts; ts="$(timestamp)"
    local msg="[$ts] [$level] $*"
    echo "$msg" | tee -a "$LOG_FILE"

    local phase="execute"
    case "$level" in
        INFO|DEBUG) phase="setup" ;;
        PASS|FAIL) phase="verify" ;;
        ERROR) phase="verify" ;;
        TEST) phase="execute" ;;
    esac
    log_json "$phase" "$msg"
}

log_pass() {
    TESTS_PASSED=$((TESTS_PASSED + 1))
    log "PASS" "$*"
}

log_fail() {
    TESTS_FAILED=$((TESTS_FAILED + 1))
    log "FAIL" "$*"
}

die() { log "ERROR" "$*"; fail_with_code 2 "$*"; }

usage() {
    sed -n '1,18p' "$0" | sed 's/^# \{0,1\}//'
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --verbose|-v) VERBOSE="1"; shift ;;
            --help|-h) usage; exit 0 ;;
            *) log "ERROR" "Unknown option: $1"; exit 3 ;;
        esac
    done
}

check_dependencies() {
    log "INFO" "Checking dependencies..."
    for cmd in cargo jq; do
        command -v "$cmd" >/dev/null 2>&1 || die "Missing: $cmd"
    done
    log "INFO" "Dependencies OK"
}

build_binaries() {
    local target_dir="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
    RCH_BIN="$target_dir/debug/rch"
    if [[ ! -x "$RCH_BIN" && -x "$target_dir/release/rch" ]]; then
        RCH_BIN="$target_dir/release/rch"
    fi
    if [[ -x "$RCH_BIN" ]]; then
        log "INFO" "Using existing rch: $RCH_BIN"
        return 0
    fi
    log "INFO" "Building rch (debug)..."
    cd "$PROJECT_ROOT"
    if ! cargo build -p rch 2>&1 | tee -a "$LOG_FILE" | tail -3; then
        die "Build failed"
    fi
    [[ -x "$RCH_BIN" ]] || die "Binary missing: rch"
    log "INFO" "Build OK"
}

run_tests() {
    local rch="$RCH_BIN"

    log "INFO" "=========================================="
    log "INFO" "Starting Error Experience E2E Tests"
    log "INFO" "Log file: $LOG_FILE"
    log "INFO" "=========================================="

    # =========================================================================
    # Test 1: Missing-worker error contains RCH-E error code
    # =========================================================================
    log "INFO" "Test 1: Missing-worker error format"
    TESTS_RUN=$((TESTS_RUN + 1))

    local stderr_file
    stderr_file=$(mktemp)

    # This lookup fails before any SSH request, independent of fleet state.
    local probe_exit=0
    "$rch" workers probe nonexistent-worker 2>"$stderr_file" || probe_exit=$?

    [[ "$VERBOSE" == "1" ]] && log "DEBUG" "stderr: $(cat "$stderr_file")"

    if [[ "$probe_exit" == 1 ]] && grep -q "RCH-E" "$stderr_file"; then
        log_pass "Missing-worker error exits 1 and contains RCH-E code"
    else
        log_fail "Expected exit 1 and RCH-E code for missing worker (exit=$probe_exit)"
    fi

    # =========================================================================
    # Test 2: Error includes remediation steps
    # =========================================================================
    log "INFO" "Test 2: Remediation steps present"
    TESTS_RUN=$((TESTS_RUN + 1))

    stderr_file=$(mktemp)
    "$rch" workers probe nonexistent-worker 2>"$stderr_file" || true

    if grep -qiE "check|verify|try|ensure|run|ping|ssh" "$stderr_file"; then
        log_pass "Error includes remediation suggestions"
    else
        log_fail "No remediation steps in error"
    fi

    # =========================================================================
    # Test 3: Error context shows worker/host info
    # =========================================================================
    log "INFO" "Test 3: Error context preservation"
    TESTS_RUN=$((TESTS_RUN + 1))

    stderr_file=$(mktemp)
    "$rch" workers probe nonexistent-worker 2>"$stderr_file" || true

    if grep -q "nonexistent" "$stderr_file"; then
        log_pass "Error shows relevant context (worker name)"
    else
        log_fail "Worker name not shown in error context"
    fi

    # =========================================================================
    # Test 4: Errors go to stderr, not stdout
    # =========================================================================
    log "INFO" "Test 4: Error stream separation"
    TESTS_RUN=$((TESTS_RUN + 1))

    local stdout_file
    stdout_file=$(mktemp)
    stderr_file=$(mktemp)

    probe_exit=0
    "$rch" workers probe nonexistent-worker >"$stdout_file" 2>"$stderr_file" || probe_exit=$?

    if [[ "$probe_exit" == 1 && -s "$stderr_file" && ! -s "$stdout_file" ]]; then
        log_pass "Errors correctly go to stderr"
    else
        log_fail "Expected exit 1, nonempty stderr, and empty stdout in human mode"
    fi

    # =========================================================================
    # Test 5: JSON error format with required fields
    # =========================================================================
    log "INFO" "Test 5: JSON error format"
    TESTS_RUN=$((TESTS_RUN + 1))

    local json_output
    probe_exit=0
    json_output=$("$rch" workers probe nonexistent-worker --json 2>"$stderr_file") || probe_exit=$?

    [[ "$VERBOSE" == "1" ]] && log "DEBUG" "JSON: $json_output"

    local json_ok=1

    # Check it's valid JSON
    if [[ "$probe_exit" != 1 ]] || ! echo "$json_output" | jq -e '.success == false' >/dev/null 2>&1; then
        log_fail "Expected exit 1 and JSON error envelope (exit=$probe_exit)"
        json_ok=0
    fi

    # Check for error.code field
    if [[ "$json_ok" == "1" ]]; then
        if echo "$json_output" | jq -e '.error.code' >/dev/null 2>&1; then
            local error_code
            error_code=$(echo "$json_output" | jq -r '.error.code')
            if [[ "$error_code" =~ ^RCH-E ]]; then
                log_pass "JSON error has valid RCH-E code: $error_code"
            else
                log_fail "JSON error code doesn't match RCH-E format: $error_code"
            fi
        else
            log_fail "JSON error missing .error.code field"
        fi
    fi

    # =========================================================================
    # Test 6: JSON error includes remediation array
    # =========================================================================
    log "INFO" "Test 6: JSON remediation array"
    TESTS_RUN=$((TESTS_RUN + 1))

    if echo "$json_output" | jq -e '.error.remediation | type == "array" and length > 0 and all(.[]; type == "string" and length > 0)' >/dev/null 2>&1; then
        local remediation_count
        remediation_count=$(echo "$json_output" | jq '.error.remediation | length')
        log_pass "JSON error has $remediation_count remediation steps"
    else
        log_fail "JSON error must include actionable remediation steps"
    fi

    # =========================================================================
    # Test 7: NO_COLOR disables styling but keeps content
    # =========================================================================
    log "INFO" "Test 7: NO_COLOR preserves content"
    TESTS_RUN=$((TESTS_RUN + 1))

    local no_color_output no_color_exit=0
    no_color_output=$(NO_COLOR=1 "$rch" workers probe nonexistent-worker 2>&1) || no_color_exit=$?

    [[ "$VERBOSE" == "1" ]] && log "DEBUG" "NO_COLOR output: $no_color_output"

    if [[ "$no_color_exit" == 1 && "$no_color_output" == *RCH-E* ]] \
        && ! echo "$no_color_output" | grep -Fq $'\033['; then
        log_pass "NO_COLOR disables styling, preserves content"
    else
        log_fail "Expected exit 1 and a coded error without ANSI styling under NO_COLOR"
    fi

    # =========================================================================
    # Test 8: Config parse error shows location info
    # =========================================================================
    log "INFO" "Test 8: Config error location"
    TESTS_RUN=$((TESTS_RUN + 1))

    local invalid_config_dir invalid_config
    invalid_config_dir=$(mktemp -d)
    invalid_config="$invalid_config_dir/config.toml"
    echo 'invalid toml [' > "$invalid_config"

    stderr_file=$(mktemp)
    local config_exit=0
    RCH_CONFIG_DIR="$invalid_config_dir" "$rch" config show >"$stdout_file" 2>"$stderr_file" || config_exit=$?

    [[ "$VERBOSE" == "1" ]] && log "DEBUG" "Config error: $(cat "$stderr_file")"

    if [[ "$config_exit" == 1 ]] \
        && grep -Fq "$invalid_config" "$stderr_file" \
        && grep -qiE 'line[[:space:]]+[0-9]+' "$stderr_file"; then
        log_pass "Config error shows file/parse info"
    else
        log_fail "Expected invalid-config exit 1, config path, and parse line (exit=$config_exit)"
    fi

    # =========================================================================
    # Test 9: Error categories are present
    # =========================================================================
    log "INFO" "Test 9: Error category in JSON"
    TESTS_RUN=$((TESTS_RUN + 1))

    if echo "$json_output" | jq -e '.error.category' >/dev/null 2>&1; then
        local category
        category=$(echo "$json_output" | jq -r '.error.category')
        local valid_categories="config network worker build transfer internal"
        if echo "$valid_categories" | grep -qw "$category"; then
            log_pass "Valid error category: $category"
        else
            log_fail "Invalid error category: $category"
        fi
    else
        log_fail "JSON error missing .error.category field"
    fi

    # =========================================================================
    # Test 10: Error message field is present and meaningful
    # =========================================================================
    log "INFO" "Test 10: Error message field"
    TESTS_RUN=$((TESTS_RUN + 1))

    if echo "$json_output" | jq -e '.error.message' >/dev/null 2>&1; then
        local message
        message=$(echo "$json_output" | jq -r '.error.message')
        if [[ ${#message} -gt 5 ]]; then
            log_pass "Error message present: ${message:0:50}..."
        else
            log_fail "Error message too short: $message"
        fi
    else
        log_fail "JSON error missing .error.message field"
    fi

    # =========================================================================
    # Test 11: Unit tests for error module pass
    # =========================================================================
    log "INFO" "Test 11: Unit tests for error module"
    TESTS_RUN=$((TESTS_RUN + 1))

    cd "$PROJECT_ROOT"
    local test_output error_test_count=0
    if test_output=$(cargo test -p rch-common --lib -- ui::error 2>&1); then
        error_test_count=$(printf '%s\n' "$test_output" | awk '/^test result: ok\./ {sum += $4} END {print sum+0}')
        if [[ "$error_test_count" -gt 0 ]]; then
            log_pass "Error unit tests pass: $error_test_count passed"
        else
            log_fail "Error unit command ran zero tests"
        fi
    else
        log_fail "Error unit tests failed"
    fi
    printf '%s\n' "$test_output" >"${LOG_FILE%.log}.unit.log"

    # =========================================================================
    # Test 12: Minimum test count (15+)
    # =========================================================================
    log "INFO" "Test 12: Minimum test count check"
    TESTS_RUN=$((TESTS_RUN + 1))

    if [[ "$error_test_count" -ge 15 ]]; then
        log_pass "Executed error test count ($error_test_count) meets minimum requirement (15+)"
    else
        log_fail "Executed error test count ($error_test_count) below minimum requirement (15+)"
    fi
}

print_summary() {
    log "INFO" "=========================================="
    log "INFO" "Test Summary"
    log "INFO" "=========================================="
    log "INFO" "Total tests: $TESTS_RUN"
    log "INFO" "Passed: $TESTS_PASSED"
    log "INFO" "Failed: $TESTS_FAILED"
    log "INFO" "Log file: $LOG_FILE"

    if [[ "$TESTS_FAILED" -gt 0 ]]; then
        log "FAIL" "Some tests failed!"
        test_fail "Some tests failed"
    fi

    log "INFO" "All Error Experience E2E tests passed!"
    test_pass
}

main() {
    parse_args "$@"
    check_dependencies
    build_binaries
    run_tests
    print_summary
}

main "$@"
