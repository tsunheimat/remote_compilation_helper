#!/usr/bin/env bash
#
# e2e_output_validation.sh - True E2E Output Validation & Correctness Tests
#
# Tests that output from remote compilation (stdout, stderr, exit codes, terminal
# colors) is correctly preserved and propagated to the calling agent.
#
# Usage:
#   ./scripts/e2e_output_validation.sh [OPTIONS]
#
# Options:
#   --verbose, -v      Enable verbose output
#   --quick            Run subset of quick tests only
#   --help, -h         Show this help message
#
# Exit codes:
#   0 - All tests passed
#   1 - Test failure
#   2 - Setup/dependency error
#
# Test Categories:
#   - Exit code semantics (0, 1, 101, 128+N)
#   - stdout/stderr preservation (byte-for-byte after normalization)
#   - Terminal formatting (ANSI colors, styles)
#   - Streaming behavior (real-time, large output)
#   - Hook protocol integrity (JSON pristine, passthrough)
#   - Error display verification
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="${RCH_E2E_LOG:-$PROJECT_ROOT/target/e2e_output_validation.jsonl}"

# shellcheck source=lib/e2e_common.sh
source "$SCRIPT_DIR/lib/e2e_common.sh"

VERBOSE="${RCH_E2E_VERBOSE:-0}"
QUICK_MODE=0

# Test counters
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0
TESTS_SKIPPED=0

run_start_ms="$(e2e_now_ms)"

# Daemon management
daemon_pid=""
tmp_root=""
socket_path=""

# =============================================================================
# Structured JSON Logging (per bd-2ga8 spec)
# =============================================================================

log_json() {
    local level="$1"
    local test_name="$2"
    local phase="$3"
    local msg="$4"
    local data="${5:-}"
    [[ -n "$data" ]] || data='{}'
    local ts
    ts="$(e2e_timestamp)"

    jq -nc --arg ts "$ts" --arg level "$level" --arg test "$test_name" \
        --arg phase "$phase" --arg msg "$msg" --argjson data "$data" \
        '{ts:$ts,level:$level,test:$test,phase:$phase,msg:$msg,data:$data}' | tee -a "$LOG_FILE"
}

log_info() { log_json "INFO" "$1" "$2" "$3" "${4:-}"; }
log_debug() {
    if [[ "$VERBOSE" == "1" ]]; then
        log_json "DEBUG" "$1" "$2" "$3" "${4:-}"
    fi
}
log_error() { log_json "ERROR" "$1" "$2" "$3" "${4:-}"; }

record_pass() {
    local test_name="$1"
    local msg="${2:-Test passed}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    log_json "INFO" "$test_name" "result" "PASS: $msg" '{"result":"pass"}'
}

record_fail() {
    local test_name="$1"
    local msg="${2:-Test failed}"
    local details="${3:-}"
    TESTS_FAILED=$((TESTS_FAILED + 1))
    log_json "ERROR" "$test_name" "result" "FAIL: $msg" "$details"
}

record_skip() {
    local test_name="$1"
    local msg="${2:-Test skipped}"
    TESTS_SKIPPED=$((TESTS_SKIPPED + 1))
    log_json "INFO" "$test_name" "result" "SKIP: $msg" '{"result":"skip"}'
}

die() {
    log_error "setup" "fatal" "$*"
    exit 2
}

# =============================================================================
# Setup & Teardown
# =============================================================================

usage() {
    sed -n '1,30p' "$0" | sed 's/^# \{0,1\}//'
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --verbose|-v) VERBOSE=1; shift ;;
            --quick) QUICK_MODE=1; shift ;;
            --help|-h) usage; exit 0 ;;
            *) echo "Unknown option: $1" >&2; exit 2 ;;
        esac
    done
}

check_dependencies() {
    log_info "setup" "dependencies" "Checking dependencies"
    local missing=()
    for cmd in cargo jq cmp; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            missing+=("$cmd")
        fi
    done

    if [[ ${#missing[@]} -gt 0 ]]; then
        die "Missing dependencies: ${missing[*]}"
    fi
    log_info "setup" "dependencies" "All dependencies present"
}

build_binaries() {
    local rch_bin="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rch"
    local rchd_bin="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rchd"

    if [[ -x "$rch_bin" && -x "$rchd_bin" ]]; then
        # Log to stderr to avoid polluting return value
        log_info "setup" "build" "Using existing binaries" \
            "{\"rch\":\"$rch_bin\",\"rchd\":\"$rchd_bin\"}" >&2
        echo "$rch_bin;$rchd_bin"
        return
    fi

    log_info "setup" "build" "Building rch + rchd (debug)" >&2
    if ! (cd "$PROJECT_ROOT" && cargo build -p rch -p rchd 2>&1 | tail -5 >&2); then
        die "Build failed"
    fi

    [[ -x "$rch_bin" ]] || die "rch binary missing"
    [[ -x "$rchd_bin" ]] || die "rchd binary missing"

    log_info "setup" "build" "Build complete" >&2
    echo "$rch_bin;$rchd_bin"
}

start_mock_daemon() {
    tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/rch-output-validation-XXXXXX")"
    tmp_root="$(cd "$tmp_root" && pwd -P)"
    local workers_toml="$tmp_root/workers.toml"
    local runtime_root
    runtime_root="$(e2e_runtime_dir)"
    socket_path="$runtime_root/rch.sock"

    cat > "$workers_toml" <<'WORKERS'
[[workers]]
id = "mock-worker"
host = "127.0.0.1"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 64
WORKERS
    mkdir -p "$tmp_root/config" "$tmp_root/cache" "$tmp_root/state"
    cp "$workers_toml" "$tmp_root/config/workers.toml"
    cat >"$tmp_root/config/config.toml" <<EOF
[path_topology]
canonical_root = "$tmp_root"
alias_root = "${tmp_root}__alias"

[output]
visibility = "none"
first_run_complete = true
EOF
    export RCH_CONFIG_DIR="$tmp_root/config"
    export RCH_SOCKET_PATH="$socket_path"
    export XDG_CACHE_HOME="$tmp_root/cache"
    export XDG_STATE_HOME="$tmp_root/state"
    export XDG_DATA_HOME="$tmp_root/data"

    log_info "setup" "daemon" "Starting mock daemon" \
        "{\"socket\":\"$socket_path\",\"config\":\"$workers_toml\"}"

    RCH_DAEMON_INSTALLS_HOOKS=0 RCH_LOG_LEVEL=error RCH_TEST_MODE=1 RCH_MOCK_SSH=1 RCH_MOCK_SSH_STDOUT=health_check \
        "$rchd_bin" --socket "$socket_path" --workers-config "$workers_toml" --foreground \
        >"$tmp_root/rchd.log" 2>&1 &
    daemon_pid=$!

    # Wait for socket
    local waited=0
    while [[ ! -S "$socket_path" && $waited -lt 50 ]]; do
        sleep 0.1
        waited=$((waited + 1))
    done

    if [[ ! -S "$socket_path" ]]; then
        die "Daemon socket not ready after 5s"
    fi

    log_info "setup" "daemon" "Daemon ready" "{\"pid\":$daemon_pid}"
}

cleanup() {
    if [[ -n "$daemon_pid" ]]; then
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# =============================================================================
# Test Fixtures
# =============================================================================

create_test_fixtures() {
    local fixtures_dir="$tmp_root/fixtures"
    mkdir -p "$fixtures_dir"

    # Simple Rust project that compiles
    mkdir -p "$fixtures_dir/simple_project/src"
    cat > "$fixtures_dir/simple_project/Cargo.toml" <<'EOF'
[package]
name = "simple_test"
version = "0.1.0"
edition = "2021"
EOF
    cat > "$fixtures_dir/simple_project/src/main.rs" <<'EOF'
fn main() {
    println!("Hello from simple_test!");
    eprintln!("This goes to stderr");
}
EOF
    CARGO_HOME="$tmp_root/cargo-home" cargo metadata --manifest-path "$fixtures_dir/simple_project/Cargo.toml" \
        --format-version 1 --no-deps >"$tmp_root/fixture-metadata.json" \
        || die "Fixture Cargo metadata preparation failed"

    echo "$fixtures_dir"
}

# The mock supplies the worker response while the real hook, wrapper,
# synchronization, result propagation, and daemon completion paths execute.
# Human-visible remote stdout and stderr both stream to stderr; stdout carries
# the machine result envelope. Keep that existing RCH stream contract explicit.
run_output_fixture() {
    local test_name="$1" expected_exit="$2" remote_stdout="$3" remote_stderr="$4"
    local command="${5:-cargo build}"
    OUTPUT_PREFIX="$tmp_root/$test_name"
    if ! e2e_run_delegated "$rch_bin" "$fixtures_dir/simple_project" "$command" "$OUTPUT_PREFIX" \
        "RCH_CONFIG_DIR=$tmp_root/config" "RCH_SOCKET_PATH=$socket_path" RCH_REQUIRE_REMOTE=1 RCH_MOCK_SSH=1 \
        "CARGO_HOME=$tmp_root/cargo-home" \
        "RCH_MOCK_SSH_EXIT_CODE=$expected_exit" "RCH_MOCK_SSH_STDOUT=$remote_stdout" \
        "RCH_MOCK_SSH_STDERR=$remote_stderr" RUST_LOG=off; then
        record_fail "$test_name" "Hook did not return an executable updatedInput command"
        return 1
    fi
    if [[ "$E2E_EXEC_EXIT" -ne "$expected_exit" ]] || ! jq -e --argjson code "$expected_exit" \
        '.location == "remote" and .outcome == "completed" and
         .worker_id == "mock-worker" and .remote_exit_code == $code' \
        "$OUTPUT_PREFIX.exec.json" >/dev/null; then
        record_fail "$test_name" "Expected completed remote exit $expected_exit, got exit $E2E_EXEC_EXIT"
        return 1
    fi
    if ! "$rch_bin" status --json >"$OUTPUT_PREFIX.status.json" 2>"$OUTPUT_PREFIX.status.err" \
        || ! jq -e --argjson code "$expected_exit" \
        '.success == true and (.data.daemon.active_builds | length) == 0 and
         .data.daemon.recent_builds[0].exit_code == $code' "$OUTPUT_PREFIX.status.json" >/dev/null; then
        record_fail "$test_name" "Daemon did not release and record the completed remote result"
        return 1
    fi
    printf '%s%s' "$remote_stdout" "$remote_stderr" >"$OUTPUT_PREFIX.expected"
}

assert_remote_output() {
    local test_name="$1"
    if cmp -s "$OUTPUT_PREFIX.expected" "$OUTPUT_PREFIX.exec.err"; then
        record_pass "$test_name" "Remote output preserved byte for byte"
    else
        record_fail "$test_name" "Remote output differs from the fixture; see $OUTPUT_PREFIX.exec.err"
    fi
}

# =============================================================================
# Exit Code Tests
# =============================================================================

test_exit_code_success() {
    local test_name="test_exit_code_0"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing exit code 0 (success)"

    run_output_fixture "$test_name" 0 $'remote build succeeded\n' "" || return 0
    assert_remote_output "$test_name"
}

test_exit_code_build_error() {
    local test_name="test_exit_code_1"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing exit code 1 (build error)"

    run_output_fixture "$test_name" 1 "" $'error: mismatched types (remote E2E fixture)\n' || return 0
    assert_remote_output "$test_name"
}

test_exit_code_test_failure() {
    local test_name="test_exit_code_101"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing exit code 101 (test failure)"

    run_output_fixture "$test_name" 101 \
        $'running 2 tests\ntest tests::test_pass ... ok\ntest tests::test_fail ... FAILED\ntest result: FAILED. 1 passed; 1 failed\n' \
        "" "cargo test" || return 0
    assert_remote_output "$test_name"
}

# =============================================================================
# stdout/stderr Preservation Tests
# =============================================================================

test_stdout_preservation() {
    local test_name="test_stdout_preservation"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing stdout preservation"

    run_output_fixture "$test_name" 0 $'remote stdout: literal $PATH and `ticks`\nsecond stdout line\n' "" || return 0
    assert_remote_output "$test_name"
}

test_stderr_preservation() {
    local test_name="test_stderr_preservation"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing stderr preservation"

    run_output_fixture "$test_name" 1 "" $'error: first remote diagnostic\nsecond remote diagnostic\n' || return 0
    assert_remote_output "$test_name"
}

test_multiline_output() {
    local test_name="test_multiline_output"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing multi-line output preservation"

    run_output_fixture "$test_name" 0 $'first\n\nthird\n' $'stderr first\n\nstderr third\n' || return 0
    assert_remote_output "$test_name"
}

# =============================================================================
# Terminal Formatting Tests
# =============================================================================

test_ansi_colors_preserved() {
    local test_name="test_ansi_colors"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing ANSI color preservation"

    run_output_fixture "$test_name" 0 $'\033[32mGreen\033[0m\n\033[31mRed\033[0m\n' "" || return 0
    assert_remote_output "$test_name"
}

test_no_color_strips_ansi() {
    local test_name="test_no_color"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing NO_COLOR strips ANSI"

    # Test NO_COLOR via error output
    local output
    output=$(NO_COLOR=1 "$rch_bin" workers probe nonexistent 2>&1) || true

    # Check for absence of ANSI codes
    local ansi_count=0
    if echo "$output" | grep -qE $'\033\[' 2>/dev/null; then
        ansi_count=$(echo "$output" | grep -oE $'\033\[[0-9;]*m' 2>/dev/null | wc -l || echo 0)
    fi

    log_info "$test_name" "verify" "NO_COLOR check" \
        "{\"ansi_present\":$([ "$ansi_count" -gt 0 ] && echo true || echo false),\"ansi_count\":$ansi_count}"

    if [[ $ansi_count -eq 0 && "$output" == *RCH-E* ]]; then
        record_pass "$test_name" "NO_COLOR properly strips ANSI"
    else
        record_fail "$test_name" "Expected a plain error code with NO_COLOR" \
            "{\"ansi_count\":$ansi_count}"
    fi
}

test_unicode_output() {
    local test_name="test_unicode_output"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing unicode output preservation"

    run_output_fixture "$test_name" 0 $'你好世界\nこんにちは\n🦀🎉✅❌\n∑∏∫√∞\n' "" || return 0
    assert_remote_output "$test_name"
}

# =============================================================================
# Hook Protocol Tests
# =============================================================================

test_hook_json_pristine() {
    local test_name="test_hook_json_pristine"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing hook JSON response integrity"

    # Create hook input
    local hook_input='{"tool_name":"Bash","tool_input":{"command":"cargo build"}}'

    local stdout_file stderr_file
    stdout_file=$(mktemp)
    stderr_file=$(mktemp)

    echo "$hook_input" | RCH_SOCKET_PATH="$socket_path" RCH_TEST_MODE=1 \
        "$rch_bin" >"$stdout_file" 2>"$stderr_file" || true

    local stdout_content
    stdout_content=$(cat "$stdout_file")
    local stderr_bytes
    stderr_bytes=$(wc -c < "$stderr_file")

    # Check if stdout is valid JSON
    local json_valid=false
    local ansi_in_stdout=false

    if echo "$stdout_content" | jq -e '.hookSpecificOutput.updatedInput.command == "rch exec -- cargo build"' >/dev/null 2>&1; then
        json_valid=true
    fi

    if echo "$stdout_content" | grep -qE $'\033\['; then
        ansi_in_stdout=true
    fi

    log_info "$test_name" "verify" "Hook response check" \
        "{\"json_valid\":$json_valid,\"ansi_in_stdout\":$ansi_in_stdout,\"stderr_bytes\":$stderr_bytes}"

    if [[ "$json_valid" == "true" && "$ansi_in_stdout" == "false" ]]; then
        record_pass "$test_name" "Hook JSON response pristine"
    else
        record_fail "$test_name" "Hook did not return a pristine executable rewrite"
    fi
}

# =============================================================================
# Error Display Tests
# =============================================================================

test_error_json_mode() {
    local test_name="test_error_json_mode"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing --json error output"

    # Trigger an error with --json
    local output
    output=$("$rch_bin" workers probe nonexistent-worker --json 2>&1) || true

    local json_valid=false
    local has_error_code=false
    local has_message=false

    if echo "$output" | jq -e '.' >/dev/null 2>&1; then
        json_valid=true
        if echo "$output" | jq -e '.error.code' >/dev/null 2>&1; then
            has_error_code=true
        fi
        if echo "$output" | jq -e '.error.message' >/dev/null 2>&1; then
            has_message=true
        fi
    fi

    local error_code
    error_code=$(echo "$output" | jq -r '.error.code // "none"' 2>/dev/null || echo "none")

    log_info "$test_name" "verify" "JSON error structure" \
        "{\"json_valid\":$json_valid,\"has_code\":$has_error_code,\"has_message\":$has_message,\"error_code\":\"$error_code\"}"

    if [[ "$json_valid" == "true" && "$has_error_code" == "true" ]]; then
        record_pass "$test_name" "JSON error format correct"
    else
        record_fail "$test_name" "JSON error format invalid" \
            "{\"json_valid\":$json_valid,\"has_code\":$has_error_code}"
    fi
}

test_error_stderr_only() {
    local test_name="test_error_stderr_only"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing errors go to stderr (non-JSON mode)"

    local stdout_file stderr_file
    stdout_file=$(mktemp)
    stderr_file=$(mktemp)

    # Trigger error without --json
    "$rch_bin" workers probe nonexistent-worker >"$stdout_file" 2>"$stderr_file" || true

    local stdout_bytes stderr_bytes
    stdout_bytes=$(wc -c < "$stdout_file")
    stderr_bytes=$(wc -c < "$stderr_file")

    log_info "$test_name" "verify" "Stream separation" \
        "{\"stdout_bytes\":$stdout_bytes,\"stderr_bytes\":$stderr_bytes}"

    # Non-JSON errors should go to stderr
    if [[ $stderr_bytes -gt 0 && $stdout_bytes -eq 0 ]] && grep -q 'RCH-E' "$stderr_file"; then
        record_pass "$test_name" "Errors correctly sent to stderr"
    else
        record_fail "$test_name" "Expected an error code on stderr and empty stdout"
    fi
}

test_error_remediation_present() {
    local test_name="test_error_remediation"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing error remediation suggestions"

    local output
    output=$("$rch_bin" workers probe nonexistent-worker --json 2>&1) || true

    local has_remediation=false
    local remediation_count=0

    if echo "$output" | jq -e '.error.remediation' >/dev/null 2>&1; then
        has_remediation=true
        remediation_count=$(echo "$output" | jq '.error.remediation | length' 2>/dev/null || echo 0)
    fi

    log_info "$test_name" "verify" "Remediation check" \
        "{\"has_remediation\":$has_remediation,\"count\":$remediation_count}"

    if [[ "$has_remediation" == "true" && $remediation_count -gt 0 ]]; then
        record_pass "$test_name" "Error has remediation steps ($remediation_count)"
    else
        record_fail "$test_name" "No actionable remediation in error response"
    fi
}

# =============================================================================
# Streaming & Large Output Tests
# =============================================================================

test_large_output_no_truncation() {
    local test_name="test_large_output"

    if [[ "$QUICK_MODE" == "1" ]]; then
        record_skip "$test_name" "Skipped in quick mode"
        return
    fi

    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing large output (no truncation)"

    local output
    output="$(head -c 100000 /dev/zero | tr '\0' 'x')"$'\n'
    run_output_fixture "$test_name" 0 "$output" "" || return 0
    assert_remote_output "$test_name"
}

test_streaming_first_byte_latency() {
    local test_name="test_streaming_latency"

    if [[ "$QUICK_MODE" == "1" ]]; then
        record_skip "$test_name" "Skipped in quick mode"
        return
    fi

    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing first-byte streaming through local fallback"

    # The in-process mock delivers remote output after its result is ready. It
    # cannot establish live SSH streaming. Exercise an actual child of the
    # delegated wrapper here and require its first byte before it terminates.
    local stream_bin="$tmp_root/stream-bin" prefix="$tmp_root/$test_name"
    mkdir -p "$stream_bin"
    cat >"$stream_bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf 'RCH_E2E_FIRST_BYTE\n' >&2
sleep 2
printf 'RCH_E2E_SECOND_BYTE\n' >&2
EOF
    chmod +x "$stream_bin/cargo"
    local start_ms first_byte_ms helper_pid first_while_running=false
    start_ms="$(e2e_now_ms)"
    e2e_run_delegated "$rch_bin" "$fixtures_dir/simple_project" "cargo build" "$prefix" \
        "PATH=$stream_bin:${rch_bin%/*}:$PATH" "RCH_SOCKET_PATH=$socket_path" \
        "CARGO_HOME=$tmp_root/cargo-home" \
        "RCH_CONFIG_DIR=$tmp_root/config" \
        RCH_REQUIRE_REMOTE=0 RCH_MOCK_SSH=1 RCH_MOCK_CIRCUIT_OPEN=1 RUST_LOG=off &
    helper_pid=$!
    while (( $(e2e_now_ms) - start_ms < 5000 )); do
        if [[ -f "$prefix.exec.err" ]] && grep -q 'RCH_E2E_FIRST_BYTE' "$prefix.exec.err"; then
            if kill -0 "$helper_pid" 2>/dev/null; then
                first_while_running=true
            fi
            break
        fi
        kill -0 "$helper_pid" 2>/dev/null || break
        sleep 0.02
    done
    first_byte_ms=$(( $(e2e_now_ms) - start_ms ))
    local helper_exit=0
    wait "$helper_pid" || helper_exit=$?
    log_info "$test_name" "timing" "Local fallback first byte" \
        "{\"first_byte_ms\":$first_byte_ms,\"threshold_ms\":5000,\"before_completion\":$first_while_running}"
    if [[ "$helper_exit" == 0 && "$first_while_running" == true && "$first_byte_ms" -lt 5000 ]] \
        && grep -q 'RCH_E2E_SECOND_BYTE' "$prefix.exec.err" \
        && jq -e '.location == "local" and .outcome == "completed" and .remote_exit_code == 0' \
            "$prefix.exec.json" >/dev/null; then
        record_pass "$test_name" "Local fallback delivered its first byte before the child completed"
    else
        record_fail "$test_name" "First-byte streaming or completed local result was not observed"
    fi
}

# =============================================================================
# Main Test Runner
# =============================================================================

run_all_tests() {
    local fixtures_dir="$1"

    log_info "suite" "start" "Starting output validation tests" \
        "{\"quick_mode\":$([ "$QUICK_MODE" == "1" ] && echo true || echo false)}"

    # Exit Code Tests
    test_exit_code_success
    test_exit_code_build_error "$fixtures_dir"
    test_exit_code_test_failure "$fixtures_dir"

    # stdout/stderr Preservation Tests
    test_stdout_preservation
    test_stderr_preservation "$fixtures_dir"
    test_multiline_output

    # Terminal Formatting Tests
    test_ansi_colors_preserved
    test_no_color_strips_ansi
    test_unicode_output

    # Hook Protocol Tests
    test_hook_json_pristine

    # Error Display Tests
    test_error_json_mode
    test_error_stderr_only
    test_error_remediation_present

    # Streaming & Large Output Tests
    test_large_output_no_truncation
    test_streaming_first_byte_latency
}

print_summary() {
    local elapsed_ms=$(( $(e2e_now_ms) - run_start_ms ))
    local elapsed_s
    elapsed_s=$(awk "BEGIN { printf \"%.2f\", ${elapsed_ms}/1000 }")

    log_info "suite" "summary" "Test suite complete" \
        "{\"total\":$TESTS_RUN,\"passed\":$TESTS_PASSED,\"failed\":$TESTS_FAILED,\"skipped\":$TESTS_SKIPPED,\"duration_ms\":$elapsed_ms}"

    echo ""
    echo "=========================================="
    echo " Output Validation E2E Test Summary"
    echo "=========================================="
    echo " Total:   $TESTS_RUN"
    echo " Passed:  $TESTS_PASSED"
    echo " Failed:  $TESTS_FAILED"
    echo " Skipped: $TESTS_SKIPPED"
    echo " Time:    ${elapsed_s}s"
    echo " Log:     $LOG_FILE"
    echo "=========================================="

    if [[ $TESTS_FAILED -gt 0 ]]; then
        return 1
    fi
    return 0
}

main() {
    parse_args "$@"

    mkdir -p "$(dirname "$LOG_FILE")"
    : > "$LOG_FILE"

    check_dependencies

    local bins
    bins="$(build_binaries)"
    rch_bin="${bins%;*}"
    rchd_bin="${bins#*;}"

    start_mock_daemon

    local fixtures_dir
    fixtures_dir="$(create_test_fixtures)"

    run_all_tests "$fixtures_dir"
    print_summary
}

main "$@"
