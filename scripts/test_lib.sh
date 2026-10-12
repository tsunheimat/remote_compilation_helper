#!/usr/bin/env bash
#
# test_lib.sh - Structured test logging helpers for RCH E2E scripts
#
# Provides JSONL logging functions for consistent, parseable test output.
# Source this file at the top of E2E test scripts.
#
# Usage:
#   source "$(dirname "$0")/test_lib.sh"
#   init_test_log "my_test_name"
#   log_json setup "Initializing test"
#   # ... test code ...
#   log_json verify "Checking results"
#
# Output:
#   JSONL logs written to target/test-logs/<test_name>.jsonl
#   Standard markers: TEST START, TEST PASS, TEST FAIL
#

# Get script directory for sourcing dependencies
_TEST_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Source common helpers if available
if [[ -f "$_TEST_LIB_DIR/lib/e2e_common.sh" ]]; then
    # shellcheck disable=SC1091
    source "$_TEST_LIB_DIR/lib/e2e_common.sh"
fi

# ============================================================================
# State variables
# ============================================================================

TEST_LOG_NAME=""
TEST_LOG_FILE=""
TEST_START_MS=0

# ============================================================================
# Core functions
# ============================================================================

# Initialize test logging for a named test.
# Creates log file in target/test-logs/<name>.jsonl
#
# Args:
#   $1 - Test name (used for log file and entries)
#
init_test_log() {
    TEST_LOG_NAME="$1"
    local safe_name
    safe_name="$(echo "$TEST_LOG_NAME" | tr '/:' '_')"

    local log_dir="${PROJECT_ROOT:-$(pwd)}/target/test-logs"
    mkdir -p "$log_dir"

    TEST_LOG_FILE="$log_dir/${safe_name}.jsonl"
    TEST_START_MS="$(_test_now_ms)"

    # Clear previous log
    : > "$TEST_LOG_FILE"

    # Log start marker
    log_json setup "TEST START"

    # Capture terminal info
    log_terminal_info
}

# Log a structured JSON entry.
#
# Args:
#   $1 - Phase: setup | execute | verify | teardown
#   $2 - Message
#   $3 - Optional: JSON data object (must be valid JSON)
#
log_json() {
    local phase="$1"
    local message="$2"
    local data="${3:-}"

    local timestamp
    timestamp="$(e2e_timestamp)" || return 1

    local duration_ms=0
    if [[ $TEST_START_MS -gt 0 ]]; then
        local now_ms
        now_ms="$(_test_now_ms)"
        duration_ms=$((now_ms - TEST_START_MS))
    fi

    # Encode every string and preserve the optional typed JSON data. Invalid
    # caller data must fail instead of appending a malformed success receipt.
    local json_entry has_data=false
    [[ -z "$data" ]] || has_data=true
    json_entry="$(jq -nc --arg timestamp "$timestamp" --arg test_name "$TEST_LOG_NAME" \
        --arg phase "$phase" --arg message "$message" --argjson duration_ms "$duration_ms" \
        --argjson has_data "$has_data" --argjson data "${data:-null}" \
        '{timestamp:$timestamp,test_name:$test_name,phase:$phase,message:$message,duration_ms:$duration_ms}
         + (if $has_data then {data:$data} else {} end)')" || return 1

    # Write to log file
    if [[ -n "$TEST_LOG_FILE" ]]; then
        printf '%s\n' "$json_entry" >> "$TEST_LOG_FILE" || return 1
    fi

    # Also emit to stdout for CI visibility
    echo "[TEST] [$phase] $message"
}

# Capture stdout/stderr from a command.
#
# Args:
#   $1 - Output file for stdout
#   $2... - Command and arguments
#
# Returns: Exit code of the command
#
log_stdout_capture() {
    local outfile="$1"
    shift
    "$@" > "$outfile" 2>&1
}

# Log terminal/environment information for debugging.
#
log_terminal_info() {
    local stdout_tty="false"
    local stderr_tty="false"

    [[ -t 1 ]] && stdout_tty="true"
    [[ -t 2 ]] && stderr_tty="true"

    local term="${TERM:-}"
    local no_color="false"
    local force_color="false"

    [[ -n "${NO_COLOR:-}" ]] && no_color="true"
    [[ -n "${FORCE_COLOR:-}" ]] && force_color="true"

    local width=""
    if [[ -n "${COLUMNS:-}" ]]; then
        width="${COLUMNS}"
    fi

    local data
    data="$(jq -nc --argjson stdout_tty "$stdout_tty" --argjson stderr_tty "$stderr_tty" \
        --arg term "$term" --argjson no_color "$no_color" --argjson force_color "$force_color" \
        --arg width "$width" \
        '{stdout_tty:$stdout_tty,stderr_tty:$stderr_tty,term:$term,no_color:$no_color,force_color:$force_color}
         + (if $width == "" then {} else {width:($width | tonumber)} end)')" || return 1

    log_json setup "Terminal info captured" "$data"
}

# Mark test as passed and exit 0.
#
test_pass() {
    log_json verify "TEST PASS" || exit 1
    exit 0
}

# Mark test as failed and exit 1.
#
# Args:
#   $1 - Failure reason
#
test_fail() {
    local reason="${1:-Unknown failure}"
    local data
    data="$(jq -nc --arg reason "$reason" '{reason:$reason}')" || exit 1
    log_json verify "TEST FAIL" "$data"
    exit 1
}

# Mark test as skipped and exit with skip code.
#
# Args:
#   $1 - Skip reason
#
test_skip() {
    local reason="${1:-}"
    local data
    data="$(jq -nc --arg reason "$reason" '{reason:$reason}')" || exit 1
    log_json setup "TEST SKIP" "$data" || exit 1
    exit "${E2E_SKIP_EXIT:-4}"
}

# ============================================================================
# Internal helpers
# ============================================================================

_test_now_ms() {
    e2e_now_ms
}

# ============================================================================
# Auto-cleanup on exit (optional)
# ============================================================================

# Trap to log test end if not already logged
_test_lib_cleanup() {
    local exit_code=$?
    if [[ $exit_code -ne 0 && $exit_code -ne "${E2E_SKIP_EXIT:-4}" ]]; then
        # Test exited with error without calling test_pass/test_fail
        if [[ -n "$TEST_LOG_NAME" && -n "$TEST_LOG_FILE" ]]; then
            log_json teardown "TEST END (exit code: $exit_code)"
        fi
    fi
}

trap _test_lib_cleanup EXIT
