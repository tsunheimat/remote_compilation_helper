#!/usr/bin/env bash
#
# e2e_bd-2ga8.sh - True E2E Output Validation & Correctness Tests
#
# Tests that output from remote compilation (stdout, stderr, exit codes,
# terminal colors) is correctly preserved and propagated.
#
# Verifies:
# - Exit code preservation (success 0 and compile error 1)
# - stdout/stderr byte-for-byte correctness
# - ANSI color code preservation
# - Large output without truncation
# - Hook JSON response integrity
#
# Mock SSH supplies worker output; the actual hook rewrite, rch exec pipeline,
# transfer, result propagation, and daemon completion paths still execute.
#
# Usage:
#   ./scripts/e2e_bd-2ga8.sh [--mock] [--verbose]
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="${RCH_E2E_LOG:-$PROJECT_ROOT/target/e2e_bd-2ga8.jsonl}"

# shellcheck source=lib/e2e_common.sh
source "$SCRIPT_DIR/lib/e2e_common.sh"

passed_tests=0
failed_tests=0
run_start_ms="$(e2e_now_ms)"

daemon_pid=""
tmp_root=""
socket_path=""
rch_bin=""
rchd_bin=""
output_prefix=""
last_build_id=0

log_json() {
    local phase="$1"
    local message="$2"
    local test_name="$3"
    local result="$4"
    local data="${5:-}"
    [[ -n "$data" ]] || data='{}'
    local ts
    ts="$(e2e_timestamp)"
    jq -nc --arg ts "$ts" --arg phase "$phase" --arg msg "$message" \
        --arg test_name "$test_name" --arg result "$result" --argjson data "$data" \
        '{ts:$ts,test:"bd-2ga8",phase:$phase,msg:$msg,test_name:$test_name,result:$result,data:$data}' \
        | tee -a "$LOG_FILE"
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
        daemon_pid=""
    fi
    # Retain the fixture and daemon log so a failed scenario remains inspectable.
}
trap cleanup EXIT

check_dependencies() {
    log_json "setup" "Checking dependencies" "dependency_check" "start"
    for cmd in cargo jq cmp; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            log_json "setup" "Missing dependency: $cmd" "dependency_check" "fail" "{\"missing\":\"$cmd\"}"
            record_fail
            return 1
        fi
    done
    log_json "setup" "Dependencies OK" "dependency_check" "pass"
    record_pass
}

build_binaries() {
    local target_dir="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
    rch_bin="$target_dir/debug/rch"
    rchd_bin="$target_dir/debug/rchd"

    if [[ -x "$rch_bin" && -x "$rchd_bin" ]]; then
        log_json "setup" "Using existing binaries" "build" "pass"
        record_pass
        return
    fi

    log_json "setup" "Building rch + rchd" "build" "start"
    if (cd "$PROJECT_ROOT" && cargo build -p rch -p rchd >/dev/null 2>&1) \
        && [[ -x "$rch_bin" && -x "$rchd_bin" ]]; then
        log_json "setup" "Build completed" "build" "pass"
        record_pass
    else
        log_json "setup" "Build failed" "build" "fail"
        record_fail
        return 1
    fi
}

setup_test_env() {
    # Call directly: the caller must retain ownership of its daemon child.
    tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/rch-bd-2ga8-XXXXXX")" || return 1
    tmp_root="$(cd "$tmp_root" && pwd -P)" || return 1
    local workers_toml="$tmp_root/workers.toml"
    local runtime_root
    runtime_root="$(e2e_runtime_dir)" || return 1
    socket_path="$runtime_root/rch.sock"

    # Create mock workers config
    cat > "$workers_toml" <<'WORKERS' || return 1
[[workers]]
id = "mock-1"
host = "127.0.0.1"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 64
WORKERS
    mkdir -p "$tmp_root/config" "$tmp_root/cache" "$tmp_root/state" "$tmp_root/fixture/src" || return 1
    cp "$workers_toml" "$tmp_root/config/workers.toml" || return 1
    cat >"$tmp_root/config/config.toml" <<EOF || return 1
[path_topology]
canonical_root = "$tmp_root"
alias_root = "${tmp_root}__alias"

[output]
visibility = "none"
first_run_complete = true
EOF
    cat >"$tmp_root/fixture/Cargo.toml" <<'EOF' || return 1
[package]
name = "bd_2ga8_output_fixture"
version = "0.1.0"
edition = "2021"
EOF
    printf 'fn main() {}\n' >"$tmp_root/fixture/src/main.rs" || return 1
    export RCH_CONFIG_DIR="$tmp_root/config"
    export RCH_SOCKET_PATH="$socket_path"
    export XDG_CACHE_HOME="$tmp_root/cache"
    export XDG_STATE_HOME="$tmp_root/state"
    export XDG_DATA_HOME="$tmp_root/data"

    log_json "setup" "Starting daemon (mock)" "daemon_start" "start" >&2
    RCH_DAEMON_INSTALLS_HOOKS=0 RCH_LOG_LEVEL=error RCH_TEST_MODE=1 RCH_MOCK_SSH=1 RCH_MOCK_SSH_STDOUT=health_check \
        "$rchd_bin" --socket "$socket_path" --workers-config "$workers_toml" --foreground \
        >"$tmp_root/rchd.log" 2>&1 &
    daemon_pid=$!

    # Wait for socket
    for _ in {1..50}; do
        if [[ -S "$socket_path" ]]; then
            log_json "setup" "Daemon ready" "daemon_start" "pass" "{\"socket\":\"$socket_path\",\"tmp_root\":\"$tmp_root\"}" >&2
            record_pass
            return
        fi
        sleep 0.1
    done

    log_json "setup" "Daemon timeout" "daemon_start" "fail" >&2
    record_fail
    return 1
}

# A fixture result must come from remote execution and a new durable completion,
# never from a usage error, local fallback, or a prior scenario's history entry.
run_output_fixture() {
    local test_name="$1" expected_exit="$2" remote_stdout="$3" remote_stderr="$4"
    shift 4
    output_prefix="$tmp_root/$test_name"
    if ! e2e_run_delegated "$rch_bin" "$tmp_root/fixture" "cargo build" "$output_prefix" \
        "RCH_CONFIG_DIR=$tmp_root/config" "RCH_SOCKET_PATH=$socket_path" \
        RCH_REQUIRE_REMOTE=1 RCH_MOCK_SSH=1 "CARGO_HOME=$tmp_root/cargo-home" \
        "RCH_MOCK_SSH_EXIT_CODE=$expected_exit" "RCH_MOCK_SSH_STDOUT=$remote_stdout" \
        "RCH_MOCK_SSH_STDERR=$remote_stderr" RUST_LOG=off "$@"; then
        log_json "verify" "Hook did not return an executable rewrite" "$test_name" "fail"
        record_fail
        return 1
    fi
    if [[ "$E2E_EXEC_EXIT" -ne "$expected_exit" ]] || ! jq -se --argjson code "$expected_exit" \
        'length == 1 and (.[0] | .location == "remote" and .outcome == "completed" and
         .worker_id == "mock-1" and .command == "cargo build" and .remote_exit_code == $code)' \
        "$output_prefix.exec.json" >/dev/null; then
        log_json "verify" "Expected completed remote result with exact child exit" "$test_name" "fail" \
            "{\"expected\":$expected_exit,\"actual\":$E2E_EXEC_EXIT}"
        record_fail
        return 1
    fi
    if ! "$rch_bin" status --json >"$output_prefix.status.json" 2>"$output_prefix.status.err" \
        || ! jq -e --argjson code "$expected_exit" --argjson previous "$last_build_id" \
        '.success == true and (.data.daemon.active_builds | length) == 0 and
         (.data.daemon.recent_builds[0] | .id > $previous and .exit_code == $code and
          .worker_id == "mock-1" and .command == "cargo build")' \
        "$output_prefix.status.json" >/dev/null; then
        log_json "verify" "Daemon did not record and release this completed build" "$test_name" "fail"
        record_fail
        return 1
    fi
    if ! last_build_id="$(jq -er '.data.daemon.recent_builds[0].id' "$output_prefix.status.json")"; then
        log_json "verify" "Cannot read the completed build identity" "$test_name" "fail"
        record_fail
        return 1
    fi
    # Native machine mode keeps the result envelope on stdout and streams both
    # remote output channels to stderr in the order supplied by MockSshClient.
    if ! printf '%s%s' "$remote_stdout" "$remote_stderr" >"$output_prefix.expected"; then
        log_json "verify" "Cannot retain the expected output bytes" "$test_name" "fail"
        record_fail
        return 1
    fi
}

assert_remote_output() {
    local test_name="$1"
    if cmp -s "$output_prefix.expected" "$output_prefix.exec.err"; then
        log_json "verify" "Remote output preserved byte for byte" "$test_name" "pass"
        record_pass
    else
        log_json "verify" "Remote output differs from the fixture" "$test_name" "fail"
        record_fail
    fi
}

# =============================================================================
# Exit Code Tests
# =============================================================================

test_exit_code_success() {
    local test_name="exit_code_success"
    log_json "execute" "Testing exact remote success exit" "$test_name" "start"
    run_output_fixture "$test_name" 0 $'remote build succeeded\n' "" || return 0
    assert_remote_output "$test_name"
}

test_exit_code_compile_error() {
    local test_name="exit_code_compile_error"
    log_json "execute" "Testing exact remote compile error exit 1" "$test_name" "start"
    run_output_fixture "$test_name" 1 "" $'error: this_function_does_not_exist (remote fixture)\n' || return 0
    assert_remote_output "$test_name"
}

# =============================================================================
# Output Preservation Tests
# =============================================================================

test_stdout_preservation() {
    local test_name="stdout_preservation"
    log_json "execute" "Testing stdout preservation" "$test_name" "start"
    run_output_fixture "$test_name" 0 $'line1_stdout: literal $PATH and `ticks`\nline2_stdout\n' \
        $'line1_stderr\n' || return 0
    assert_remote_output "$test_name"
}

test_stderr_preservation() {
    local test_name="stderr_preservation"
    log_json "execute" "Testing stderr preservation" "$test_name" "start"
    run_output_fixture "$test_name" 0 "" $'warning: unused variable\nsecond diagnostic\n' || return 0
    assert_remote_output "$test_name"
}

# =============================================================================
# Hook Protocol Tests
# =============================================================================

test_hook_json_integrity() {
    local test_name="hook_json_integrity"
    log_json "execute" "Testing hook JSON response integrity" "$test_name" "start"
    run_output_fixture "$test_name" 0 $'actual hook execution\n' "" || return 0
    if jq -se 'length == 1 and (.[0].hookSpecificOutput |
        .hookEventName == "PreToolUse" and .updatedInput.command == "rch exec -- cargo build")' \
        "$output_prefix.hook.json" >/dev/null \
        && cmp -s "$output_prefix.expected" "$output_prefix.exec.err"; then
        log_json "verify" "Single hook JSON rewrite executed and completed" "$test_name" "pass"
        record_pass
    else
        log_json "verify" "Hook response or executed output is invalid" "$test_name" "fail"
        record_fail
    fi
}

test_hook_no_ansi_in_json() {
    local test_name="hook_no_ansi"
    log_json "execute" "Testing no ANSI in hook JSON" "$test_name" "start"
    run_output_fixture "$test_name" 0 $'\033[32mremote output has color\033[0m\n' "" \
        FORCE_COLOR=1 TERM=xterm-256color || return 0
    if jq -se 'length == 1 and (.[0].hookSpecificOutput |
        .hookEventName == "PreToolUse" and .updatedInput.command == "rch exec -- cargo build")' \
        "$output_prefix.hook.json" >/dev/null \
        && ! grep -Fq $'\033' "$output_prefix.hook.json" \
        && ! grep -Fq $'\033' "$output_prefix.exec.json" \
        && cmp -s "$output_prefix.expected" "$output_prefix.exec.err"; then
        log_json "verify" "Machine JSON has no ANSI and remote color remains intact" "$test_name" "pass"
        record_pass
    else
        log_json "verify" "Expected clean machine JSON and unchanged remote output" "$test_name" "fail"
        record_fail
    fi
}

# =============================================================================
# ANSI Color Tests
# =============================================================================

test_ansi_preservation_with_term() {
    local test_name="ansi_preservation"
    log_json "execute" "Testing ANSI color preservation" "$test_name" "start"
    run_output_fixture "$test_name" 0 $'\033[32mFinished\033[0m\n' \
        $'\033[33mwarning\033[0m\n' TERM=xterm-256color CARGO_TERM_COLOR=always || return 0
    assert_remote_output "$test_name"
}

test_no_color_strips_ansi() {
    local test_name="no_color_strips"
    log_json "execute" "Testing NO_COLOR keeps native diagnostics plain" "$test_name" "start"
    local command_exit=0
    # NO_COLOR controls RCH diagnostics. Remote program output is passthrough.
    env -u RCH_JSON NO_COLOR=1 FORCE_COLOR=1 CARGO_TERM_COLOR=never \
        "$rch_bin" workers probe nonexistent-bd-2ga8 \
        >"$tmp_root/$test_name.stdout" 2>"$tmp_root/$test_name.stderr" || command_exit=$?
    if [[ "$command_exit" == 1 && ! -s "$tmp_root/$test_name.stdout" ]] \
        && grep -Fq 'nonexistent-bd-2ga8' "$tmp_root/$test_name.stderr" \
        && grep -Fq 'RCH-E008' "$tmp_root/$test_name.stderr" \
        && ! grep -Fq $'\033' "$tmp_root/$test_name.stderr"; then
        log_json "verify" "NO_COLOR preserved a coded error without ANSI" "$test_name" "pass"
        record_pass
    else
        log_json "verify" "Expected exit 1 and a plain contextual diagnostic on stderr" "$test_name" "fail" \
            "{\"actual_exit\":$command_exit}"
        record_fail
    fi
}

# =============================================================================
# Large Output Tests
# =============================================================================

test_large_output_handling() {
    local test_name="large_output"
    log_json "execute" "Testing large output handling" "$test_name" "start"
    local start_ms line remote_output
    remote_output="$(for line in {0..999}; do
        printf 'Line number %s: This is a test line with some content to make it longer\n' "$line"
    done)"$'\n'
    start_ms="$(e2e_now_ms)"
    run_output_fixture "$test_name" 0 "$remote_output" "" || return 0
    local duration_ms output_bytes output_lines
    duration_ms=$(( $(e2e_now_ms) - start_ms ))
    output_bytes="$(wc -c <"$output_prefix.exec.err" | tr -d ' ')"
    output_lines="$(wc -l <"$output_prefix.exec.err" | tr -d ' ')"
    if [[ "$output_lines" == 1000 && "$output_bytes" -ge 64000 ]] \
        && cmp -s "$output_prefix.expected" "$output_prefix.exec.err"; then
        log_json "verify" "All 1000 remote output lines preserved" "$test_name" "pass" \
            "{\"output_bytes\":$output_bytes,\"duration_ms\":$duration_ms}"
        record_pass
    else
        log_json "verify" "Large output was missing, truncated, or changed" "$test_name" "fail" \
            "{\"output_bytes\":$output_bytes,\"output_lines\":$output_lines}"
        record_fail
    fi
}

# =============================================================================
# Error Display Tests (from bd-vp67)
# =============================================================================

test_config_parse_error_display() {
    local test_name="config_error_display"
    log_json "execute" "Testing config parse error display" "$test_name" "start"
    local bad_config_dir="$tmp_root/bad-config" command_exit=0
    mkdir -p "$bad_config_dir"
    printf 'invalid [ toml syntax\n' >"$bad_config_dir/config.toml"
    env -u RCH_JSON RCH_CONFIG_DIR="$bad_config_dir" NO_COLOR=1 \
        "$rch_bin" config show >"$tmp_root/$test_name.stdout" 2>"$tmp_root/$test_name.stderr" \
        || command_exit=$?
    if [[ "$command_exit" == 1 && ! -s "$tmp_root/$test_name.stdout" ]] \
        && grep -Fq "$bad_config_dir/config.toml" "$tmp_root/$test_name.stderr" \
        && grep -qiE 'line[[:space:]]+[0-9]+' "$tmp_root/$test_name.stderr"; then
        log_json "verify" "Config parse failure identifies its file and line" "$test_name" "pass"
        record_pass
    else
        log_json "verify" "Expected exit 1 with malformed configuration location" "$test_name" "fail" \
            "{\"actual_exit\":$command_exit}"
        record_fail
    fi
}

test_json_error_mode() {
    local test_name="json_error_mode"
    log_json "execute" "Testing --json error output" "$test_name" "start"
    local command_exit=0
    "$rch_bin" workers probe nonexistent-bd-2ga8 --json \
        >"$tmp_root/$test_name.json" 2>"$tmp_root/$test_name.stderr" || command_exit=$?
    if [[ "$command_exit" == 1 && ! -s "$tmp_root/$test_name.stderr" ]] \
        && jq -se 'length == 1 and (.[0] | .api_version == "1.0" and .success == false and
            .data == null and .error.code == "RCH-E008" and .error.category == "config" and
            (.error.details | contains("nonexistent-bd-2ga8")) and
            .error.context.worker_id == "nonexistent-bd-2ga8" and
            (.error.remediation | type == "array" and length > 0))' \
            "$tmp_root/$test_name.json" >/dev/null; then
        log_json "verify" "Exact failure exit and single typed actionable JSON error" "$test_name" "pass"
        record_pass
    else
        log_json "verify" "Expected exit 1 and one typed error envelope" "$test_name" "fail" \
            "{\"actual_exit\":$command_exit}"
        record_fail
    fi
}

# =============================================================================
# Main
# =============================================================================

main() {
    mkdir -p "$(dirname "$LOG_FILE")"
    : > "$LOG_FILE"

    log_json "summary" "Starting E2E output validation tests" "init" "start"

    if ! check_dependencies; then
        return 1
    fi

    if ! build_binaries; then
        return 1
    fi

    if ! setup_test_env; then
        return 1
    fi

    # Run exit code tests
    test_exit_code_success
    test_exit_code_compile_error

    # Run output preservation tests
    test_stdout_preservation
    test_stderr_preservation

    # Run hook protocol tests
    test_hook_json_integrity
    test_hook_no_ansi_in_json

    # Run ANSI/color tests
    test_ansi_preservation_with_term
    test_no_color_strips_ansi

    # Run large output tests
    test_large_output_handling

    # Run error display tests
    test_config_parse_error_display
    test_json_error_mode

    # Summary
    local elapsed_ms
    elapsed_ms=$(( $(e2e_now_ms) - run_start_ms ))
    local total_count
    total_count=$((passed_tests + failed_tests))

    log_json \
        "summary" \
        "bd-2ga8 tests complete (pass=${passed_tests} fail=${failed_tests} total=${total_count})" \
        "final" \
        "$([ "$failed_tests" -eq 0 ] && echo "pass" || echo "fail")" \
        "{\"passed\":$passed_tests,\"failed\":$failed_tests,\"total\":$total_count,\"elapsed_ms\":$elapsed_ms}"

    if [[ "$failed_tests" -gt 0 ]]; then
        return 1
    fi
    return 0
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
