#!/usr/bin/env bash
#
# e2e_bd-szio.sh - Daemon-scheduled worker cache cleanup
#
# Verifies:
# - Cache cleanup scheduler can be configured via daemon.toml
# - Default cleanup config is loaded when no config present
# - Cleanup respects enabled/disabled flag
# - JSONL logging format for test output

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="${RCH_E2E_LOG:-${PROJECT_ROOT}/target/e2e_bd-szio.jsonl}"
# shellcheck source=lib/e2e_common.sh
source "$SCRIPT_DIR/lib/e2e_common.sh"

timestamp() {
    e2e_timestamp
}

log_json() {
    local phase="$1"
    local message="$2"
    local extra="${3:-}"
    if [[ -z "$extra" ]]; then
        extra='{}'
    fi
    local ts
    ts="$(timestamp)"
    jq -nc \
        --arg ts "$ts" \
        --arg test "bd-szio" \
        --arg phase "$phase" \
        --arg message "$message" \
        --argjson extra "$extra" \
        '{ts:$ts,test:$test,phase:$phase,message:$message} + $extra' \
        | tee -a "$LOG_FILE"
}

die() {
    log_json "error" "$*" '{"result":"fail"}'
    exit 1
}

check_dependencies() {
    log_json "setup" "Checking dependencies"
    for cmd in cargo jq python3; do
        command -v "$cmd" >/dev/null 2>&1 || die "Missing dependency: $cmd"
    done
}

build_rchd() {
    local rchd_bin="${PROJECT_ROOT}/target/debug/rchd"
    if [[ -x "$rchd_bin" ]]; then
        log_json "setup" "Using existing rchd binary" "{\"path\":\"$rchd_bin\"}" >/dev/null
        echo "$rchd_bin"
        return
    fi
    log_json "setup" "Building rchd (debug)" >/dev/null
    (cd "$PROJECT_ROOT" && cargo build -p rchd >/dev/null 2>&1) || die "cargo build failed"
    [[ -x "$rchd_bin" ]] || die "rchd binary missing after build"
    echo "$rchd_bin"
}

# Test 1: Verify cache cleanup module compiles and links
test_module_compilation() {
    log_json "test" "Cache cleanup module compiles"
    local rchd_bin
    rchd_bin="$(build_rchd)"
    # If we get here, the module compiled successfully
    log_json "verify" "Module compilation successful" "{\"binary\":\"$rchd_bin\",\"result\":\"pass\"}"
}

# Test 2: Verify cache_cleanup module unit tests pass
test_cache_cleanup_unit_tests() {
    log_json "test" "Cache cleanup unit tests"

    if e2e_cargo_test -p rchd cache_cleanup --no-fail-fast; then
        log_json "verify" "All cache_cleanup unit tests passed" '{"result":"pass"}'
    else
        die "Cache cleanup unit tests failed"
    fi
}

# Test 3: Verify daemon config includes cache_cleanup section
test_daemon_config_includes_cache_cleanup() {
    log_json "test" "DaemonConfig includes cache_cleanup section"

    if e2e_cargo_test -p rchd test_daemon_config_parses_cache_cleanup_section --no-fail-fast; then
        log_json "verify" "Daemon config parses cache_cleanup section" '{"result":"pass"}'
    else
        die "Daemon config parsing failed"
    fi
}

# Test 4: Verify rchd starts with cache cleanup scheduler
test_daemon_startup_with_cleanup() {
    log_json "test" "Daemon starts with cache cleanup scheduler"

    local rchd_bin
    rchd_bin="$(build_rchd)"

    local runtime_dir artifact_dir
    runtime_dir="$(e2e_runtime_dir)"
    artifact_dir="$(mktemp -d "$(dirname "$LOG_FILE")/bd-szio-startup.XXXXXX")"
    if ! python3 - "$rchd_bin" "$runtime_dir" "$artifact_dir" <<'PY'
import os
from pathlib import Path
import subprocess
import sys
import time

binary, root, artifacts = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
config = root / "config"
config.mkdir()
(config / "workers.toml").write_text("workers = []\n")
(config / "daemon.toml").write_text("[cache_cleanup]\nenabled = true\n")
socket = root / "rchd.sock"
env = {key: value for key, value in os.environ.items() if not key.startswith("RCH_")}
env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / "xdg-config"),
           XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"),
           XDG_STATE_HOME=str(root / "state"), RCH_CONFIG_DIR=str(config),
           RCH_STATE_HOME=str(root / "state" / "rch"), RCH_MOCK_SSH="1", NO_COLOR="1")
log_path = artifacts / "daemon.stderr"
stdout_path = artifacts / "daemon.stdout"
with stdout_path.open("wb") as stdout, log_path.open("wb") as stderr:
    process = subprocess.Popen([binary, "--socket", str(socket), "--workers-config",
                                str(config / "workers.toml"), "--metrics-port", "0",
                                "--foreground", "--verbose"],
                               env=env, stdout=stdout, stderr=stderr)
    try:
        deadline = time.monotonic() + 3
        while True:
            if process.poll() is not None:
                raise RuntimeError(f"daemon exited before readiness: {process.returncode}; {log_path}")
            # The daemon's console tracing uses stdout; retain both streams.
            diagnostics = stdout_path.read_bytes() + log_path.read_bytes()
            if socket.is_socket() and b"Cache cleanup scheduler started" in diagnostics:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError(f"scheduler and socket were not ready within 3s; {log_path}")
            time.sleep(0.05)
    finally:
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
print(f"Native scheduler readiness verified; retained artifacts: {artifacts}; runtime: {root}")
PY
    then
        die "Daemon cache cleanup startup failed; retained artifacts: $artifact_dir; runtime: $runtime_dir"
    fi
    log_json "verify" "Daemon startup includes cleanup scheduler" \
        '{"check":"scheduler_and_socket","result":"pass"}'
}

main() {
    : > "$LOG_FILE"
    check_dependencies

    log_json "setup" "Starting bd-szio E2E tests"

    test_module_compilation
    test_cache_cleanup_unit_tests
    test_daemon_config_includes_cache_cleanup
    test_daemon_startup_with_cleanup

    log_json "summary" "All bd-szio checks passed" '{"result":"pass","tests_run":4}'
}

main "$@"
