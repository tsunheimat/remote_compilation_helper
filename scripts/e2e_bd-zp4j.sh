#!/usr/bin/env bash
#
# e2e_bd-zp4j.sh - Configurable remote temp base path
#
# Verifies:
# - Custom remote_base can be configured in project config
# - remote_base is validated (absolute path, no traversal)
# - Default remote_base is /data/tmp/rch
# - Output is logged in JSONL format

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="${RCH_E2E_LOG:-$PROJECT_ROOT/target/e2e_bd-zp4j.jsonl}"

timestamp() {
    date -u '+%Y-%m-%dT%H:%M:%S.%3NZ' 2>/dev/null || date -u '+%Y-%m-%dT%H:%M:%SZ'
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
        --arg test "bd-zp4j" \
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
    for cmd in cargo jq; do
        command -v "$cmd" >/dev/null 2>&1 || die "Missing dependency: $cmd"
    done
}

build_rch() {
    local rch_bin="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rch"
    if [[ -x "$rch_bin" ]]; then
        log_json "setup" "Using existing rch binary" "{\"path\":\"$rch_bin\"}" >&2
        echo "$rch_bin"
        return
    fi
    log_json "setup" "Building rch (debug)" >&2
    (cd "$PROJECT_ROOT" && cargo build -p rch >/dev/null 2>&1) || die "cargo build failed"
    [[ -x "$rch_bin" ]] || die "rch binary missing after build"
    echo "$rch_bin"
}

write_project_config() {
    local project_dir="$1"
    local remote_base="$2"
    mkdir -p "$project_dir/.rch"
    cat > "$project_dir/.rch/config.toml" <<EOF
[transfer]
remote_base = "$remote_base"
EOF
}

read_remote_base() {
    local rch_bin="$1"
    local project_dir="$2"
    local stderr_file="${3:-$project_dir/config-show.stderr}"
    (cd "$project_dir" && "$rch_bin" config show --json 2>"$stderr_file") \
        | jq -er '.data.transfer.remote_base | select(type == "string" and length > 0)'
}

main() {
    mkdir -p "$(dirname "$LOG_FILE")"
    : > "$LOG_FILE"
    check_dependencies
    local rch_bin
    rch_bin="$(build_rch)"

    local tmp_root
    tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/rch-remote-base-XXXXXX")"
    mkdir -p "$tmp_root/config"
    export RCH_CONFIG_DIR="$tmp_root/config"
    # Intentionally do not auto-delete temp dirs (avoid destructive rm -rf patterns).

    local project_default="$tmp_root/project-default"
    local project_custom="$tmp_root/project-custom"
    local project_tilde="$tmp_root/project-tilde"
    local project_invalid="$tmp_root/project-invalid"
    mkdir -p "$project_default" "$project_custom" "$project_tilde" "$project_invalid"

    log_json "setup" "Created test projects" "{\"root\":\"$tmp_root\"}"

    # Test 1: Default remote_base
    log_json "test" "Default remote_base is /data/tmp/rch"
    local default_base
    default_base="$(read_remote_base "$rch_bin" "$project_default")"
    if [[ "$default_base" != "/data/tmp/rch" ]]; then
        die "Expected default remote_base /data/tmp/rch, got: $default_base"
    fi
    log_json "verify" "Default remote_base ok" "{\"remote_base\":\"$default_base\"}"

    # Test 2: Custom remote_base
    log_json "test" "Custom remote_base /var/rch-builds"
    write_project_config "$project_custom" "/var/rch-builds"
    local custom_base
    custom_base="$(read_remote_base "$rch_bin" "$project_custom")"
    if [[ "$custom_base" != "/var/rch-builds" ]]; then
        die "Expected custom remote_base /var/rch-builds, got: $custom_base"
    fi
    log_json "verify" "Custom remote_base ok" "{\"remote_base\":\"$custom_base\"}"

    # Test 3: Tilde expansion
    log_json "test" "Tilde expansion in remote_base"
    write_project_config "$project_tilde" "~/rch-builds"
    local tilde_base
    tilde_base="$(read_remote_base "$rch_bin" "$project_tilde")"
    if [[ "$tilde_base" == "~/rch-builds" ]] || [[ ! "$tilde_base" =~ ^/ ]]; then
        die "Expected tilde to be expanded to absolute path, got: $tilde_base"
    fi
    log_json "verify" "Tilde expansion ok" "{\"remote_base\":\"$tilde_base\"}"

    # Test 4: Path with trailing slash is normalized
    log_json "test" "Trailing slash normalization"
    write_project_config "$project_custom" "/var/rch-builds/"
    local normalized_base
    normalized_base="$(read_remote_base "$rch_bin" "$project_custom")"
    if [[ "$normalized_base" != "/var/rch-builds" ]]; then
        die "Expected normalized remote_base /var/rch-builds, got: $normalized_base"
    fi
    log_json "verify" "Trailing slash normalized" "{\"remote_base\":\"$normalized_base\"}"

    # Test 5: A rejected relative project path retains the default.
    log_json "test" "Relative path validation"
    write_project_config "$project_invalid" "relative/path"
    local invalid_base
    local relative_stderr="$project_invalid/relative.stderr"
    invalid_base="$(read_remote_base "$rch_bin" "$project_invalid" "$relative_stderr")" \
        || die "Relative path validation command failed (stderr: $relative_stderr)"
    [[ "$invalid_base" == "$default_base" ]] \
        || die "Rejected relative path must retain $default_base, got: $invalid_base (stderr: $relative_stderr)"
    log_json "verify" "Relative path rejected; default retained" \
        "{\"remote_base\":\"$invalid_base\",\"stderr_path\":\"$relative_stderr\"}"

    # Test 6: A rejected traversal path retains an accepted user-level path.
    log_json "test" "Path traversal rejection"
    write_project_config "$project_invalid" "/tmp/../etc/rch"
    cat > "$RCH_CONFIG_DIR/config.toml" <<'EOF'
[transfer]
remote_base = "/var/rch-user-builds"
EOF
    local traversal_base
    local traversal_stderr="$project_invalid/traversal.stderr"
    traversal_base="$(read_remote_base "$rch_bin" "$project_invalid" "$traversal_stderr")" \
        || die "Traversal validation command failed (stderr: $traversal_stderr)"
    [[ "$traversal_base" == "/var/rch-user-builds" ]] \
        || die "Rejected traversal path must retain /var/rch-user-builds, got: $traversal_base (stderr: $traversal_stderr)"
    log_json "verify" "Traversal rejected; accepted user path retained" \
        "{\"remote_base\":\"$traversal_base\",\"stderr_path\":\"$traversal_stderr\"}"

    log_json "summary" "All bd-zp4j checks passed" '{"result":"pass","tests_run":6}'
}

main "$@"
