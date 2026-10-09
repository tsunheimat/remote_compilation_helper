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

# Use a known absent socket and fresh state. Earlier scenarios can record local
# fallback incidents, which legitimately add a typed error context to status.
# Keep both native renderings valid without admitting unrelated CLI failures.
PIPE_FIXTURE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rch-stream-pipe-XXXXXX")"
PIPE_FIXTURE_DIR="$(cd "$PIPE_FIXTURE_DIR" && pwd -P)"
mkdir -p "$PIPE_FIXTURE_DIR/config" "$PIPE_FIXTURE_DIR/state" "$PIPE_FIXTURE_DIR/cache" "$PIPE_FIXTURE_DIR/data"
PIPE_SOCKET="$PIPE_FIXTURE_DIR/absent.sock"
PIPE_EXPECTED_ERROR="Daemon socket not found at $PIPE_SOCKET"
PIPE_CAPTURE_PARENT="$(dirname "${RCH_E2E_LOG:-$PROJECT_ROOT/target/test-logs/test_stream_isolation.jsonl}")"
mkdir -p "$PIPE_CAPTURE_PARENT"
PIPE_CAPTURE_DIR="$(mktemp -d "$PIPE_CAPTURE_PARENT/stream-pipe-XXXXXX")"
PIPE_ENV=(env -u RCH_JSON -u RCH_OUTPUT_FORMAT -u TOON_DEFAULT_FORMAT -u FORCE_COLOR -u RCH_HOOK_MODE
    "RCH_CONFIG_DIR=$PIPE_FIXTURE_DIR/config" "RCH_SOCKET_PATH=$PIPE_SOCKET"
    "RCH_STATE_HOME=$PIPE_FIXTURE_DIR/state/rch" "XDG_STATE_HOME=$PIPE_FIXTURE_DIR/state"
    "XDG_CACHE_HOME=$PIPE_FIXTURE_DIR/cache" "XDG_DATA_HOME=$PIPE_FIXTURE_DIR/data")

set +e
"${PIPE_ENV[@]}" "$RCH" status 2>"$PIPE_CAPTURE_DIR/status.stderr" \
    | tee "$PIPE_CAPTURE_DIR/status.stdout" | wc -l >"$PIPE_CAPTURE_DIR/status.lines"
pipeline_status=("${PIPESTATUS[@]}")
set -e
PIPE_JSON_EXIT=0
"${PIPE_ENV[@]}" "$RCH" status --json >"$PIPE_CAPTURE_DIR/status.json" \
    2>"$PIPE_CAPTURE_DIR/status-json.stderr" || PIPE_JSON_EXIT=$?
jq -nc --arg socket "$PIPE_SOCKET" --arg expected "$PIPE_EXPECTED_ERROR" \
    --argjson status "${pipeline_status[0]}" --argjson tee "${pipeline_status[1]}" \
    --argjson wc "${pipeline_status[2]}" --argjson json_exit "$PIPE_JSON_EXIT" \
    '{socket:$socket,expected_error:$expected,pipe_status:[$status,$tee,$wc],json_exit:$json_exit}' \
    >"$PIPE_CAPTURE_DIR/receipt.json"
log_json verify "Piped status captures retained" \
    "$(jq -nc --arg directory "$PIPE_CAPTURE_DIR" '{directory:$directory}')"

[[ "${pipeline_status[0]}" == 1 && "${pipeline_status[1]}" == 0 && "${pipeline_status[2]}" == 0 ]] \
    || fail "Expected missing-daemon exit 1 and successful pipe consumers"
[[ ! -s "$PIPE_CAPTURE_DIR/status.stdout" ]] || fail "Missing-daemon status wrote to stdout"
[[ "$PIPE_JSON_EXIT" == 1 ]] \
    && jq -se --arg expected "$PIPE_EXPECTED_ERROR" \
        'length == 1 and (.[0] | .success == false and .error.details == $expected
            and (.error.code == "RCH-E504" or (.error.code == "RCH-E500"
                and (.error.context.local_build_warning | type == "string" and length > 0))))' \
        "$PIPE_CAPTURE_DIR/status.json" >/dev/null \
    || fail "Piped status did not identify the exact missing fixture socket"
PIPE_EXPECTED_STDERR="Error: $PIPE_EXPECTED_ERROR"
if [[ "$(jq -r '.error.code' "$PIPE_CAPTURE_DIR/status.json")" == RCH-E500 ]]; then
    PIPE_EXPECTED_STDERR="Error: [RCH-E500] Failed to connect to daemon socket: $PIPE_EXPECTED_ERROR: $PIPE_EXPECTED_ERROR"
fi
grep -Fxq "$PIPE_EXPECTED_STDERR" "$PIPE_CAPTURE_DIR/status.stderr" \
    && ! grep -Fq $'\033' "$PIPE_CAPTURE_DIR/status.stderr" \
    || fail "Expected the native plain missing-socket diagnostic on stderr"

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
