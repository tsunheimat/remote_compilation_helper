#!/usr/bin/env bash
#
# e2e_config_inspect.sh - Config lint and diff commands (bd-159h)
#
# Verifies:
# - config lint detects missing workers.toml
# - config lint detects risky configurations
# - config diff shows non-default values
# - JSON output is stable and parseable
# - Exit codes are correct (1 for lint errors, 0 otherwise)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="${RCH_E2E_LOG:-$PROJECT_ROOT/target/e2e_config_inspect.jsonl}"

timestamp() {
    date -u '+%Y-%m-%dT%H:%M:%S.%3NZ' 2>/dev/null || date -u '+%Y-%m-%dT%H:%M:%SZ'
}

log_json() {
    local phase="$1"
    local message="$2"
    local extra="${3:-}"
    [[ -n "$extra" ]] || extra='{}'
    local ts
    ts="$(timestamp)"
    jq -nc --arg ts "$ts" --arg phase "$phase" --arg message "$message" \
        --argjson extra "$extra" \
        '{ts:$ts,test:"config_inspect",phase:$phase,message:$message} + $extra' \
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

test_lint_missing_workers() {
    local rch_bin="$1"
    local tmp_dir="$2"

    log_json "test" "config lint detects missing workers.toml"

    # Create a minimal config without workers.toml
    local test_home="$tmp_dir/home-no-workers"
    mkdir -p "$test_home/.config/rch"
    cat > "$test_home/.config/rch/config.toml" <<'EOF'
[general]
enabled = true
EOF

    # Select only this fixture's config, independent of the host config and cwd.
    local exit_code=0
    local output
    output=$(cd "$test_home" && RCH_CONFIG_DIR="$test_home/.config/rch" "$rch_bin" --json config lint 2>&1) || exit_code=$?

    # Should exit with code 1 (errors found)
    if [[ "$exit_code" -ne 1 ]]; then
        die "Expected exit code 1 for missing workers.toml, got: $exit_code"
    fi

    # Should contain LINT-E001 or LINT-E002
    if ! echo "$output" | jq -e '.data.issues[] | select(.code | startswith("LINT-E"))' >/dev/null 2>&1; then
        die "Expected LINT-E error for missing workers, got: $output"
    fi

    log_json "verify" "Missing workers detection ok" '{"lint_error":"LINT-E001"}'
}

test_lint_json_output() {
    local rch_bin="$1"
    local tmp_dir="$2"

    log_json "test" "config lint JSON output is parseable"

    local output
    output=$("$rch_bin" --json config lint 2>&1 || true)

    # Should be valid JSON with expected fields
    if ! echo "$output" | jq -e '.data.issues' >/dev/null 2>&1; then
        die "JSON output missing 'issues' field: $output"
    fi
    if ! echo "$output" | jq -e '.data.error_count' >/dev/null 2>&1; then
        die "JSON output missing 'error_count' field: $output"
    fi
    if ! echo "$output" | jq -e '.data.warning_count' >/dev/null 2>&1; then
        die "JSON output missing 'warning_count' field: $output"
    fi

    log_json "verify" "Lint JSON output valid" '{"fields":"issues,error_count,warning_count"}'
}

test_diff_shows_changes() {
    local rch_bin="$1"
    local tmp_dir="$2"

    log_json "test" "config diff shows non-default values"

    # Create a config with non-default values
    local test_home="$tmp_dir/home-custom"
    mkdir -p "$test_home/.config/rch"
    cat > "$test_home/.config/rch/config.toml" <<'EOF'
[general]
log_level = "warn"

[compilation]
confidence_threshold = 0.5

[transfer]
compression_level = 10
EOF
    # Also create a minimal workers.toml so lint doesn't fail
    cat > "$test_home/.config/rch/workers.toml" <<'EOF'
[[workers]]
id = "test-worker"
host = "localhost"
user = "test"
EOF

    local output
    output=$(cd "$test_home" && RCH_CONFIG_DIR="$test_home/.config/rch" "$rch_bin" --json config diff 2>&1)

    # Should show changes for log_level, confidence_threshold, compression_level
    local total_changes
    total_changes=$(echo "$output" | jq -r '.data.total_changes')

    if [[ "$total_changes" -lt 3 ]]; then
        die "Expected at least 3 changed values, got: $total_changes"
    fi

    # Verify specific changes are detected
    if ! echo "$output" | jq -e '.data.entries[] | select(.key == "general.log_level")' >/dev/null 2>&1; then
        die "Missing log_level change in diff"
    fi

    log_json "verify" "Config diff detects changes" "{\"total_changes\":$total_changes}"
}

test_diff_json_output() {
    local rch_bin="$1"

    log_json "test" "config diff JSON output is parseable"

    local output
    output=$("$rch_bin" --json config diff 2>&1)

    # Should be valid JSON with expected fields
    if ! echo "$output" | jq -e '.data.entries' >/dev/null 2>&1; then
        die "JSON output missing 'entries' field: $output"
    fi
    if ! echo "$output" | jq -e '.data.total_changes' >/dev/null 2>&1; then
        die "JSON output missing 'total_changes' field: $output"
    fi

    log_json "verify" "Diff JSON output valid" '{"fields":"entries,total_changes"}'
}

test_lint_risky_config() {
    local rch_bin="$1"
    local tmp_dir="$2"

    log_json "test" "config lint detects risky settings"

    # Create a config with risky settings
    local test_home="$tmp_dir/home-risky"
    mkdir -p "$test_home/.config/rch"
    cat > "$test_home/.config/rch/config.toml" <<'EOF'
[general]
enabled = false

[compilation]
confidence_threshold = 0.1
build_timeout_sec = 10

[transfer]
compression_level = 0
exclude_patterns = ["src/"]
EOF
    cat > "$test_home/.config/rch/workers.toml" <<'EOF'
[[workers]]
id = "test-worker"
host = "localhost"
user = "test"
EOF

    local output
    output=$(cd "$test_home" && RCH_CONFIG_DIR="$test_home/.config/rch" "$rch_bin" --json config lint 2>&1 || true)

    # Should detect multiple warnings
    local warning_count
    warning_count=$(echo "$output" | jq -r '.data.warning_count')

    if [[ "$warning_count" -lt 3 ]]; then
        die "Expected at least 3 warnings for risky config, got: $warning_count"
    fi

    log_json "verify" "Risky settings detected" "{\"warning_count\":$warning_count}"
}

main() {
    mkdir -p "$(dirname "$LOG_FILE")"
    : > "$LOG_FILE"
    check_dependencies

    local rch_bin
    rch_bin="$(build_rch)"

    local tmp_root
    tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/rch-config-inspect-XXXXXX")"

    log_json "setup" "Test directory created" "{\"root\":\"$tmp_root\"}"

    # Run tests
    test_lint_json_output "$rch_bin" "$tmp_root"
    test_diff_json_output "$rch_bin"
    test_lint_missing_workers "$rch_bin" "$tmp_root"
    test_diff_shows_changes "$rch_bin" "$tmp_root"
    test_lint_risky_config "$rch_bin" "$tmp_root"

    log_json "setup" "Retained test fixtures" "{\"root\":\"$tmp_root\"}"
    log_json "summary" "All config_inspect checks passed" '{"result":"pass"}'
}

main "$@"
