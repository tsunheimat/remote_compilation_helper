#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# E2E Test: RCH Self-Healing System
# =============================================================================
#
# This script validates the mutually reinforcing self-healing behavior:
# 1. Hook delegates to rch exec, which recovers an unavailable daemon
# 2. Daemon auto-installs hooks on startup
# 3. Doctor --fix repairs both
#
# Prerequisites:
# - rch/rchd pair in CARGO_TARGET_DIR, repository target, or PATH (RCH_BIN/RCHD_BIN override)
# - jq, Python 3, nohup and ps
# - Write access to /tmp/ (all HOME/config/runtime state is isolated)
# - macOS: no registered com.rch.daemon launchd service
#
# Usage:
#   ./tests/e2e/test_self_healing.sh [--verbose]
#

# =============================================================================
# Configuration
# =============================================================================

TEST_ROOT=$(mktemp -d)
TEST_ROOT=$(cd "$TEST_ROOT" && pwd -P)
TEST_RUNTIME_ROOT=$(mktemp -d /tmp/rch-healing.XXXXXX)
TEST_RUNTIME_ROOT=$(cd "$TEST_RUNTIME_ROOT" && pwd -P)
LOG_FILE="${TEST_ROOT}/test.log"
TEST_PROJECT_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
TEST_TARGET_DIR="${CARGO_TARGET_DIR:-$TEST_PROJECT_ROOT/target}"
TEST_BIN_DIR=""
for profile in debug release; do
    if [[ -x "$TEST_TARGET_DIR/$profile/rch" && -x "$TEST_TARGET_DIR/$profile/rchd" ]]; then
        TEST_BIN_DIR="$TEST_TARGET_DIR/$profile"
        break
    fi
done
TEST_RCH_BIN="${RCH_BIN:-${TEST_BIN_DIR:+$TEST_BIN_DIR/rch}}"
TEST_RCH_BIN="${TEST_RCH_BIN:-$(command -v rch || true)}"
TEST_RCHD_BIN="${RCHD_BIN:-$(dirname "$TEST_RCH_BIN")/rchd}"
TEST_NOHUP_BIN=$(command -v nohup || true)
TEST_PYTHON_BIN="${RCH_E2E_PYTHON_BIN:-$(command -v python3 || true)}"
TEST_CARGO_HOME="${CARGO_HOME:-${HOME}/.cargo}"
TEST_RUSTUP_HOME="${RUSTUP_HOME:-${HOME}/.rustup}"

VERBOSE="${1:-}"
PASSED=0
FAILED=0
TOTAL=0

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# =============================================================================
# Logging Functions
# =============================================================================

log() {
    local level=$1
    shift
    local timestamp
    timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo -e "[$timestamp] [$level] $*" | tee -a "$LOG_FILE"
}

log_info()  { log "INFO " "$*"; }
log_pass()  { log "${GREEN}PASS${NC} " "$*"; }
log_fail()  { log "${RED}FAIL${NC} " "$*"; }
log_test()  { log "${BLUE}TEST${NC} " "$*"; }
log_detail() { [[ "$VERBOSE" == "--verbose" ]] && log "DEBUG" "$*" || true; }

# =============================================================================
# Test Utilities
# =============================================================================

setup_test_env() {
    local prerequisite
    for prerequisite in jq ps; do
        command -v "$prerequisite" >/dev/null || {
            log_fail "Missing prerequisite: $prerequisite"
            return 2
        }
    done
    [[ -x "$TEST_RCH_BIN" && -x "$TEST_RCHD_BIN" && -x "$TEST_NOHUP_BIN" && -x "$TEST_PYTHON_BIN" ]] || {
        log_fail "Need executable rch, rchd, nohup and python3; fixtures: $TEST_ROOT"
        return 2
    }
    TEST_RCH_BIN=$(cd "$(dirname "$TEST_RCH_BIN")" && pwd -P)/$(basename "$TEST_RCH_BIN")
    TEST_RCHD_BIN=$(cd "$(dirname "$TEST_RCHD_BIN")" && pwd -P)/$(basename "$TEST_RCHD_BIN")
    # rch's recovery code prefers its sibling daemon over a PATH override.
    [[ "$TEST_RCHD_BIN" == "$(dirname "$TEST_RCH_BIN")/rchd" ]] || {
        log_fail "Use an rch/rchd pair from the same directory"
        return 2
    }
    if [[ $(uname -s) == Darwin ]]; then
        local services
        services=$(/bin/launchctl list) || return 2
        if [[ "$services" == *com.rch.daemon* ]]; then
            log_fail "Registered com.rch.daemon prevents an isolated standalone test"
            return 2
        fi
    fi
    log_info "Fixtures and logs are retained at $TEST_ROOT"
    log_info "Short socket and cooldown paths are retained at $TEST_RUNTIME_ROOT"
    log_info "Using $TEST_RCH_BIN and $TEST_RCHD_BIN"
}

setup_case() {
    TEST_HOME="$CASE_ROOT/home"
    TEST_CONFIG_DIR="$TEST_HOME/.config/rch"
    TEST_CLAUDE_DIR="$TEST_HOME/.claude"
    TEST_RUNTIME_DIR="$TEST_RUNTIME_ROOT/case-$TOTAL"
    TEST_SOCKET="$TEST_RUNTIME_DIR/rch.sock"
    TEST_PROJECT="$CASE_ROOT/project"
    TEST_LAUNCHES="$CASE_ROOT/daemon-pids"
    TEST_COOLDOWN="$TEST_RUNTIME_DIR/rch/hook_autostart.cooldown"
    HOOK_ATTEMPT=0
    EXEC_ATTEMPT=0
    # Keep pathname sockets within sockaddr_un's platform limit even when
    # inherited TMPDIR points at a long external build-storage path.
    [[ ${#TEST_SOCKET} -lt 100 ]] || { log_fail "Fixture socket path is too long: $TEST_SOCKET"; return 2; }
    mkdir -p "$TEST_CONFIG_DIR" "$TEST_CLAUDE_DIR" "$TEST_RUNTIME_DIR" "$CASE_ROOT/bin" "$TEST_PROJECT/src"
    cat > "$CASE_ROOT/bin/nohup" <<'EOF'
#!/bin/sh
set -eu
[ "$1" = "$RCH_TEST_DAEMON_BIN" ] || { echo "unexpected nohup command" >&2; exit 2; }
printf '%s\n' "$$" >> "$RCH_TEST_DAEMON_PIDS"
exec "$RCH_TEST_NOHUP_BIN" "$@"
EOF
    chmod +x "$CASE_ROOT/bin/nohup"
    cat > "$TEST_PROJECT/Cargo.toml" <<'EOF'
[package]
name = "self_healing_fixture"
version = "0.1.0"
edition = "2021"
EOF
    printf 'fn main() {}\n' > "$TEST_PROJECT/src/main.rs"
    printf 'workers = []\n' > "$TEST_CONFIG_DIR/workers.toml"
    CASE_ENV=(env)
    local variable
    while IFS= read -r variable; do
        case "$variable" in
            RCH_*|TOON_DEFAULT_FORMAT|RUST_LOG|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|NOTIFY_SOCKET)
                CASE_ENV+=(-u "$variable") ;;
        esac
    done < <(compgen -e)
    CASE_ENV+=(
        "HOME=$TEST_HOME" "RCH_CONFIG_DIR=$TEST_CONFIG_DIR"
        "XDG_CONFIG_HOME=$TEST_HOME/.config" "XDG_CACHE_HOME=$TEST_HOME/.cache"
        "XDG_DATA_HOME=$TEST_HOME/.local/share" "XDG_STATE_HOME=$TEST_HOME/.local/state"
        "XDG_RUNTIME_DIR=$TEST_RUNTIME_DIR"
        "DBUS_SESSION_BUS_ADDRESS=unix:path=$TEST_RUNTIME_DIR/no-session-bus"
        "CARGO_HOME=$TEST_CARGO_HOME" "RUSTUP_HOME=$TEST_RUSTUP_HOME"
        "CARGO_TARGET_DIR=$TEST_PROJECT/target" "NO_COLOR=1"
        "PATH=$CASE_ROOT/bin:$(dirname "$TEST_RCH_BIN"):$PATH"
        "RCH_REQUIRE_REMOTE=1" "RCH_TEST_DAEMON_BIN=$TEST_RCHD_BIN"
        "RCH_TEST_DAEMON_PIDS=$TEST_LAUNCHES" "RCH_TEST_NOHUP_BIN=$TEST_NOHUP_BIN"
    )
    write_case_config true true 30
}

write_case_config() {
    cat > "$TEST_CONFIG_DIR/config.toml" <<EOF
[general]
enabled = true
force_local = false
socket_path = "$TEST_SOCKET"

[path_topology]
canonical_root = "$CASE_ROOT"
alias_root = "${CASE_ROOT}__alias_sentinel"

[self_healing]
hook_starts_daemon = $1
daemon_installs_hooks = $2
auto_start_timeout_secs = 5
auto_start_cooldown_secs = $3
EOF
}

bounded_run() {
    "$TEST_PYTHON_BIN" -c '
import subprocess, sys
try:
    sys.exit(subprocess.run(sys.argv[2:], timeout=int(sys.argv[1])).returncode)
except subprocess.TimeoutExpired:
    print("self-healing command exceeded " + sys.argv[1] + " seconds", file=sys.stderr)
    sys.exit(124)
' "$@"
}

case_rch_for() {
    local budget=$1
    shift
    (cd "$TEST_PROJECT" && bounded_run "$budget" "${CASE_ENV[@]}" "$TEST_RCH_BIN" "$@")
}

case_rch() {
    case_rch_for 45 "$@"
}

daemon_pid() {
    local report
    report=$(mktemp "$CASE_ROOT/status.XXXXXX")
    case_rch --json status > "$report" 2> "$report.stderr" || return 1
    jq -ers --arg socket "$TEST_SOCKET" '
        select(length == 1) | .[0]
        | select(.success and .data.daemon.daemon.socket_path == $socket)
        | .data.daemon.daemon.pid | select(type == "number" and . > 1)
    ' "$report"
}

assert_owned_daemon() {
    local pid launched
    pid=$(daemon_pid) || return 1
    [[ -f "$TEST_LAUNCHES" ]] || return 1
    while IFS= read -r launched; do
        [[ "$pid" == "$launched" ]] && return 0
    done < "$TEST_LAUNCHES"
    log_fail "Serving PID $pid was not a child launched by this fixture"
    return 1
}

owned_daemon_alive() {
    local command
    command=$(ps -ww -p "$1" -o command= 2>/dev/null) || return 1
    [[ "$command" == *"$TEST_RCHD_BIN"* && "$command" == *"$TEST_SOCKET"* ]]
}

stop_case_daemons() {
    [[ -f "$TEST_LAUNCHES" ]] || return 0
    local pid tick
    while IFS= read -r pid; do
        [[ "$pid" =~ ^[0-9]+$ && "$pid" -gt 1 ]] || return 1
        if owned_daemon_alive "$pid"; then
            kill -TERM "$pid" || return 1
            for ((tick = 0; tick < 100; tick++)); do
                owned_daemon_alive "$pid" || break
                sleep 0.1
            done
            if owned_daemon_alive "$pid"; then
                log_fail "Owned daemon $pid did not stop; retained at $CASE_ROOT"
                return 1
            fi
        fi
    done < "$TEST_LAUNCHES"
}

finish_case() {
    local status=$?
    stop_case_daemons || status=1
    exit "$status"
}

assert_no_live_daemon() {
    local report
    report=$(mktemp "$CASE_ROOT/daemon-status.XXXXXX")
    case_rch --json daemon status > "$report"
    jq -es --arg socket "$TEST_SOCKET" '
        length == 1 and (.[0] | .success and (.data.running == false) and .data.socket_path == $socket)
    ' "$report" >/dev/null
}

start_case_daemon() {
    case_rch --json daemon start > "$CASE_ROOT/daemon-start.json"
    jq -es 'length == 1 and (.[0] | .success and .data.success)' "$CASE_ROOT/daemon-start.json" >/dev/null
    assert_owned_daemon
}

invoke_hook() {
    HOOK_ATTEMPT=$((HOOK_ATTEMPT + 1))
    local prefix="$CASE_ROOT/hook-$HOOK_ATTEMPT"
    jq -n '{tool_name:"Bash",tool_input:{command:"cargo check"},session_id:"isolated-self-healing"}' > "$prefix.input.json"
    case_rch_for "${1:-30}" < "$prefix.input.json" > "$prefix.output.json" 2> "$prefix.stderr"
    DELEGATED_COMMAND=$(jq -ers '
        select(length == 1) | .[0].hookSpecificOutput.updatedInput.command
        | select(. == "rch exec -- cargo check")
    ' "$prefix.output.json")
}

execute_delegated() {
    EXEC_ATTEMPT=$((EXEC_ATTEMPT + 1))
    local prefix="$CASE_ROOT/exec-$EXEC_ATTEMPT"
    local status=0
    (cd "$TEST_PROJECT" && bounded_run 30 "${CASE_ENV[@]}" bash -c "$DELEGATED_COMMAND") \
        > "$prefix.stdout" 2> "$prefix.stderr" || status=$?
    printf '%s\n' "$status" > "$prefix.status"
    # The empty fleet must refuse remote-required work, never compile locally.
    assert_eq 103 "$status" "Empty fleet should return the retryable remote-required refusal"
    [[ ! -d "$TEST_PROJECT/target" ]]
}

assert_rch_hook() {
    jq -e --arg executable "$TEST_RCH_BIN" '
        [.hooks.PreToolUse[] | .hooks[]?
            | select(.type == "command" and (.command == "rch" or .command == $executable))]
        | length == 1
    ' "$TEST_CLAUDE_DIR/settings.json" >/dev/null
}

run_case_doctor() {
    case_rch --json doctor --fix "$@" > "$CASE_ROOT/doctor.json" 2> "$CASE_ROOT/doctor.stderr"
    jq -es 'length == 1 and (.[0] | .success and .command == "doctor")' "$CASE_ROOT/doctor.json" >/dev/null
}

assert_eq() {
    local expected=$1
    local actual=$2
    local msg=$3

    if [[ "$expected" == "$actual" ]]; then
        return 0
    else
        log_fail "Assertion failed: $msg"
        log_detail "  Expected: $expected"
        log_detail "  Actual:   $actual"
        return 1
    fi
}

run_test() {
    local name=$1
    shift
    local test_fn=$1

    TOTAL=$((TOTAL + 1))
    log_test "Running: $name"
    CASE_ROOT="$TEST_ROOT/case-$TOTAL"
    mkdir -p "$CASE_ROOT"
    # A test in an `if` condition disables Bash errexit inside its functions.
    # Run each case as an ordinary subshell so failed assertions stay fatal.
    set +e
    (
        set -euo pipefail
        setup_case
        trap finish_case EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM
        "$test_fn"
    ) > "$CASE_ROOT/stdout.log" 2> "$CASE_ROOT/stderr.log"
    local status=$?
    set -e
    if [[ "$status" -eq 0 ]]; then
        PASSED=$((PASSED + 1))
        log_pass "$name"
    else
        FAILED=$((FAILED + 1))
        log_fail "$name"
    fi
    if [[ "$status" -ne 0 || "$VERBOSE" == "--verbose" ]]; then
        cat "$CASE_ROOT/stdout.log" "$CASE_ROOT/stderr.log"
    fi
}

# =============================================================================
# Test Cases
# =============================================================================

test_delegated_exec_auto_starts_daemon() {
    assert_no_live_daemon
    invoke_hook
    [[ ! -e "$TEST_LAUNCHES" && ! -S "$TEST_SOCKET" ]]
    execute_delegated
    assert_owned_daemon
    [[ $(wc -l < "$TEST_LAUNCHES" | tr -d ' ') == 1 ]]
    [[ $(cat "$TEST_COOLDOWN") =~ ^[0-9]+$ ]]
}

test_daemon_auto_installs_hook() {
    [[ ! -f "$TEST_CLAUDE_DIR/settings.json" ]]
    start_case_daemon
    assert_rch_hook
}

test_daemon_preserves_existing_hooks() {
    cat > "$TEST_CLAUDE_DIR/settings.json" <<'EOF'
{
  "permissions": {"allow": ["Read"]},
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{"type": "command", "command": "dcg", "timeout": 10}]
      }
    ]
  }
}
EOF
    start_case_daemon
    assert_rch_hook
    jq -e '
        .permissions == {"allow":["Read"]}
        and ([.hooks.PreToolUse[] | .hooks[]? | select(.command == "dcg")]
            == [{"type":"command","command":"dcg","timeout":10}])
    ' "$TEST_CLAUDE_DIR/settings.json" >/dev/null
}

test_doctor_fix_installs_hook() {
    # Attribute installation to doctor, independently of daemon hook repair.
    write_case_config true false 30
    run_case_doctor
    assert_rch_hook
    jq -e '
        [.data.checks[] | select(.name == "claude_code_hook")]
        | length == 1 and .[0].fix_applied and .[0].status == "pass"
    ' "$CASE_ROOT/doctor.json" >/dev/null
}

test_doctor_fix_starts_daemon() {
    run_case_doctor
    assert_owned_daemon
    jq -e '
        [.data.checks[] | select(.name == "daemon_socket")]
        | length == 1 and .[0].fix_applied and .[0].status == "pass"
    ' "$CASE_ROOT/doctor.json" >/dev/null
}

test_doctor_dry_run_no_changes() {
    run_case_doctor --dry-run
    [[ ! -f "$TEST_CLAUDE_DIR/settings.json" && ! -e "$TEST_LAUNCHES" && ! -e "$TEST_SOCKET" ]]
    jq -e '
        [.data.checks[] | select(.name == "claude_code_hook" or .name == "daemon_socket")]
        | length == 2 and all(.fix_applied == false and (.fix_message | startswith("Would ")))
    ' "$CASE_ROOT/doctor.json" >/dev/null
    assert_no_live_daemon
}

test_auto_start_cooldown() {
    write_case_config true true 300
    invoke_hook 10
    execute_delegated
    assert_owned_daemon
    local timestamp
    timestamp=$(cat "$TEST_COOLDOWN")
    [[ "$timestamp" =~ ^[0-9]+$ ]]
    printf '%s\n' "$timestamp" > "$CASE_ROOT/cooldown-first.timestamp"
    [[ $(wc -l < "$TEST_LAUNCHES" | tr -d ' ') == 1 ]]
    stop_case_daemons
    assert_no_live_daemon

    # Cooldown still observes readiness for the configured startup budget.
    # Its contract is no new launch, not an immediate return.
    invoke_hook 5
    execute_delegated
    assert_no_live_daemon
    assert_eq "$timestamp" "$(cat "$TEST_COOLDOWN")" "Cooldown must not record a second launch"
    [[ $(wc -l < "$TEST_LAUNCHES" | tr -d ' ') == 1 ]]
}

test_config_disables_self_healing() {
    write_case_config false false 30
    invoke_hook
    execute_delegated
    assert_no_live_daemon
    [[ ! -e "$TEST_LAUNCHES" && ! -f "$TEST_CLAUDE_DIR/settings.json" ]]
    # Disabling recovery does not prohibit an explicit operator startup.
    start_case_daemon
    [[ ! -f "$TEST_CLAUDE_DIR/settings.json" ]]
}

test_full_self_healing_cycle() {
    run_case_doctor
    assert_rch_hook
    assert_owned_daemon
    local original_pid original_settings tick
    original_pid=$(daemon_pid)
    original_settings=$(jq -cS . "$TEST_CLAUDE_DIR/settings.json")
    owned_daemon_alive "$original_pid"
    # Kill only the daemon whose PID and configured socket were verified above.
    kill -KILL "$original_pid"
    for ((tick = 0; tick < 100; tick++)); do
        owned_daemon_alive "$original_pid" || break
        sleep 0.1
    done
    assert_no_live_daemon
    invoke_hook
    execute_delegated
    assert_owned_daemon
    [[ $(daemon_pid) != "$original_pid" ]]
    [[ $(wc -l < "$TEST_LAUNCHES" | tr -d ' ') == 2 ]]
    assert_eq "$original_settings" "$(jq -cS . "$TEST_CLAUDE_DIR/settings.json")" "Recovery must preserve installed hook settings"
}

# =============================================================================
# Main
# =============================================================================

main() {
    echo ""
    echo "=============================================="
    echo "  RCH Self-Healing E2E Test Suite"
    echo "=============================================="
    echo ""

    setup_test_env

    # Run all tests
    run_test "Hook delegates and exec auto-starts daemon" test_delegated_exec_auto_starts_daemon
    run_test "Daemon auto-installs hook" test_daemon_auto_installs_hook
    run_test "Daemon preserves existing hooks" test_daemon_preserves_existing_hooks
    run_test "Doctor --fix installs hook" test_doctor_fix_installs_hook
    run_test "Doctor --fix starts daemon" test_doctor_fix_starts_daemon
    run_test "Doctor --dry-run makes no changes" test_doctor_dry_run_no_changes
    run_test "Auto-start cooldown works" test_auto_start_cooldown
    run_test "Config disables self-healing" test_config_disables_self_healing
    run_test "Full self-healing cycle" test_full_self_healing_cycle

    # Summary
    echo ""
    echo "=============================================="
    echo "  Test Results"
    echo "=============================================="
    echo ""
    echo -e "  Total:  $TOTAL"
    echo -e "  ${GREEN}Passed: $PASSED${NC}"
    echo -e "  ${RED}Failed: $FAILED${NC}"
    echo ""

    if [[ $FAILED -gt 0 ]]; then
        echo -e "${RED}SOME TESTS FAILED${NC}"
        echo "See $LOG_FILE and $TEST_ROOT/case-* for retained evidence"
        exit 1
    else
        echo -e "${GREEN}ALL TESTS PASSED${NC}"
        exit 0
    fi
}

main "$@"
