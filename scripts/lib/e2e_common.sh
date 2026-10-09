#!/usr/bin/env bash
#
# Shared helpers for RCH E2E shell scripts.
# Intentionally does not set shell options; callers control strictness.
#

E2E_SKIP_EXIT=4

# Unix socket names are bounded even when the caller's TMPDIR is a long NFS
# path. Keep only sockets/daemon runtime here; logs and fixture data retain the
# caller-selected location. Runtime directories are retained for inspection.
e2e_runtime_dir() {
    mktemp -d /tmp/rch-e2e-runtime-XXXXXX
}

e2e_timestamp() {
    local timestamp
    timestamp="$(date -u '+%Y-%m-%dT%H:%M:%S.%3NZ' 2>/dev/null)" || timestamp=""
    if [[ -n "$timestamp" && "$timestamp" != *N* ]]; then
        printf '%s\n' "$timestamp"
    else
        date -u '+%Y-%m-%dT%H:%M:%SZ'
    fi
}

e2e_now_ms() {
    local milliseconds
    milliseconds="$(date +%s%3N 2>/dev/null)" || milliseconds=""
    # BSD date can succeed while leaving an unsupported %N in the output.
    if [[ "$milliseconds" =~ ^[0-9]+$ ]]; then
        printf '%s\n' "$milliseconds"
        return
    fi
    if command -v python3 >/dev/null 2>&1; then
        python3 -c 'import time; print(time.time_ns() // 1_000_000)'
        return
    fi
    local seconds
    seconds="$(date +%s)"
    printf '%s000' "$seconds"
}

e2e_log() {
    printf '[E2E] %s\n' "$*"
}

e2e_default_parallelism() {
    if command -v nproc >/dev/null 2>&1; then
        nproc
        return
    fi
    if command -v sysctl >/dev/null 2>&1; then
        sysctl -n hw.ncpu
        return
    fi
    echo 4
}

e2e_slug() {
    echo "$1" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9._-]/_/g'
}

e2e_xml_escape() {
    local value="$1"
    value="${value//&/&amp;}"
    value="${value//</&lt;}"
    value="${value//>/&gt;}"
    value="${value//\"/&quot;}"
    value="${value//\'/&apos;}"
    printf '%s' "$value"
}

# Exercise the same two phases as the hook consumer: obtain updatedInput, then
# execute its command. The child exit is data, not this helper's return status;
# callers must assert E2E_EXEC_EXIT and the execution envelope separately.
e2e_run_delegated() {
    local rch_bin="$1" project_dir="$2" command="$3" log_prefix="$4"
    shift 4
    local hook_input delegated variable
    local -a fixture_env=(env)
    # Operator overrides (requested workers, queue policy, output format, etc.)
    # are not inputs to these fixtures. Only explicit per-scenario RCH values
    # below may affect admission or formatting.
    while IFS= read -r variable; do
        case "$variable" in
            RCH_*|TOON_DEFAULT_FORMAT|CARGO_TARGET_DIR) fixture_env+=(-u "$variable") ;;
        esac
    done < <(compgen -e)
    hook_input="$(jq -nc --arg command "$command" \
        '{tool_name:"Bash",tool_input:{command:$command,description:"RCH E2E fixture"}}')" || return 1
    (
        cd "$project_dir" || exit 1
        printf '%s\n' "$hook_input" | "${fixture_env[@]}" "RCH_STATE_HOME=${log_prefix%/*}/state/rch" "$@" "$rch_bin" \
            >"$log_prefix.hook.json" 2>"$log_prefix.hook.err"
    ) || return 1
    delegated="$(jq -er '.hookSpecificOutput.updatedInput.command | strings' \
        "$log_prefix.hook.json")" || return 1
    [[ "$delegated" == "rch exec -- "* ]] || return 1
    E2E_EXEC_EXIT=0
    (
        cd "$project_dir" || exit 1
        "${fixture_env[@]}" PATH="${rch_bin%/*}:$PATH" RCH_JSON=1 "RCH_STATE_HOME=${log_prefix%/*}/state/rch" "$@" bash -c "$delegated" \
            >"$log_prefix.exec.json" 2>"$log_prefix.exec.err"
    ) || E2E_EXEC_EXIT=$?
}

e2e_worker_binary() {
    local target_dir="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}" candidate
    for candidate in "$target_dir/debug/rch-wkr" "$target_dir/release/rch-wkr"; do
        if [[ -x "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    printf 'Build rch-wkr before running worker topology scenarios\n' >&2
    return 1
}

# A successful Cargo command can still select zero tests. Retain each exact
# invocation's output and require executed tests before admitting its family.
e2e_cargo_test() {
    local test_log command_exit=0 passed_count
    test_log="$(mktemp "${TMPDIR:-/tmp}/rch-e2e-cargo-test-XXXXXX")" || return 1
    if cargo test "$@" 2>&1 | tee "$test_log"; then
        passed_count="$(awk '/^test result: ok\./ {sum += $4} END {print sum+0}' "$test_log")"
    else
        command_exit=$?
        printf 'Cargo test command failed (exit %s); retained log: %s\n' "$command_exit" "$test_log" >&2
        return "$command_exit"
    fi
    if [[ ! "$passed_count" =~ ^[0-9]+$ || "$passed_count" -eq 0 ]]; then
        printf 'Cargo test command selected no passing tests; retained log: %s\n' "$test_log" >&2
        return 1
    fi
    printf '[E2E] Cargo test invocation ran %s passing tests; log: %s\n' "$passed_count" "$test_log"
}

# Consume the temporary topology through the actual worker capability probe.
# Both invalid shapes must be rejected; a symlink's existence alone proves no
# production admission behavior.
e2e_assert_worker_topology() {
    local worker_bin="$1" topology_dir="$2"
    local canonical_root alias_root wrong_alias
    mkdir -p "$topology_dir/data/projects" "$topology_dir/wrong-projects"
    canonical_root="$(cd "$topology_dir/data/projects" && pwd -P)" || return 1
    alias_root="$topology_dir/dp"
    wrong_alias="$topology_dir/wrong-dp"
    ln -s "$canonical_root" "$alias_root" || return 1
    ln -s "$topology_dir/wrong-projects" "$wrong_alias" || return 1
    env RCH_WKR_CANONICAL_ROOT="$canonical_root" RCH_WKR_ALIAS_ROOT="$alias_root" \
        "$worker_bin" capabilities >"$topology_dir/capabilities.valid.json" \
        2>"$topology_dir/capabilities.valid.err" || return 1
    jq -e '.projects_root_ok == true and .projects_root_issue == null' \
        "$topology_dir/capabilities.valid.json" >/dev/null || return 1
    env RCH_WKR_CANONICAL_ROOT="$canonical_root" RCH_WKR_ALIAS_ROOT="$wrong_alias" \
        "$worker_bin" capabilities >"$topology_dir/capabilities.wrong-alias.json" \
        2>"$topology_dir/capabilities.wrong-alias.err" || return 1
    jq -e '.projects_root_ok == false and (.projects_root_issue | type) == "string" and
           (.projects_root_issue | length) > 0' "$topology_dir/capabilities.wrong-alias.json" >/dev/null || return 1
    env RCH_WKR_CANONICAL_ROOT="$canonical_root" RCH_WKR_ALIAS_ROOT="$topology_dir/missing-alias" \
        "$worker_bin" capabilities >"$topology_dir/capabilities.missing-alias.json" \
        2>"$topology_dir/capabilities.missing-alias.err" || return 1
    jq -e '.projects_root_ok == false and (.projects_root_issue | type) == "string" and
           (.projects_root_issue | length) > 0' "$topology_dir/capabilities.missing-alias.json" >/dev/null
}
