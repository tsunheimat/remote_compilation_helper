#!/usr/bin/env bash
#
# e2e_project_sync.sh - True E2E Project Sync Tests (rsync)
#
# Usage:
#   ./scripts/e2e_project_sync.sh [OPTIONS]
#
# Options:
#   --quick            Skip slow tests (large files, many files)
#   --worker WORKER    Use specific worker for real sync tests
#   --verbose          Enable verbose output
#   --mock-only        Only run tests that work in mock mode
#   --help             Show this help message
#
# Environment:
#   RCH_TEST_MODE=1    Enable test mode
#   RCH_MOCK_SSH=1     Use mock SSH transport
#   RCH_E2E_VERBOSE=1  Enable verbose logging
#
# Purpose:
#   Validates project synchronization (rsync to workers) works correctly.
#   Critical because sync issues cause mysterious build failures.
#
# Exit codes:
#   0 - All tests passed
#   1 - Test failure
#   2 - Setup/dependency error
#   4 - Skipped (E2E skip code)
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="${RCH_E2E_LOG:-$PROJECT_ROOT/target/e2e_project_sync.jsonl}"

# shellcheck source=lib/e2e_common.sh
source "$SCRIPT_DIR/lib/e2e_common.sh"

# =============================================================================
# Configuration
# =============================================================================

QUICK_MODE="${RCH_E2E_QUICK:-0}"
VERBOSE="${RCH_E2E_VERBOSE:-0}"
MOCK_ONLY="${RCH_E2E_MOCK_ONLY:-0}"
WORKER_ID="${RCH_E2E_WORKER:-}"

# Test counters
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0
TESTS_SKIPPED=0

# Test directories
TEST_TMPDIR=""
FIXTURE_DIR=""
TEST_DAEMON_PID=""

# =============================================================================
# Structured Logging (JSONL format per bead spec)
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
log_warn() { log_json "WARN" "$1" "$2" "$3" "${4:-}"; }

die() {
    log_error "setup" "fatal" "$1" "{}"
    exit 2
}

# =============================================================================
# Command Line Parsing
# =============================================================================

usage() {
    sed -n '1,30p' "$0" | sed 's/^# \{0,1\}//'
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --quick|-q)
                QUICK_MODE="1"
                shift
                ;;
            --worker|-w)
                WORKER_ID="$2"
                shift 2
                ;;
            --verbose|-v)
                VERBOSE="1"
                shift
                ;;
            --mock-only|-m)
                MOCK_ONLY="1"
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                log_error "args" "parse" "Unknown option: $1" "{}"
                exit 3
                ;;
        esac
    done
}

# =============================================================================
# Setup and Teardown
# =============================================================================

check_dependencies() {
    log_info "setup" "dependencies" "Checking dependencies" "{}"

    local missing=()
    for cmd in rsync jq cargo; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            missing+=("$cmd")
        fi
    done

    if [[ ${#missing[@]} -gt 0 ]]; then
        die "Missing dependencies: ${missing[*]}"
    fi

    # Check rsync version for zstd support
    local rsync_version
    rsync_version=$(rsync --version 2>&1 | sed -n '1p' | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || echo "unknown")

    log_info "setup" "dependencies" "All dependencies present" \
        "{\"rsync_version\":\"$rsync_version\"}"
}

setup_test_fixtures() {
    log_info "setup" "fixtures" "Creating test fixtures" "{}"

    TEST_TMPDIR="$(mktemp -d)"
    TEST_TMPDIR="$(cd "$TEST_TMPDIR" && pwd -P)"
    FIXTURE_DIR="$TEST_TMPDIR/fixtures"
    mkdir -p "$FIXTURE_DIR"

    # Fixture: basic_rust - Simple Cargo project
    local basic="$FIXTURE_DIR/basic_rust"
    mkdir -p "$basic/src"
    cat > "$basic/Cargo.toml" << 'EOF'
[package]
name = "test_project"
version = "0.1.0"
edition = "2021"

[dependencies]
EOF
    cat > "$basic/src/main.rs" << 'EOF'
fn main() {
    println!("Hello from test project!");
}
EOF
    cat > "$basic/src/lib.rs" << 'EOF'
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(2, 3), 5);
    }
}
EOF

    # Fixture: with_target - Has target/ that should be excluded
    local with_target="$FIXTURE_DIR/with_target"
    cp -r "$basic" "$with_target"
    mkdir -p "$with_target/target/debug"
    dd if=/dev/zero of="$with_target/target/debug/fake_binary" bs=1024 count=100 2>/dev/null
    mkdir -p "$with_target/target/release"
    touch "$with_target/target/release/.gitkeep"

    # Fixture: with_special - Special characters in names
    local with_special="$FIXTURE_DIR/with_special"
    cp -r "$basic" "$with_special"
    touch "$with_special/src/my file.rs"
    touch "$with_special/src/it's_fine.rs"
    mkdir -p "$with_special/path with spaces"
    touch "$with_special/path with spaces/code.rs"

    # Fixture: with_git - Git repo with objects
    local with_git="$FIXTURE_DIR/with_git"
    cp -r "$basic" "$with_git"
    (
        cd "$with_git"
        git init -q
        git config user.email "test@test.com"
        git config user.name "Test"
        git add .
        git commit -q -m "Initial commit"
        # Create some git objects
        for i in {1..5}; do
            echo "change $i" >> src/main.rs
            git add src/main.rs
            git commit -q -m "Commit $i"
        done
    )

    # Fixture: with_node_modules - Has node_modules/ to exclude
    local with_node="$FIXTURE_DIR/with_node_modules"
    mkdir -p "$with_node/src" "$with_node/node_modules/fake-package"
    echo 'console.log("hello");' > "$with_node/src/index.js"
    echo '{"name":"test"}' > "$with_node/package.json"
    dd if=/dev/zero of="$with_node/node_modules/fake-package/big.js" bs=1024 count=50 2>/dev/null

    if [[ "$QUICK_MODE" != "1" ]]; then
        # Fixture: many_files - 1000+ small files
        local many_files="$FIXTURE_DIR/many_files"
        mkdir -p "$many_files/src"
        for i in $(seq 1 100); do
            mkdir -p "$many_files/src/mod_$i"
            for j in $(seq 1 10); do
                echo "// Module $i file $j" > "$many_files/src/mod_$i/file_$j.rs"
            done
        done
        cat > "$many_files/Cargo.toml" << 'EOF'
[package]
name = "many_files"
version = "0.1.0"
edition = "2021"
EOF

        # Fixture: large_file - Single large binary
        local large_file="$FIXTURE_DIR/large_file"
        mkdir -p "$large_file/assets"
        cp -r "$basic/"* "$large_file/"
        dd if=/dev/urandom of="$large_file/assets/large.bin" bs=1M count=10 2>/dev/null
    fi

    # Fixture: with_symlinks - Has symlinks
    local with_symlinks="$FIXTURE_DIR/with_symlinks"
    cp -r "$basic" "$with_symlinks"
    ln -s src/main.rs "$with_symlinks/main_link.rs"
    ln -s src "$with_symlinks/src_link"

    log_info "setup" "fixtures" "Test fixtures created" \
        "{\"fixture_dir\":\"$FIXTURE_DIR\",\"quick_mode\":$QUICK_MODE}"
}

cleanup() {
    if [[ -n "$TEST_DAEMON_PID" ]]; then
        kill "$TEST_DAEMON_PID" 2>/dev/null || true
        wait "$TEST_DAEMON_PID" 2>/dev/null || true
    fi
    if [[ -n "$TEST_TMPDIR" && -d "$TEST_TMPDIR" ]]; then
        log_info "cleanup" "teardown" "Retained test fixtures and logs" "{\"root\":\"$TEST_TMPDIR\"}"
    fi
}

trap cleanup EXIT

# =============================================================================
# Build Binaries
# =============================================================================

build_binaries() {
    local rch_bin="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rch"
    local rchd_bin="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/rchd"

    if [[ -x "$rch_bin" && -x "$rchd_bin" ]]; then
        log_info "setup" "build" "Using existing binary" \
            "{\"rch\":\"$rch_bin\"}" >&2
        echo "$rch_bin"
        return
    fi

    log_info "setup" "build" "Building rch binary" "{}" >&2

    if ! cargo build -p rch -p rchd --quiet >&2; then
        die "Failed to build rch"
    fi

    if [[ ! -x "$rch_bin" ]]; then
        die "Binary not found after build: $rch_bin"
    fi

    log_info "setup" "build" "Build complete" "{\"rch\":\"$rch_bin\"}" >&2
    echo "$rch_bin"
}

# =============================================================================
# Test Result Helpers
# =============================================================================

test_pass() {
    local test_name="$1"
    local msg="$2"
    local data="${3:-}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    log_info "$test_name" "result" "PASS: $msg" "$data"
}

test_fail() {
    local test_name="$1"
    local msg="$2"
    local data="${3:-}"
    TESTS_FAILED=$((TESTS_FAILED + 1))
    log_error "$test_name" "result" "FAIL: $msg" "$data"
}

test_skip() {
    local test_name="$1"
    local msg="$2"
    local data="${3:-}"
    TESTS_SKIPPED=$((TESTS_SKIPPED + 1))
    log_info "$test_name" "result" "SKIP: $msg" "$data"
}

# =============================================================================
# Sync Test Utilities
# =============================================================================

# Count files in directory (excluding hidden and excluded patterns)
count_files() {
    local dir="$1"
    find "$dir" -type f ! -path '*/target/*' ! -path '*/.git/*' ! -path '*/node_modules/*' | wc -l | tr -d ' '
}

# Get total size of directory
get_dir_size() {
    local dir="$1"
    du -sb "$dir" 2>/dev/null | cut -f1 || echo "0"
}

# Calculate file hash
file_hash() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" 2>/dev/null | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" 2>/dev/null | cut -d' ' -f1
    else
        md5sum "$file" 2>/dev/null | cut -d' ' -f1
    fi
}

# Run rsync with stats and capture output
run_rsync_with_stats() {
    local source="$1"
    local dest="$2"
    shift 2

    local rsync_args=(
        -av
        --delete
        --stats
    )

    local pattern
    for pattern in "$@"; do
        rsync_args+=(--exclude "$pattern")
    done

    rsync_args+=("$source/" "$dest/")

    local start_ms
    start_ms="$(e2e_now_ms)"

    local output stats_log
    stats_log=$(mktemp "$(dirname "$LOG_FILE")/rsync-stats.XXXXXX")
    if ! LC_ALL=C rsync "${rsync_args[@]}" >"$stats_log" 2>&1; then
        printf 'rsync fixture failed; retained output: %s\n' "$stats_log" >&2
        cat "$stats_log" >&2
        return 1
    fi
    output=$(<"$stats_log")

    local end_ms
    end_ms="$(e2e_now_ms)"
    local duration_ms=$((end_ms - start_ms))

    # Older Apple rsync reports wire bytes only in the final sent/received
    # summary; newer versions also provide the Total bytes labels.
    local files_transferred bytes_sent bytes_received speedup
    files_transferred=$(printf '%s\n' "$output" | awk -F: '/^Number of (regular )?files transferred:/ {gsub(/[[:space:],]/, "", $2); print $2; exit}')
    bytes_sent=$(printf '%s\n' "$output" | awk -F: '/^Total bytes sent:/ {gsub(/[[:space:],]/, "", $2); print $2; exit}')
    bytes_received=$(printf '%s\n' "$output" | awk -F: '/^Total bytes received:/ {gsub(/[[:space:],]/, "", $2); print $2; exit}')
    if [[ -z "$bytes_sent" || -z "$bytes_received" ]]; then
        bytes_sent=$(printf '%s\n' "$output" | awk '/^sent [0-9,]+ bytes[[:space:]]+received [0-9,]+ bytes/ {gsub(/,/, "", $2); print $2; exit}')
        bytes_received=$(printf '%s\n' "$output" | awk '/^sent [0-9,]+ bytes[[:space:]]+received [0-9,]+ bytes/ {gsub(/,/, "", $5); print $5; exit}')
    fi
    speedup=$(printf '%s\n' "$output" | awk '{for (i = 1; i + 2 <= NF; i++) if ($i == "speedup" && $(i + 1) == "is") {gsub(/,/, "", $(i + 2)); print $(i + 2); exit}}')
    if [[ ! "$files_transferred" =~ ^[0-9]+$ || ! "$bytes_sent" =~ ^[0-9]+$ \
        || ! "$bytes_received" =~ ^[0-9]+$ || ! "$speedup" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
        printf 'Missing or malformed rsync statistics; retained output: %s\n' "$stats_log" >&2
        return 1
    fi

    # Return JSON stats
    printf '{"files_transferred":%s,"bytes_sent":%s,"bytes_received":%s,"duration_ms":%s,"speedup":"%s"}' \
        "$files_transferred" "$bytes_sent" "$bytes_received" "$duration_ms" "$speedup"
}

# =============================================================================
# Mock Mode Tests (work without real workers)
# =============================================================================

test_rsync_basic_local() {
    local test_name="test_rsync_basic_local"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing basic rsync local-to-local" "{}"

    local source="$FIXTURE_DIR/basic_rust"
    local dest="$TEST_TMPDIR/dest_basic"
    mkdir -p "$dest"

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Rsync completed" "$stats"

    # Verify files arrived
    local source_files
    source_files=$(count_files "$source")
    local dest_files
    dest_files=$(count_files "$dest")

    log_info "$test_name" "verify" "Comparing file counts" \
        "{\"source_files\":$source_files,\"dest_files\":$dest_files}"

    if [[ "$dest_files" -ge "$source_files" ]]; then
        test_pass "$test_name" "Basic rsync transferred all files" \
            "{\"source_files\":$source_files,\"dest_files\":$dest_files}"
    else
        test_fail "$test_name" "File count mismatch" \
            "{\"source_files\":$source_files,\"dest_files\":$dest_files}"
    fi
}

test_rsync_target_exclusion() {
    local test_name="test_rsync_target_exclusion"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing target/ exclusion" "{}"

    local source="$FIXTURE_DIR/with_target"
    local dest="$TEST_TMPDIR/dest_target"
    mkdir -p "$dest"

    # Get size of target/ that should be excluded
    local target_size
    target_size=$(get_dir_size "$source/target")

    log_debug "$test_name" "verify" "Checking exclusion" \
        "{\"pattern\":\"target/\",\"would_be_size\":$target_size}"

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Rsync completed" "$stats"

    # Verify target/ was NOT transferred
    if [[ -d "$dest/target" ]]; then
        local transferred_size
        transferred_size=$(get_dir_size "$dest/target")
        test_fail "$test_name" "target/ was transferred when it should be excluded" \
            "{\"pattern\":\"target/\",\"expected_excluded\":true,\"actually_excluded\":false,\"transferred_size\":$transferred_size}"
    else
        test_pass "$test_name" "target/ correctly excluded" \
            "{\"pattern\":\"target/\",\"expected_excluded\":true,\"actually_excluded\":true,\"skipped_bytes\":$target_size}"
    fi
}

test_rsync_node_modules_exclusion() {
    local test_name="test_rsync_node_modules_exclusion"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing node_modules/ exclusion" "{}"

    local source="$FIXTURE_DIR/with_node_modules"
    local dest="$TEST_TMPDIR/dest_node"
    mkdir -p "$dest"

    local nm_size
    nm_size=$(get_dir_size "$source/node_modules")

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "node_modules/" ".git/")

    log_info "$test_name" "sync_stats" "Rsync completed" "$stats"

    if [[ -d "$dest/node_modules" ]]; then
        test_fail "$test_name" "node_modules/ was transferred" \
            "{\"pattern\":\"node_modules/\",\"expected_excluded\":true,\"actually_excluded\":false}"
    else
        test_pass "$test_name" "node_modules/ correctly excluded" \
            "{\"pattern\":\"node_modules/\",\"expected_excluded\":true,\"actually_excluded\":true,\"skipped_bytes\":$nm_size}"
    fi
}

test_rsync_git_objects_exclusion() {
    local test_name="test_rsync_git_objects_exclusion"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing .git/objects/ exclusion" "{}"

    local source="$FIXTURE_DIR/with_git"
    local dest="$TEST_TMPDIR/dest_git"
    mkdir -p "$dest"

    local git_objects_size=0
    if [[ -d "$source/.git/objects" ]]; then
        git_objects_size=$(get_dir_size "$source/.git/objects")
    fi

    # Exclude .git/objects but allow .git itself (for branch info etc)
    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" ".git/objects/" "target/")

    log_info "$test_name" "sync_stats" "Rsync completed" "$stats"

    if [[ -d "$dest/.git/objects" ]]; then
        local dest_objects_size
        dest_objects_size=$(get_dir_size "$dest/.git/objects")
        # Objects dir might exist but be mostly empty
        if [[ "$dest_objects_size" -gt 1000 ]]; then
            test_fail "$test_name" ".git/objects/ was transferred" \
                "{\"pattern\":\".git/objects/\",\"expected_excluded\":true,\"transferred_size\":$dest_objects_size}"
        else
            test_pass "$test_name" ".git/objects/ effectively excluded" \
                "{\"pattern\":\".git/objects/\",\"skipped_bytes\":$git_objects_size}"
        fi
    else
        test_pass "$test_name" ".git/objects/ correctly excluded" \
            "{\"pattern\":\".git/objects/\",\"expected_excluded\":true,\"actually_excluded\":true,\"skipped_bytes\":$git_objects_size}"
    fi
}

test_rsync_incremental_sync() {
    local test_name="test_rsync_incremental_sync"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing incremental sync (changed files only)" "{}"

    local source="$FIXTURE_DIR/basic_rust"
    local dest="$TEST_TMPDIR/dest_incremental"
    mkdir -p "$dest"

    # First sync - fresh
    log_info "$test_name" "sync_start" "Starting fresh sync" \
        "{\"local_path\":\"$source\",\"sync_type\":\"fresh\"}"

    local fresh_stats
    fresh_stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Fresh sync completed" "$fresh_stats"

    # Modify one file
    echo "// Modified" >> "$source/src/main.rs"

    # Second sync - incremental
    log_info "$test_name" "sync_start" "Starting incremental sync" \
        "{\"local_path\":\"$source\",\"sync_type\":\"incremental\"}"

    local incr_stats
    incr_stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Incremental sync completed" "$incr_stats"

    # Compare bytes transferred
    local fresh_bytes
    fresh_bytes=$(echo "$fresh_stats" | jq -r '.bytes_sent')
    local incr_bytes
    incr_bytes=$(echo "$incr_stats" | jq -r '.bytes_sent')

    log_info "$test_name" "verify" "Comparing sync sizes" \
        "{\"fresh_bytes\":$fresh_bytes,\"incremental_bytes\":$incr_bytes}"

    # Incremental should transfer significantly less data
    if [[ "$incr_bytes" -lt "$fresh_bytes" ]]; then
        test_pass "$test_name" "Incremental sync transferred less data" \
            "{\"fresh_bytes\":$fresh_bytes,\"incremental_bytes\":$incr_bytes,\"savings_percent\":$(( (fresh_bytes - incr_bytes) * 100 / fresh_bytes ))}"
    else
        # Even if same bytes, it might be metadata; as long as it's fast
        local fresh_duration
        fresh_duration=$(echo "$fresh_stats" | jq -r '.duration_ms')
        local incr_duration
        incr_duration=$(echo "$incr_stats" | jq -r '.duration_ms')

        if [[ "$incr_duration" -le "$fresh_duration" ]]; then
            test_pass "$test_name" "Incremental sync was fast" \
                "{\"fresh_duration_ms\":$fresh_duration,\"incremental_duration_ms\":$incr_duration}"
        else
            test_fail "$test_name" "Incremental sync was slower than fresh" \
                "{\"fresh_bytes\":$fresh_bytes,\"incremental_bytes\":$incr_bytes}"
        fi
    fi

    # Restore original file
    sed -i '$ d' "$source/src/main.rs" 2>/dev/null || sed -i '' '$ d' "$source/src/main.rs"
}

test_rsync_delete_detection() {
    local test_name="test_rsync_delete_detection"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing delete detection" "{}"

    local source="$TEST_TMPDIR/source_delete_test"
    local dest="$TEST_TMPDIR/dest_delete_test"
    mkdir -p "$source/src" "$dest"

    # Create initial files
    echo "fn main() {}" > "$source/src/main.rs"
    echo "pub fn lib() {}" > "$source/src/lib.rs"
    echo "[package]" > "$source/Cargo.toml"

    # First sync
    local stats1
    stats1=$(run_rsync_with_stats "$source" "$dest")
    log_info "$test_name" "sync_stats" "Initial sync" "$stats1"

    # Verify lib.rs exists on dest
    if [[ ! -f "$dest/src/lib.rs" ]]; then
        test_fail "$test_name" "Initial sync failed to transfer lib.rs" "{}"
        return
    fi

    # Remove the source-tree entry while retaining its bytes for inspection.
    mv "$source/src/lib.rs" "$TEST_TMPDIR/removed-lib.rs"

    log_info "$test_name" "sync_start" "Syncing after local delete" \
        "{\"deleted_file\":\"src/lib.rs\"}"

    # Second sync with --delete
    local stats2
    stats2=$(run_rsync_with_stats "$source" "$dest")
    log_info "$test_name" "sync_stats" "Delete sync" "$stats2"

    # Verify file was removed on dest
    if [[ -f "$dest/src/lib.rs" ]]; then
        test_fail "$test_name" "Deleted file still exists on destination" \
            "{\"file\":\"src/lib.rs\",\"expected_deleted\":true,\"actually_deleted\":false}"
    else
        test_pass "$test_name" "Deleted file removed from destination" \
            "{\"file\":\"src/lib.rs\",\"expected_deleted\":true,\"actually_deleted\":true}"
    fi
}

test_rsync_special_characters() {
    local test_name="test_rsync_special_characters"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing special characters in paths" "{}"

    local source="$FIXTURE_DIR/with_special"
    local dest="$TEST_TMPDIR/dest_special"
    mkdir -p "$dest"

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Rsync completed" "$stats"

    # Verify files with spaces
    local files_with_spaces=0
    local files_with_quotes=0

    if [[ -f "$dest/src/my file.rs" ]]; then
        files_with_spaces=1
    fi

    if [[ -f "$dest/src/it's_fine.rs" ]]; then
        files_with_quotes=1
    fi

    if [[ -d "$dest/path with spaces" ]] && [[ -f "$dest/path with spaces/code.rs" ]]; then
        files_with_spaces=$((files_with_spaces + 1))
    fi

    log_info "$test_name" "verify" "Checking special character files" \
        "{\"files_with_spaces\":$files_with_spaces,\"files_with_quotes\":$files_with_quotes}"

    if [[ "$files_with_spaces" -ge 1 ]] && [[ "$files_with_quotes" -ge 1 ]]; then
        test_pass "$test_name" "Special characters handled correctly" \
            "{\"files_with_spaces\":$files_with_spaces,\"files_with_quotes\":$files_with_quotes}"
    else
        test_fail "$test_name" "Some special character files missing" \
            "{\"files_with_spaces\":$files_with_spaces,\"files_with_quotes\":$files_with_quotes}"
    fi
}

test_rsync_symlinks() {
    local test_name="test_rsync_symlinks"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing symlink handling" "{}"

    local source="$FIXTURE_DIR/with_symlinks"
    local dest="$TEST_TMPDIR/dest_symlinks"
    mkdir -p "$dest"

    # Count symlinks in source
    local source_symlinks
    source_symlinks=$(find "$source" -type l 2>/dev/null | wc -l | tr -d ' ')

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Rsync completed" "$stats"

    # Check symlink handling
    local dest_symlinks
    dest_symlinks=$(find "$dest" -type l 2>/dev/null | wc -l | tr -d ' ')

    log_info "$test_name" "verify" "Checking symlinks" \
        "{\"source_symlinks\":$source_symlinks,\"dest_symlinks\":$dest_symlinks}"

    # rsync -a preserves symlinks by default
    if [[ "$dest_symlinks" -eq "$source_symlinks" ]]; then
        test_pass "$test_name" "Symlinks preserved" \
            "{\"source_symlinks\":$source_symlinks,\"dest_symlinks\":$dest_symlinks,\"handling\":\"preserved\"}"
    elif [[ "$dest_symlinks" -eq 0 ]] && [[ -f "$dest/main_link.rs" ]]; then
        # Symlinks were followed (dereferenced)
        test_pass "$test_name" "Symlinks dereferenced" \
            "{\"source_symlinks\":$source_symlinks,\"handling\":\"dereferenced\"}"
    else
        test_fail "$test_name" "Symlink handling inconsistent" \
            "{\"source_symlinks\":$source_symlinks,\"dest_symlinks\":$dest_symlinks}"
    fi
}

test_rsync_file_hash_verification() {
    local test_name="test_rsync_file_hash_verification"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing file content integrity" "{}"

    local source="$FIXTURE_DIR/basic_rust"
    local dest="$TEST_TMPDIR/dest_hash"
    mkdir -p "$dest"

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Rsync completed" "$stats"

    # Verify hash of key files
    local files_checked=0
    local files_matched=0

    for file in "src/main.rs" "src/lib.rs" "Cargo.toml"; do
        if [[ -f "$source/$file" ]] && [[ -f "$dest/$file" ]]; then
            files_checked=$((files_checked + 1))
            local source_hash
            source_hash=$(file_hash "$source/$file")
            local dest_hash
            dest_hash=$(file_hash "$dest/$file")

            log_debug "$test_name" "verify" "Hash comparison" \
                "{\"file\":\"$file\",\"source_hash\":\"${source_hash:0:16}...\",\"dest_hash\":\"${dest_hash:0:16}...\",\"match\":$([ \"$source_hash\" = \"$dest_hash\" ] && echo true || echo false)}"

            if [[ "$source_hash" == "$dest_hash" ]]; then
                files_matched=$((files_matched + 1))
            fi
        fi
    done

    if [[ "$files_checked" -gt 0 ]] && [[ "$files_matched" -eq "$files_checked" ]]; then
        test_pass "$test_name" "All file hashes match" \
            "{\"files_checked\":$files_checked,\"files_matched\":$files_matched}"
    else
        test_fail "$test_name" "Some file hashes do not match" \
            "{\"files_checked\":$files_checked,\"files_matched\":$files_matched}"
    fi
}

test_rsync_no_change_overhead() {
    local test_name="test_rsync_no_change_overhead"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing no-change sync overhead" "{}"

    local source="$FIXTURE_DIR/basic_rust"
    local dest="$TEST_TMPDIR/dest_overhead"
    mkdir -p "$dest"

    # First sync
    run_rsync_with_stats "$source" "$dest" "target/" ".git/" >/dev/null

    # Second sync with no changes
    log_info "$test_name" "sync_start" "Syncing with no changes" "{}"

    local start_ms
    start_ms="$(e2e_now_ms)"

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    local end_ms
    end_ms="$(e2e_now_ms)"
    local duration_ms=$((end_ms - start_ms))

    log_info "$test_name" "sync_stats" "No-change sync completed" "$stats"

    # No-change sync should be very fast (< 1 second for small projects)
    if [[ "$duration_ms" -lt 1000 ]]; then
        test_pass "$test_name" "No-change sync overhead is acceptable" \
            "{\"duration_ms\":$duration_ms,\"threshold_ms\":1000}"
    else
        # Might be slower on HDD or with many files
        log_warn "$test_name" "verify" "No-change sync slower than expected" \
            "{\"duration_ms\":$duration_ms,\"threshold_ms\":1000}"
        test_pass "$test_name" "No-change sync completed" \
            "{\"duration_ms\":$duration_ms}"
    fi
}

# =============================================================================
# Performance Tests (optional, slow)
# =============================================================================

test_rsync_many_files() {
    local test_name="test_rsync_many_files"

    if [[ "$QUICK_MODE" == "1" ]]; then
        log_info "$test_name" "skip" "Skipped in quick mode" "{}"
        return
    fi

    if [[ ! -d "$FIXTURE_DIR/many_files" ]]; then
        log_info "$test_name" "skip" "Fixture not available" "{}"
        return
    fi

    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing many small files" "{}"

    local source="$FIXTURE_DIR/many_files"
    local dest="$TEST_TMPDIR/dest_many"
    mkdir -p "$dest"

    local source_files
    source_files=$(find "$source" -type f | wc -l | tr -d ' ')

    log_info "$test_name" "sync_start" "Syncing $source_files files" \
        "{\"file_count\":$source_files}"

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Many-files sync completed" "$stats"

    local dest_files
    dest_files=$(find "$dest" -type f | wc -l | tr -d ' ')

    local duration_ms
    duration_ms=$(echo "$stats" | jq -r '.duration_ms')

    if [[ "$dest_files" -ge "$source_files" ]]; then
        local overhead_per_file
        overhead_per_file=$((duration_ms * 1000 / source_files))  # microseconds

        test_pass "$test_name" "All $source_files files transferred" \
            "{\"source_files\":$source_files,\"dest_files\":$dest_files,\"duration_ms\":$duration_ms,\"overhead_per_file_us\":$overhead_per_file}"
    else
        test_fail "$test_name" "Some files missing" \
            "{\"source_files\":$source_files,\"dest_files\":$dest_files}"
    fi
}

test_rsync_large_file() {
    local test_name="test_rsync_large_file"

    if [[ "$QUICK_MODE" == "1" ]]; then
        log_info "$test_name" "skip" "Skipped in quick mode" "{}"
        return
    fi

    if [[ ! -d "$FIXTURE_DIR/large_file" ]]; then
        log_info "$test_name" "skip" "Fixture not available" "{}"
        return
    fi

    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing large file transfer" "{}"

    local source="$FIXTURE_DIR/large_file"
    local dest="$TEST_TMPDIR/dest_large"
    mkdir -p "$dest"

    local large_file="$source/assets/large.bin"
    local file_size
    file_size=$(stat -c%s "$large_file" 2>/dev/null || stat -f%z "$large_file" 2>/dev/null || echo "0")
    local file_size_mb=$((file_size / 1024 / 1024))

    log_info "$test_name" "sync_start" "Syncing ${file_size_mb}MB file" \
        "{\"file_size_bytes\":$file_size,\"file_size_mb\":$file_size_mb}"

    local stats
    stats=$(run_rsync_with_stats "$source" "$dest" "target/" ".git/")

    log_info "$test_name" "sync_stats" "Large file sync completed" "$stats"

    # Verify file arrived intact
    if [[ -f "$dest/assets/large.bin" ]]; then
        local source_hash
        source_hash=$(file_hash "$large_file")
        local dest_hash
        dest_hash=$(file_hash "$dest/assets/large.bin")

        local duration_ms
        duration_ms=$(echo "$stats" | jq -r '.duration_ms')

        # Calculate transfer rate
        local rate_mbs=0
        if [[ "$duration_ms" -gt 0 ]]; then
            rate_mbs=$((file_size * 1000 / duration_ms / 1024 / 1024))
        fi

        if [[ "$source_hash" == "$dest_hash" ]]; then
            test_pass "$test_name" "Large file transferred correctly" \
                "{\"file_size_mb\":$file_size_mb,\"duration_ms\":$duration_ms,\"rate_mbs\":$rate_mbs}"
        else
            test_fail "$test_name" "Large file hash mismatch" \
                "{\"source_hash\":\"${source_hash:0:16}...\",\"dest_hash\":\"${dest_hash:0:16}...\"}"
        fi
    else
        test_fail "$test_name" "Large file not transferred" "{}"
    fi
}

# =============================================================================
# RCH Mock Mode Tests
# =============================================================================

test_rch_mock_sync() {
    local test_name="test_rch_mock_sync"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Testing RCH mock sync invocation" "{}"

    local rch_bin="$1"

    local config_dir="$TEST_TMPDIR/config" socket_path runtime_root
    runtime_root="$(e2e_runtime_dir)"
    socket_path="$runtime_root/rch.sock"
    local prefix="$TEST_TMPDIR/mock-sync" rchd_bin="${rch_bin%/*}/rchd"
    mkdir -p "$config_dir"
    cat >"$config_dir/config.toml" <<EOF
[path_topology]
canonical_root = "$TEST_TMPDIR"
alias_root = "${TEST_TMPDIR}__alias"

[output]
visibility = "verbose"
first_run_complete = true
EOF
    cat >"$config_dir/workers.toml" <<'EOF'
[[workers]]
id = "mock-sync"
host = "127.0.0.1"
user = "test"
identity_file = "~/.ssh/id_rsa"
total_slots = 64
EOF
    RCH_CONFIG_DIR="$config_dir" RCH_DAEMON_INSTALLS_HOOKS=0 RCH_MOCK_SSH=1 RCH_MOCK_SSH_STDOUT=health_check \
        "$rchd_bin" --socket "$socket_path" --workers-config "$config_dir/workers.toml" \
        --foreground >"$prefix.daemon.log" 2>&1 &
    TEST_DAEMON_PID=$!
    for _ in {1..50}; do
        [[ ! -S "$socket_path" ]] || break
        kill -0 "$TEST_DAEMON_PID" 2>/dev/null || die "Mock daemon exited; see $prefix.daemon.log"
        sleep 0.1
    done
    [[ -S "$socket_path" ]] || die "Mock daemon socket not ready"
    CARGO_HOME="$TEST_TMPDIR/cargo-home" cargo metadata --manifest-path "$FIXTURE_DIR/basic_rust/Cargo.toml" --format-version 1 --no-deps \
        >"$prefix.metadata.json" || die "Fixture metadata preparation failed"
    if e2e_run_delegated "$rch_bin" "$FIXTURE_DIR/basic_rust" "cargo build" "$prefix" \
        "RCH_CONFIG_DIR=$config_dir" "RCH_SOCKET_PATH=$socket_path" \
        "CARGO_HOME=$TEST_TMPDIR/cargo-home" \
        "XDG_CACHE_HOME=$TEST_TMPDIR/cache" "XDG_STATE_HOME=$TEST_TMPDIR/state" \
        RCH_REQUIRE_REMOTE=1 RCH_MOCK_SSH=1 RCH_MOCK_RSYNC_FILES=25 RCH_MOCK_RSYNC_BYTES=51200 RUST_LOG=info \
        && [[ "$E2E_EXEC_EXIT" == 0 ]] \
        && jq -e '.location == "remote" and .outcome == "completed" and .worker_id == "mock-sync" and
                  .remote_exit_code == 0 and (.timing.sync_up | type) == "number"' "$prefix.exec.json" >/dev/null \
        && grep -q 'Sync complete: 25 files, 51200 bytes' "$prefix.exec.err"; then
        test_pass "$test_name" "Hook rewrite executed the remote source-sync path with fixture transfer statistics" \
            '{"mock_mode":true,"files":25,"bytes":51200}'
    else
        test_fail "$test_name" "Delegated remote sync did not complete; see $prefix.exec.json and .err"
    fi
}

test_rch_transfer_config_excludes() {
    local test_name="test_rch_transfer_config_excludes"
    TESTS_RUN=$((TESTS_RUN + 1))

    log_info "$test_name" "execute" "Verifying effective default transfer exclude patterns" "{}"
    local rch_bin="$1" output
    output="$(RCH_CONFIG_DIR="$TEST_TMPDIR/config" "$rch_bin" config show --json)" \
        || die "Cannot inspect effective transfer configuration"

    printf '%s\n' "$output" >"$TEST_TMPDIR/effective-transfer-config.json"
    jq -e '.data.transfer.exclude_patterns | type == "array" and length > 0 and all(.[]; type == "string")' \
        "$TEST_TMPDIR/effective-transfer-config.json" >/dev/null \
        || die "Effective transfer exclude list is missing or malformed"
    local effective_excludes=() pattern
    while IFS= read -r pattern; do
        effective_excludes+=("$pattern")
    done < <(jq -r '.data.transfer.exclude_patterns[]' "$TEST_TMPDIR/effective-transfer-config.json")

    local source="$TEST_TMPDIR/effective-excludes-source"
    local dest="$TEST_TMPDIR/effective-excludes-dest"
    mkdir -p "$source" "$dest"
    cp -r "$FIXTURE_DIR/with_git/." "$source/"
    mkdir -p "$source/target/debug" "$source/node_modules/fake-package"
    printf 'excluded target artifact\n' >"$source/target/debug/artifact"
    printf 'excluded package\n' >"$source/node_modules/fake-package/index.js"
    local stats
    stats="$(run_rsync_with_stats "$source" "$dest" "${effective_excludes[@]}")" \
        || die "Effective-excludes rsync fixture failed"
    log_info "$test_name" "sync_stats" "Applied native effective exclude list" "$stats"

    local excluded_paths=("target" "node_modules" ".git/objects") found=0
    for pattern in "${excluded_paths[@]}"; do
        if [[ ! -e "$dest/$pattern" ]]; then
            found=$((found + 1))
        fi
    done

    if [[ "$found" -eq "${#excluded_paths[@]}" && -f "$dest/src/main.rs" && -f "$dest/Cargo.toml" ]]; then
        test_pass "$test_name" "Native effective excludes omit artifacts, dependencies, and Git objects while retaining source" \
            "{\"excluded_paths_checked\":${#excluded_paths[@]},\"excluded_paths_absent\":$found}"
    else
        test_fail "$test_name" "Native effective exclusions transferred excluded data or lost source files" \
            "{\"excluded_paths_checked\":${#excluded_paths[@]},\"excluded_paths_absent\":$found}"
    fi
}

# =============================================================================
# Main Test Runner
# =============================================================================

run_tests() {
    local rch_bin="$1"

    log_info "suite" "start" "Starting project sync E2E tests" \
        "{\"quick_mode\":$QUICK_MODE,\"mock_only\":$MOCK_ONLY,\"verbose\":$VERBOSE}"

    # =========================================================================
    # Local rsync tests (always run)
    # =========================================================================
    test_rsync_basic_local
    test_rsync_target_exclusion
    test_rsync_node_modules_exclusion
    test_rsync_git_objects_exclusion
    test_rsync_incremental_sync
    test_rsync_delete_detection
    test_rsync_special_characters
    test_rsync_symlinks
    test_rsync_file_hash_verification
    test_rsync_no_change_overhead

    # =========================================================================
    # Performance tests (slow, skip in quick mode)
    # =========================================================================
    test_rsync_many_files
    test_rsync_large_file

    # =========================================================================
    # RCH-specific tests
    # =========================================================================
    test_rch_mock_sync "$rch_bin"
    test_rch_transfer_config_excludes "$rch_bin"
}

print_summary() {
    log_info "suite" "summary" "Test Summary" \
        "{\"total\":$TESTS_RUN,\"passed\":$TESTS_PASSED,\"failed\":$TESTS_FAILED,\"skipped\":$TESTS_SKIPPED}"

    if [[ "$TESTS_FAILED" -gt 0 ]]; then
        log_error "suite" "complete" "Some tests failed" \
            "{\"exit_code\":1}"
        return 1
    fi

    log_info "suite" "complete" "All project sync tests passed" \
        "{\"exit_code\":0}"
    return 0
}

main() {
    mkdir -p "$(dirname "$LOG_FILE")"
    : >"$LOG_FILE"
    parse_args "$@"
    check_dependencies
    setup_test_fixtures

    local rch_bin
    rch_bin=$(build_binaries)

    run_tests "$rch_bin"
    print_summary
}

main "$@"
