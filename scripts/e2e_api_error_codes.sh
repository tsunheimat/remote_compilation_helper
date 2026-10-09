#!/usr/bin/env bash
#
# e2e_api_error_codes.sh - Verify all JSON error responses use RCH-Exxx format
#
# Usage:
#   ./scripts/e2e_api_error_codes.sh [OPTIONS]
#
# Options:
#   --verbose          Enable verbose output
#   --help             Show this help message
#
# Purpose:
#   Validates that all CLI commands return properly formatted API errors
#   using the unified RCH-Exxx error code system.
#
# Exit codes:
#   0 - All tests passed
#   1 - Test failure
#   2 - Setup/dependency error
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
export PROJECT_ROOT
VERBOSE="${RCH_E2E_VERBOSE:-0}"
LOG_FILE="${RCH_E2E_LOG:-${TMPDIR:-/tmp}/rch_e2e_error_codes_$(date +%Y%m%d_%H%M%S).jsonl}"
LOG_FILE="${LOG_FILE%.jsonl}.diagnostics.log"
mkdir -p "$(dirname "$LOG_FILE")"
CAPTURE_DIR=$(mktemp -d "$(dirname "$LOG_FILE")/rch-api-error-codes.XXXXXX")

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
    sed -n '1,20p' "$0" | sed 's/^# \{0,1\}//'
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

# Test helper: check the captured JSON response without evaluating its contents.
test_error_format() {
    local test_name="$1"
    local output="$2"
    local expected_pattern="${3:-RCH-E}"

    TESTS_RUN=$((TESTS_RUN + 1))
    log "TEST" "[$test_name] Checking captured error response"

    [[ "$VERBOSE" == "1" ]] && log "DEBUG" "Output: $output"

    # Check if output is valid JSON
    if ! echo "$output" | jq -e '.' >/dev/null 2>&1; then
        log_fail "[$test_name] Output is not valid JSON"
        return 1
    fi

    # Check for error.code field with RCH-E format
    local error_code
    error_code=$(echo "$output" | jq -r '.error.code // empty' 2>/dev/null || true)

    if [[ -z "$error_code" ]]; then
        log_fail "[$test_name] Missing .error.code field"
        return 1
    fi

    if [[ ! "$error_code" =~ $expected_pattern ]]; then
        log_fail "[$test_name] Error code '$error_code' does not match pattern '$expected_pattern'"
        return 1
    fi

    log_pass "[$test_name] Error code: $error_code"
    return 0
}

# Test helper: check that error has required fields
test_error_structure() {
    local test_name="$1"
    local json_output="$2"

    TESTS_RUN=$((TESTS_RUN + 1))
    log "TEST" "[$test_name] Checking error structure"

    local required_fields=("code" "category" "message")
    local missing_fields=()

    for field in "${required_fields[@]}"; do
        if ! echo "$json_output" | jq -e ".error.$field" >/dev/null 2>&1; then
            missing_fields+=("$field")
        fi
    done

    if [[ ${#missing_fields[@]} -gt 0 ]]; then
        log_fail "[$test_name] Missing required fields: ${missing_fields[*]}"
        return 1
    fi

    log_pass "[$test_name] All required fields present"
    return 0
}

# Test helper: validate error category
test_error_category() {
    local test_name="$1"
    local json_output="$2"

    TESTS_RUN=$((TESTS_RUN + 1))
    log "TEST" "[$test_name] Validating error category"

    local category
    category=$(echo "$json_output" | jq -r '.error.category // empty')

    local valid_categories="config network worker build transfer internal"
    if ! echo "$valid_categories" | grep -qw "$category"; then
        log_fail "[$test_name] Invalid category: $category"
        return 1
    fi

    log_pass "[$test_name] Category '$category' is valid"
    return 0
}

# Test helper: check remediation steps
test_remediation_present() {
    local test_name="$1"
    local json_output="$2"

    TESTS_RUN=$((TESTS_RUN + 1))
    log "TEST" "[$test_name] Checking remediation steps"

    local remediation_count
    remediation_count=$(echo "$json_output" | jq '.error.remediation | length // 0')

    if [[ "$remediation_count" -eq 0 ]]; then
        # Remediation is optional but recommended - warn don't fail
        log "WARN" "[$test_name] No remediation steps (optional but recommended)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        return 0
    fi

    log_pass "[$test_name] Has $remediation_count remediation steps"
    return 0
}

run_tests() {
    local rch="$RCH_BIN"

    log "INFO" "=========================================="
    log "INFO" "Starting API Error Code E2E Tests"
    log "INFO" "Log file: $LOG_FILE"
    log "INFO" "Raw stdout and stderr: $CAPTURE_DIR"
    log "INFO" "=========================================="

    # =========================================================================
    # Test 1: Invalid worker probe should return RCH-Exxx code
    # =========================================================================
    log "INFO" "Test 1: Invalid worker probe error format"
    local output
    "$rch" workers probe nonexistent-worker --json >"$CAPTURE_DIR/probe-json.stdout" 2>"$CAPTURE_DIR/probe-json.stderr" || true
    output=$(<"$CAPTURE_DIR/probe-json.stdout")
    [[ "$VERBOSE" == "1" ]] && log "DEBUG" "Output: $output"

    test_error_format "probe-invalid-worker" "$output" "RCH-E"
    test_error_structure "probe-structure" "$output"
    test_error_category "probe-category" "$output"
    test_remediation_present "probe-remediation" "$output"

    # =========================================================================
    # Test 2: Status command with daemon not running
    # =========================================================================
    log "INFO" "Test 2: Daemon not running error"
    # Temporarily unset socket path to ensure daemon not found
    local orig_socket="${RCH_SOCKET_PATH:-}"
    export RCH_SOCKET_PATH="/tmp/nonexistent-rch-socket-$$"

    local status_exit=0
    "$rch" status --json >"$CAPTURE_DIR/status-json.stdout" 2>"$CAPTURE_DIR/status-json.stderr" || status_exit=$?
    output=$(<"$CAPTURE_DIR/status-json.stdout")
    [[ "$VERBOSE" == "1" ]] && log "DEBUG" "Output: $output"

    if [[ "$status_exit" == 1 ]] && echo "$output" | jq -e '.success == false and .error' >/dev/null 2>&1; then
        test_error_format "daemon-not-running" "$output" "RCH-E"
        test_error_structure "daemon-structure" "$output"
    else
        log_fail "[daemon-not-running] Expected exit 1 and error envelope for the deliberately absent socket"
    fi

    # Restore socket path
    if [[ -n "$orig_socket" ]]; then
        export RCH_SOCKET_PATH="$orig_socket"
    else
        unset RCH_SOCKET_PATH
    fi

    # =========================================================================
    # Test 3: Invalid config file error
    # =========================================================================
    log "INFO" "Test 3: Invalid config file error"
    local invalid_config_dir invalid_config
    invalid_config_dir=$(mktemp -d)
    invalid_config="$invalid_config_dir/config.toml"
    echo 'invalid toml [' > "$invalid_config"

    local config_exit=0
    RCH_CONFIG_DIR="$invalid_config_dir" "$rch" config show --json >"$CAPTURE_DIR/config-json.stdout" 2>"$CAPTURE_DIR/config-json.stderr" || config_exit=$?
    output=$(<"$CAPTURE_DIR/config-json.stdout")
    [[ "$VERBOSE" == "1" ]] && log "DEBUG" "Output: $output"

    if [[ "$config_exit" == 1 ]] && echo "$output" | jq -e \
        --arg path "$invalid_config" '.success == false and .error.category == "config" and (.error.details | contains($path))' >/dev/null 2>&1; then
        test_error_format "config-invalid" "$output" "RCH-E"
    else
        log_fail "[config-invalid] Expected exit 1 and configuration error identifying the malformed file"
    fi

    # =========================================================================
    # Test 4: Errors use stderr, not stdout (stream separation)
    # =========================================================================
    log "INFO" "Test 4: Error stream separation"
    TESTS_RUN=$((TESTS_RUN + 1))

    local stdout_file stderr_file
    stdout_file="$CAPTURE_DIR/probe-plain.stdout"
    stderr_file="$CAPTURE_DIR/probe-plain.stderr"

    local probe_exit=0
    "$rch" workers probe nonexistent-worker >"$stdout_file" 2>"$stderr_file" || probe_exit=$?

    if [[ "$probe_exit" == 1 && -s "$stderr_file" && ! -s "$stdout_file" ]] \
        && grep -q 'RCH-E' "$stderr_file"; then
        log_pass "[stream-separation] Error output correctly routed"
    else
        log_fail "[stream-separation] Expected exit 1, coded error on stderr, and empty stdout"
    fi


    # =========================================================================
    # Test 5: JSON error format is parseable
    # =========================================================================
    log "INFO" "Test 5: JSON error parseable by jq"
    TESTS_RUN=$((TESTS_RUN + 1))

    "$rch" workers probe nonexistent-worker --json >"$CAPTURE_DIR/probe-fields.stdout" 2>"$CAPTURE_DIR/probe-fields.stderr" || true
    output=$(<"$CAPTURE_DIR/probe-fields.stdout")

    # Try to extract all standard fields
    local fields_ok=1
    for field in api_version timestamp success; do
        if ! echo "$output" | jq -e "has(\"$field\")" >/dev/null 2>&1; then
            log "WARN" "Missing top-level field: $field"
            fields_ok=0
        fi
    done

    if [[ "$fields_ok" == "1" ]]; then
        log_pass "[json-parseable] All standard response fields present"
    else
        log_fail "[json-parseable] Some response fields missing"
    fi

    # =========================================================================
    # Test 6: Error codes are unique (no duplicates in catalog)
    # =========================================================================
    log "INFO" "Test 6: Verify unit tests pass for error code uniqueness"
    TESTS_RUN=$((TESTS_RUN + 1))

    # This is validated by unit tests, but we verify they pass
    local test_output test_exit=0
    test_output=$(cargo test -p rch-common --lib -- api:: --quiet 2>&1) || test_exit=$?
    if [[ "$test_exit" == 0 ]] && echo "$test_output" | grep -qE 'test result: ok\. [1-9][0-9]* passed'; then
        log_pass "[unit-tests] API unit tests pass"
    else
        log_fail "[unit-tests] API unit tests failed or selected no tests (exit $test_exit)"
        [[ "$VERBOSE" == "1" ]] && log "DEBUG" "$test_output"
    fi

    # =========================================================================
    # Test 7: NO_COLOR doesn't break JSON output
    # =========================================================================
    log "INFO" "Test 7: NO_COLOR preserves JSON"
    TESTS_RUN=$((TESTS_RUN + 1))

    NO_COLOR=1 "$rch" workers probe nonexistent-worker --json >"$CAPTURE_DIR/probe-no-color.stdout" 2>"$CAPTURE_DIR/probe-no-color.stderr" || true
    output=$(<"$CAPTURE_DIR/probe-no-color.stdout")

    if echo "$output" | jq -e '.' >/dev/null 2>&1; then
        log_pass "[no-color] JSON output valid with NO_COLOR"
    else
        log_fail "[no-color] JSON output broken with NO_COLOR"
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

    log "INFO" "All API error code tests passed!"
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
