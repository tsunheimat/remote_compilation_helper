#!/usr/bin/env bash
# E2E: stderr/stdout Stream Isolation Test
#
# Verifies that rich output ONLY goes to stderr, and stdout remains
# pristine for machine-parseable data.
#
# This is critical for:
# - Agent JSON parsing
# - Compiler output capture
# - Pipeline composition (rch --json exec -- cargo build | jq)
#
# Implements bead: bd-2ans

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
export PROJECT_ROOT
TEST_LOG="${PROJECT_ROOT}/target/test_stream_isolation.log"

# Structured JSONL logging
# shellcheck disable=SC1091
source "$SCRIPT_DIR/test_lib.sh"
init_test_log "$(basename "${BASH_SOURCE[0]}" .sh)"

# Ensure target directory exists
mkdir -p "${PROJECT_ROOT}/target"

log() {
    echo "[$(date +%H:%M:%S)] $*" | tee -a "$TEST_LOG"
    log_json execute "$*"
}
pass() {
    log "PASS: $*"
    log_json verify "PASS $*"
}
fail() {
    log "FAIL: $*"
    log_json verify "FAIL $*"
    test_fail "$*"
}

# Clean up previous log
: > "$TEST_LOG"

log "Starting Stream Isolation Tests"
log "Project root: $PROJECT_ROOT"
log ""

# =============================================================================
# Build rch (or use existing binary)
# =============================================================================
RCH="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rch"
if [[ ! -x "$RCH" && -x "${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/release/rch" ]]; then
    RCH="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/release/rch"
fi

if [[ -x "$RCH" ]]; then
    log "Using existing binary: $RCH"
else
    log "Building rch..."
    if ! cargo build -p rch 2>&1 | tail -10; then
        log ""
        log "NOTE: Build failed. This may be due to rich_rust dependency issues."
        log "      To run these tests, either:"
        log "      1. Fix the rich_rust crate"
        log "      2. Pre-build rch and place at: $RCH"
        log ""
        fail "Failed to build rch"
    fi
fi

if [[ ! -x "$RCH" ]]; then
    fail "rch binary not found at $RCH"
fi
log "Using binary: $RCH"
log ""

# Create temp files for stdout/stderr capture
STDOUT_FILE="$(mktemp)"
STDERR_FILE="$(mktemp)"
trap '_test_lib_cleanup' EXIT

# =============================================================================
# TEST 1: rch status --json outputs ONLY to stdout
# =============================================================================
log "TEST 1: rch status --json stream isolation"

"$RCH" status --json > "$STDOUT_FILE" 2> "$STDERR_FILE" || true

# stdout must be valid JSON (or empty for certain error states)
STDOUT_CONTENT=$(cat "$STDOUT_FILE")
if [[ -n "$STDOUT_CONTENT" ]]; then
    if ! echo "$STDOUT_CONTENT" | jq -e . >/dev/null 2>&1; then
        log "stdout content: $STDOUT_CONTENT"
        fail "stdout is not valid JSON"
    fi

    # stdout must NOT contain ANSI codes
    if echo "$STDOUT_CONTENT" | grep -Fq $'\033['; then
        log "stdout content: $STDOUT_CONTENT"
        fail "stdout contains ANSI escape codes!"
    fi
fi

pass "--json flag isolates JSON to stdout"
log ""

# =============================================================================
# TEST 2: Hook output isolation (JSON to stdout only)
# =============================================================================
log "TEST 2: Hook output stream isolation"

HOOK_INPUT='{"tool_name":"Bash","tool_input":{"command":"echo hello"}}'
echo "$HOOK_INPUT" | "$RCH" > "$STDOUT_FILE" 2> "$STDERR_FILE"
HOOK_EXIT=$?

# stdout must be empty or valid JSON
STDOUT_CONTENT=$(cat "$STDOUT_FILE")
if [[ -n "$STDOUT_CONTENT" ]]; then
    if ! echo "$STDOUT_CONTENT" | jq -e . >/dev/null 2>&1; then
        log "hook stdout: $STDOUT_CONTENT"
        fail "hook stdout is not valid JSON"
    fi

    if echo "$STDOUT_CONTENT" | grep -Fq $'\033['; then
        log "hook stdout: $STDOUT_CONTENT"
        fail "hook stdout contains ANSI codes"
    fi
fi

pass "Hook output properly isolated"
log ""

# =============================================================================
# TEST 3: NO_COLOR environment variable
# =============================================================================
log "TEST 3: NO_COLOR environment variable"

export NO_COLOR=1

# Run status and capture output
"$RCH" status > "$STDOUT_FILE" 2> "$STDERR_FILE" || true

STDOUT_CONTENT=$(cat "$STDOUT_FILE")
STDERR_CONTENT=$(cat "$STDERR_FILE")

# Neither stdout nor stderr should have ANSI codes with NO_COLOR
if echo "$STDOUT_CONTENT" | grep -Fq $'\033['; then
    fail "stdout has ANSI codes with NO_COLOR=1"
fi

if echo "$STDERR_CONTENT" | grep -Fq $'\033['; then
    fail "stderr has ANSI codes with NO_COLOR=1"
fi

unset NO_COLOR
pass "NO_COLOR=1 disables all ANSI codes"
log ""

# =============================================================================
# TEST 4: Workers list --json isolation
# =============================================================================
log "TEST 4: rch workers list --json stream isolation"

"$RCH" workers list --json > "$STDOUT_FILE" 2> "$STDERR_FILE" || true

STDOUT_CONTENT=$(cat "$STDOUT_FILE")
if [[ -n "$STDOUT_CONTENT" ]]; then
    if ! echo "$STDOUT_CONTENT" | jq -e . >/dev/null 2>&1; then
        log "workers stdout: $STDOUT_CONTENT"
        fail "workers list --json stdout not valid JSON"
    fi

    if echo "$STDOUT_CONTENT" | grep -Fq $'\033['; then
        fail "workers list --json stdout has ANSI codes"
    fi
fi

pass "workers list --json isolated correctly"
log ""

# =============================================================================
# TEST 5: Config show --json isolation
# =============================================================================
log "TEST 5: rch config show --json stream isolation"

"$RCH" config show --json > "$STDOUT_FILE" 2> "$STDERR_FILE" || true

STDOUT_CONTENT=$(cat "$STDOUT_FILE")
if [[ -n "$STDOUT_CONTENT" ]]; then
    if ! echo "$STDOUT_CONTENT" | jq -e . >/dev/null 2>&1; then
        log "config stdout: $STDOUT_CONTENT"
        fail "config show --json stdout not valid JSON"
    fi

    if echo "$STDOUT_CONTENT" | grep -Fq $'\033['; then
        fail "config show --json stdout has ANSI codes"
    fi
fi

pass "config show --json isolated correctly"
log ""

# =============================================================================
# TEST 6: Piped output detection (no TTY = minimal output)
# =============================================================================
log "TEST 6: Piped output detection"

# Check the real pipeline statuses. A missing daemon is an expected status
# error, but a broken pipe consumer or any other CLI failure still fails.
set +e
"$RCH" status 2>"$STDERR_FILE" | tee "$STDOUT_FILE" | wc -l >/dev/null
pipeline_status=("${PIPESTATUS[@]}")
set -e
[[ "${pipeline_status[1]}" == 0 && "${pipeline_status[2]}" == 0 ]] \
    || fail "Piped output consumer failed"
if [[ "${pipeline_status[0]}" != 0 ]]; then
    [[ "${pipeline_status[0]}" == 1 && ! -s "$STDOUT_FILE" ]] \
        && grep -Eq '^Error: Daemon socket not found at /' "$STDERR_FILE" \
        || fail "Unexpected status failure in the pipe"
fi

pass "Piped output works correctly"
log ""

# =============================================================================
# TEST 7: Multiple JSON commands produce parseable output
# =============================================================================
log "TEST 7: Multiple JSON commands pipeline"

JSON_PIPELINE_FILE="$(mktemp)"
for command_name in status workers; do
    command_status=0
    if [[ "$command_name" == workers ]]; then
        "$RCH" workers list --json >"$STDOUT_FILE" 2>"$STDERR_FILE" || command_status=$?
    else
        "$RCH" status --json >"$STDOUT_FILE" 2>"$STDERR_FILE" || command_status=$?
    fi
    if [[ "$command_status" == 0 ]]; then
        jq -e '.api_version and .timestamp and .success == true' "$STDOUT_FILE" >/dev/null \
            || fail "$command_name did not emit a success envelope"
    else
        [[ "$command_name" == status && "$command_status" == 1 ]] \
            && jq -e '.success == false and (.error.code | startswith("RCH-E"))' "$STDOUT_FILE" >/dev/null \
            || fail "Unexpected $command_name JSON failure"
    fi
    jq -c . "$STDOUT_FILE" >>"$JSON_PIPELINE_FILE"
done
cat "$JSON_PIPELINE_FILE" | jq -se 'length == 2 and all(.[]; has("api_version") and has("success"))' >/dev/null \
    || fail "Expected two complete JSON envelopes in the pipeline"

pass "Multiple JSON commands can be pipelined"
log ""

# =============================================================================
# TEST 8: Daemon error messages to stderr
# =============================================================================
log "TEST 8: Error messages to stderr"

# An unknown configured-worker request is an error independent of daemon state.
ERROR_EXIT=0
"$RCH" workers probe __rch_e2e_absent__ > "$STDOUT_FILE" 2> "$STDERR_FILE" || ERROR_EXIT=$?

# stdout should be minimal for non-JSON commands
STDOUT_SIZE=$(wc -c < "$STDOUT_FILE")
STDERR_SIZE=$(wc -c < "$STDERR_FILE")

log "  stdout size: $STDOUT_SIZE bytes"
log "  stderr size: $STDERR_SIZE bytes"

if [[ "$ERROR_EXIT" == 1 && "$STDOUT_SIZE" -eq 0 && "$STDERR_SIZE" -gt 0 ]] \
    && grep -q 'RCH-E' "$STDERR_FILE"; then
    pass "Error output separation verified"
else
    fail "Expected error exit 1, an RCH error on stderr, and empty stdout"
fi
log ""

# =============================================================================
# SUMMARY
# =============================================================================
log ""
log "============================================================================="
log "ALL STREAM ISOLATION TESTS PASSED"
log "============================================================================="
log ""
log "Verified:"
log "  1. --json flag outputs clean JSON to stdout"
log "  2. Hook output contains no ANSI codes in stdout"
log "  3. NO_COLOR=1 disables all ANSI codes"
log "  4. All JSON commands produce parseable output"
log "  5. Error messages go to stderr"
log "  6. Piped output works correctly"
log ""

test_pass
