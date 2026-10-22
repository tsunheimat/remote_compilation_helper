#!/usr/bin/env bash
# install_no_downgrade_unit_test.sh - Unit tests for install.sh version-resolution safety.
#
# Regression cover for the fleet-wide downgrade of 2026-08-20: the GitHub API
# release lookup is rate limited to 60 req/hour per IP for unauthenticated
# clients. When a batch tool-update run exhausted that budget the response
# carried no "tag_name", the parse produced an empty string, and install.sh fell
# back to its built-in INSTALLER_VERSION floor - silently replacing rch 1.0.57
# with 1.0.52 on dispatchers, every nightly run.
#
# Run from project root: ./scripts/install_no_downgrade_unit_test.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
export PROJECT_ROOT
TEST_DIR=$(mktemp -d)
LOG_FILE="$TEST_DIR/install_no_downgrade_unit.log"

# shellcheck disable=SC1091
source "$SCRIPT_DIR/test_lib.sh"
init_test_log "$(basename "${BASH_SOURCE[0]}" .sh)"

TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0

log() {
    echo "[$(date -Iseconds)] $*" | tee -a "$LOG_FILE"
    log_json execute "$*"
}
pass() { TESTS_PASSED=$((TESTS_PASSED + 1)); log "PASS: $1"; }
fail() { TESTS_FAILED=$((TESTS_FAILED + 1)); log "FAIL: $1"; }
start_test() { TESTS_RUN=$((TESTS_RUN + 1)); log "Test $TESTS_RUN: $1"; }
cleanup() { log "Fixture files retained (no deletion): $TEST_DIR"; }
trap '_test_lib_cleanup; cleanup' EXIT

log "=== RCH Installer No-Downgrade Unit Tests ==="

if [[ ! -f "$PROJECT_ROOT/install.sh" ]]; then
    log "ERROR: install.sh not found at $PROJECT_ROOT/install.sh"
    test_fail "install.sh missing"
fi

export RCH_INSTALLER_LIB=1
# shellcheck disable=SC1090
source "$PROJECT_ROOT/install.sh"

USE_GUM=false
USE_COLOR=false

# Silence the guard's advisory output during tests
warn() { :; }
info() { :; }

# Build a fake rch that reports a chosen version
make_fake_rch() {
    local dir="$1" ver="$2"
    mkdir -p "$dir"
    cat > "$dir/rch" <<EOF
#!/usr/bin/env bash
echo "rch $ver (commit deadbeef)"
EOF
    chmod +x "$dir/rch"
}

# ---------------------------------------------------------------- version_gt
start_test "version_gt orders release versions correctly"
vg_ok=true
version_gt "1.0.57" "1.0.52" || vg_ok=false      # newer > older
version_gt "1.0.52" "1.0.57" && vg_ok=false      # older is not > newer
version_gt "1.0.57" "1.0.57" && vg_ok=false      # equal is not >
version_gt "1.0.10" "1.0.9"  || vg_ok=false      # numeric, not lexical
if $vg_ok; then pass "version_gt semantics"; else fail "version_gt semantics"; fi

# ------------------------------------------------- guard blocks the downgrade
start_test "stale fallback does NOT replace a newer install"
INSTALL_DIR="$TEST_DIR/case1"; make_fake_rch "$INSTALL_DIR" "1.0.57"
VERSION="1.0.52"; VERSION_EXPLICIT=false; SKIP_INSTALL_NO_DOWNGRADE=false
OFFLINE_TARBALL=""; FROM_SOURCE=false
enforce_no_downgrade
if [[ "$VERSION" == "1.0.57" && "$SKIP_INSTALL_NO_DOWNGRADE" == "true" ]]; then
    pass "downgrade 1.0.57 -> 1.0.52 refused"
else
    fail "downgrade refused (VERSION=$VERSION SKIP=$SKIP_INSTALL_NO_DOWNGRADE)"
fi

# ------------------------------------------------------ upgrades still happen
start_test "genuine upgrade is allowed"
INSTALL_DIR="$TEST_DIR/case2"; make_fake_rch "$INSTALL_DIR" "1.0.52"
VERSION="1.0.57"; VERSION_EXPLICIT=false; SKIP_INSTALL_NO_DOWNGRADE=false
enforce_no_downgrade
if [[ "$VERSION" == "1.0.57" && "$SKIP_INSTALL_NO_DOWNGRADE" == "false" ]]; then
    pass "upgrade 1.0.52 -> 1.0.57 allowed"
else
    fail "upgrade allowed (VERSION=$VERSION SKIP=$SKIP_INSTALL_NO_DOWNGRADE)"
fi

# -------------------------------------------- explicit pin overrides the guard
start_test "explicitly requested version still permits rollback"
INSTALL_DIR="$TEST_DIR/case3"; make_fake_rch "$INSTALL_DIR" "1.0.57"
VERSION="1.0.52"; VERSION_EXPLICIT=true; SKIP_INSTALL_NO_DOWNGRADE=false
enforce_no_downgrade
if [[ "$VERSION" == "1.0.52" && "$SKIP_INSTALL_NO_DOWNGRADE" == "false" ]]; then
    pass "explicit rollback honoured"
else
    fail "explicit rollback honoured (VERSION=$VERSION SKIP=$SKIP_INSTALL_NO_DOWNGRADE)"
fi

# ------------------------------------------------------- fresh install is a no-op
start_test "no existing install leaves target untouched"
INSTALL_DIR="$TEST_DIR/case4-empty"; mkdir -p "$INSTALL_DIR"
VERSION="1.0.52"; VERSION_EXPLICIT=false; SKIP_INSTALL_NO_DOWNGRADE=false
enforce_no_downgrade
if [[ "$VERSION" == "1.0.52" && "$SKIP_INSTALL_NO_DOWNGRADE" == "false" ]]; then
    pass "fresh install unaffected"
else
    fail "fresh install unaffected (VERSION=$VERSION SKIP=$SKIP_INSTALL_NO_DOWNGRADE)"
fi

# Real retained-byte/source tests remain alongside the request doubles below.
# A public upstream release floor is not the release authority for this fork;
# exact requested tags, no downgrade and invalid-source refusal are checked here.
start_test "release URLs remain on the requested tag for every archive name"
url_ok=true
for version in 2.1.16 v2.1.16 2.1.17-rc.1; do
    tag=$(installer_release_tag "$version") || url_ok=false
    for target in x86_64-unknown-linux-musl x86_64-unknown-linux-gnu aarch64-apple-darwin x86_64-pc-windows-msvc linux-x86_64 darwin-aarch64 windows-x86_64; do
        for asset in "rch-${tag}-${target}.tar.gz" "rch-${target}.tar.gz" "rch-${target}.zip"; do
            actual=$(release_download_url "$tag" "$asset") || url_ok=false
            [[ "$actual" == "https://github.com/${GITHUB_REPO}/releases/download/${tag}/${asset}" ]] || url_ok=false
            [[ "$actual" != *"/latest/"* ]] || url_ok=false
        done
    done
done
for invalid in '' '2.1.16/other' '2.1.16?query' '2.1.16 with-space' 'main'; do
    if installer_release_tag "$invalid"; then url_ok=false; fi
done
if release_download_url v2.1.16 ../another; then url_ok=false; fi
if $url_ok; then pass "exact release tag and asset URL binding"; else fail "exact release tag and asset URL binding"; fi

start_test "real checksum files require one exact valid archive entry"
artifact="$TEST_DIR/rch-v2.1.16-x86_64-unknown-linux-gnu.tar.gz"
printf 'retained archive-byte fixture\n' > "$artifact"
if command_exists sha256sum; then
    digest=$(sha256sum "$artifact" | awk '{print $1}')
else
    digest=$(shasum -a 256 "$artifact" | awk '{print $1}')
fi
checksum_ok=true
name=$(basename "$artifact")
for prefix in '' '*' './' 'dist/'; do
    checksum_file="$TEST_DIR/checksum-${prefix//[^A-Za-z0-9]/x}-${#prefix}.txt"
    printf '%s  %s%s\r\n' "$digest" "$prefix" "$name" > "$checksum_file"
    verify_checksum "$artifact" "$checksum_file" "$name" || checksum_ok=false
done
sidecar=$(extract_checksum_from_manifest "$digest" "$name" true) || checksum_ok=false
[[ "$sidecar" == "$digest" ]] || checksum_ok=false
sidecar=$(extract_checksum_from_manifest "$(printf '%s\r\n\r\n' "$digest")" "$name" true) || checksum_ok=false
[[ "$sidecar" == "$digest" ]] || checksum_ok=false
upper=$(printf '%s' "$digest" | tr '[:lower:]' '[:upper:]')
[[ "$(extract_checksum_from_manifest "$upper  $name" "$name")" == "$digest" ]] || checksum_ok=false
index=0
for invalid in \
    "$digest  ${name}.another" \
    "$digest  ${name//./x}" \
    "not-a-digest  $name" \
    "$digest  $name extra-field" \
    "$(printf '%s  %s\n%s  %s' "$digest" "$name" "$digest" "$name")" \
    ""; do
    index=$((index + 1))
    checksum_file="$TEST_DIR/invalid-checksum-${index}.txt"
    printf '%s\n' "$invalid" > "$checksum_file"
    if verify_checksum "$artifact" "$checksum_file" "$name"; then checksum_ok=false; fi
done
if extract_checksum_from_manifest "$(printf '%s\n%s' "$digest" "$digest")" "$name" true; then checksum_ok=false; fi
if extract_checksum_from_manifest "$(printf '%s  unrelated\n%s  %s' "$digest" "$digest" "$name")" "$name" true; then checksum_ok=false; fi
mismatch="$TEST_DIR/mismatch-checksum.txt"
printf '%064d  %s\n' 0 "$name" > "$mismatch"
if verify_checksum "$artifact" "$mismatch" "$name"; then checksum_ok=false; fi
if command_exists sha256sum; then
    after=$(sha256sum "$artifact" | awk '{print $1}')
else
    after=$(shasum -a 256 "$artifact" | awk '{print $1}')
fi
[[ "$after" == "$digest" ]] || checksum_ok=false
if $checksum_ok; then pass "actual digest verification and preserved archive bytes"; else fail "actual digest verification and preserved archive bytes"; fi

start_test "actual source checkout must equal the requested release tag"
source_root="$TEST_DIR/release-source"
git init --initial-branch=main "$source_root"
git -C "$source_root" -c user.name=InstallerTest -c user.email=installer-test@example.invalid \
    commit --allow-empty -m 'retained release fixture'
git -C "$source_root" tag v2.1.16
git -C "$source_root" -c user.name=InstallerTest -c user.email=installer-test@example.invalid \
    tag -a v2.1.17 -m 'retained annotated release fixture'
source_ok=true
verify_source_release "$source_root" v2.1.16 || source_ok=false
verify_source_release "$source_root" v2.1.17 || source_ok=false
if verify_source_release "$source_root" v9.9.9; then source_ok=false; fi
printf 'retained untracked source fixture\n' > "$source_root/new-source.rs"
if verify_source_release "$source_root" v2.1.16; then source_ok=false; fi
git -C "$source_root" add -- new-source.rs
if verify_source_release "$source_root" v2.1.16; then source_ok=false; fi
git -C "$source_root" -c user.name=InstallerTest -c user.email=installer-test@example.invalid \
    commit -m 'retained different checkout fixture'
head_before=$(git -C "$source_root" rev-parse HEAD)
if verify_source_release "$source_root" v2.1.16; then source_ok=false; fi
[[ "$(git -C "$source_root" rev-parse HEAD)" == "$head_before" ]] || source_ok=false
if $source_ok; then pass "actual tag and checkout comparison, no source mutation"; else fail "actual tag and checkout comparison, no source mutation"; fi

# Hermetic FIXTURE evidence only: no release, network, or signer is live here.
# Block all network/build/install/deletion side effects, even on regressions.
curl() {
    local url="" output="" arg
    while (( $# )); do
        arg="$1"; shift
        case "$arg" in
            -o) output="$1"; shift ;;
            -A) shift ;;
            https://*) url="$arg" ;;
        esac
    done
    printf '%s\n' "$url" >> "$CASE_DIR/urls"
    case "$url" in
        */latest*) return 99 ;;
        *.sigstore.json)
            [[ "$SCENARIO" != missing_bundle ]] || return 22
            printf 'fixture bundle\n' > "$output" ;;
        *.sha256)
            case "$SCENARIO" in
                missing|manifest|manifest_duplicate) return 22 ;;
                malformed) printf 'garbage  %s\n' "$ASSET" ;;
                mismatch) printf '%064d  %s\n' 0 "$ASSET" ;;
                wrong_name) printf '%s  other.tar.gz\n' "$DIGEST" ;;
                dist_prefix) printf '%s  dist/%s\n' "$DIGEST" "$ASSET" ;;
                *) printf '%s  %s\n' "$DIGEST" "$ASSET" ;;
            esac ;;
        */SHA256SUMS)
            [[ "$SCENARIO" != missing ]] || return 22
            printf '%s  %s\n' "$DIGEST" "$ASSET"
            [[ "$SCENARIO" != manifest_duplicate ]] || printf '%s  %s\n' "$DIGEST" "$ASSET" ;;
        *.tar.gz)
            [[ "$SCENARIO" != unavailable ]] || return 22
            if [[ "$SCENARIO" == legacy || "$SCENARIO" == legacy_triple ]]; then
                [[ "${url##*/}" == "$ASSET" ]] || return 22
            fi
            printf 'fixture archive\n' > "$output" ;;
        *) return 99 ;;
    esac
    return 0
}
cleanup_lock() { :; }
rm() { printf 'forbidden deletion\n' >&2; return 99; }
# Ignore installer cleanup traps in child cases; retain every fixture directory.
trap() { :; }
mktemp() { command mktemp -d "$CASE_DIR/temp.XXXXXX"; }
tar() {
    printf 'extract\n' >> "$CASE_DIR/events"
    local dest="${@: -1}"
    printf 'fixture worker\n' > "$dest/rch-wkr"
}
unzip() { printf 'unexpected unzip\n' >> "$CASE_DIR/events"; return 99; }
install_binary_file() { printf 'install\n' >> "$CASE_DIR/events"; }
cargo() { printf 'build\n' >> "$CASE_DIR/events"; return 99; }
install_rust_toolchain() { return 99; }
git() {
    printf '%s\n' "$*" >> "$CASE_DIR/git"
    if [[ "$1" == clone ]]; then
        mkdir -p "${@: -1}"
    elif [[ "$SCENARIO" == branch_only ]]; then
        return 1
    fi
}
cosign() {
    printf '%s\n' "$*" >> "$CASE_DIR/cosign"
    [[ "$SCENARIO" != bad_signature ]]
}
command_exists() {
    if [[ "$1" == cosign && "$SCENARIO" == no_cosign ]]; then return 1; fi
    command -v "$1" >/dev/null 2>&1
}

DIGEST=$(printf 'fixture archive\n' | sha256sum)
DIGEST="${DIGEST%% *}"
ASSET="rch-v1.2.3-x86_64-unknown-linux-gnu.tar.gz"
VERSION=1.2.3
VERSION_EXPLICIT=true
TARGET=linux-x86_64
RELEASE_TARGETS=(x86_64-unknown-linux-gnu)
RELEASE_ARCHIVE_EXT=tar.gz
MODE=worker
SIGNATURE_POLICY=optional

run_release_case() {
    local scenario="$1" policy="$2" want_success="$3"
    start_test "FIXTURE release $scenario ($policy)"
    CASE_DIR="$TEST_DIR/$scenario-$policy"
    mkdir "$CASE_DIR"
    SCENARIO="$scenario"; SIGNATURE_POLICY="$policy"
    ASSET="rch-v1.2.3-x86_64-unknown-linux-gnu.tar.gz"
    [[ "$scenario" != legacy ]] || ASSET="rch-linux-x86_64.tar.gz"
    [[ "$scenario" != legacy_triple ]] || ASSET="rch-x86_64-unknown-linux-gnu.tar.gz"
    local result=0
    # Do not put the function in an if/! context: preserve production errexit.
    set +e
    (set -e; download_binaries) > "$CASE_DIR/output" 2>&1
    result=$?
    set -e
    if { [[ "$want_success" == true && "$result" == 0 ]] &&
         [[ "$(<"$CASE_DIR/events")" == $'extract\ninstall' ]]; } ||
       { [[ "$want_success" == false && "$result" != 0 ]] &&
         [[ ! -f "$CASE_DIR/events" ]]; }; then
        pass "release $scenario gate"
    else
        fail "release $scenario gate (exit $result); see $CASE_DIR/output"
    fi
    if grep -q '/latest' "$CASE_DIR/urls"; then fail "explicit version requested latest"; fi
    if [[ "$scenario" != unavailable && -f "$CASE_DIR/git" ]]; then fail "integrity failure reached source fallback"; fi
}

for scenario in valid manifest dist_prefix legacy legacy_triple; do run_release_case "$scenario" optional true; done
run_release_case unavailable required false
for scenario in missing malformed mismatch wrong_name manifest_duplicate bad_signature; do
    run_release_case "$scenario" optional false
done
for scenario in missing_bundle no_cosign; do
    run_release_case "$scenario" optional true
    run_release_case "$scenario" required false
done
run_release_case valid required true
start_test "FIXTURE exact fork signer and GitHub issuer"
if grep -Fq -- '--certificate-identity=https://github.com/tsunheimat/remote_compilation_helper/.github/workflows/release.yml@refs/tags/v1.2.3' "$CASE_DIR/cosign" &&
   grep -Fq -- '--certificate-oidc-issuer=https://token.actions.githubusercontent.com' "$CASE_DIR/cosign" &&
   ! grep -q regexp "$CASE_DIR/cosign"; then
    pass "anchored signer arguments"
else
    fail "anchored signer arguments"
fi

start_test "FIXTURE source fallback pins tag and rejects branch look-alikes"
CASE_DIR="$TEST_DIR/source"; mkdir "$CASE_DIR"
SCENARIO=branch_only; SIGNATURE_POLICY=optional
set +e
(set -e; clone_and_build_from_source) > "$CASE_DIR/output" 2>&1
source_result=$?
set -e
if [[ "$source_result" != 0 && ! -f "$CASE_DIR/events" ]] &&
   grep -Fq -- '--branch v1.2.3 -- https://github.com/tsunheimat/remote_compilation_helper' "$CASE_DIR/git" &&
   grep -Fq -- 'checkout --detach refs/tags/v1.2.3' "$CASE_DIR/git"; then
    pass "source tag required before build"
else fail "source pin"; fi

start_test "FIXTURE implicit source fallback is refused without git or build"
CASE_DIR="$TEST_DIR/implicit"; mkdir "$CASE_DIR"
VERSION_EXPLICIT=false
set +e
(set -e; clone_and_build_from_source) > "$CASE_DIR/output" 2>&1
source_result=$?
set -e
if [[ "$source_result" != 0 && ! -f "$CASE_DIR/git" && ! -f "$CASE_DIR/events" ]]; then
    pass "implicit source fallback refused"
else fail "implicit source fallback refused"; fi

for scenario in missing malformed mismatch valid; do
    start_test "FIXTURE offline checksum $scenario"
    CASE_DIR="$TEST_DIR/offline-$scenario"; mkdir "$CASE_DIR"
    SCENARIO=no_cosign; SIGNATURE_POLICY=optional
    printf 'fixture archive\n' > "$CASE_DIR/archive.tar.gz"
    case "$scenario" in
        missing) ;;
        malformed) printf 'bad  archive.tar.gz\n' > "$CASE_DIR/archive.tar.gz.sha256" ;;
        mismatch) printf '%064d  archive.tar.gz\n' 0 > "$CASE_DIR/archive.tar.gz.sha256" ;;
        valid) printf '%s  archive.tar.gz\n' "$DIGEST" > "$CASE_DIR/archive.tar.gz.sha256" ;;
    esac
    set +e
    (set -e; install_from_tarball "$CASE_DIR/archive.tar.gz") > "$CASE_DIR/output" 2>&1
    result=$?
    set -e
    if { [[ "$scenario" == valid && "$result" == 0 ]] &&
         [[ "$(<"$CASE_DIR/events")" == $'extract\ninstall' ]]; } ||
       { [[ "$scenario" != valid && "$result" != 0 && ! -f "$CASE_DIR/events" ]]; }; then
        pass "offline checksum $scenario"
    else fail "offline checksum $scenario"; fi
done

for release_version in 1.2.3-rc1 1.2.3-rc.1-test 1.2.3; do
    start_test "FIXTURE offline auto-detect and exact signer tag $release_version"
    CASE_DIR="$TEST_DIR/offline-tag-$release_version"; mkdir "$CASE_DIR"
    SCENARIO=valid; SIGNATURE_POLICY=required
    OFFLINE_TARBALL="$CASE_DIR/rch-v${release_version}-x86_64-unknown-linux-gnu.tar.gz"
    printf 'fixture archive\n' > "$OFFLINE_TARBALL"
    printf '%s  %s\n' "$DIGEST" "${OFFLINE_TARBALL##*/}" > "$OFFLINE_TARBALL.sha256"
    printf 'fixture bundle\n' > "$OFFLINE_TARBALL.sigstore.json"
    set +e
    (
        set -e
        VERSION=""; VERSION_EXPLICIT=false
        # Run from the real project root: its Cargo version must not override
        # the identity of the explicitly selected offline archive.
        cd "$PROJECT_ROOT"
        detect_target_version
        [[ "$VERSION" == "$release_version" && "$VERSION_EXPLICIT" == false ]]
        install_from_tarball "$OFFLINE_TARBALL"
    ) > "$CASE_DIR/output" 2>&1
    result=$?
    set -e
    if [[ "$result" == 0 && ! -f "$CASE_DIR/urls" ]] &&
       grep -Fq -- "--certificate-identity=https://github.com/tsunheimat/remote_compilation_helper/.github/workflows/release.yml@refs/tags/v${release_version} " "$CASE_DIR/cosign"; then
        pass "offline tag and signer $release_version"
    else fail "offline tag and signer $release_version"; fi
done

for filename in archive.tar.gz rch-v1.2.3-unknown-target.tar.gz; do
    start_test "FIXTURE ambiguous offline filename requires explicit version: $filename"
    CASE_DIR="$TEST_DIR/ambiguous-$filename"; mkdir "$CASE_DIR"
    SCENARIO=valid; SIGNATURE_POLICY=optional
    OFFLINE_TARBALL="$CASE_DIR/$filename"
    set +e
    (set -e; VERSION=""; detect_target_version) > "$CASE_DIR/output" 2>&1
    result=$?
    set -e
    if [[ "$result" != 0 && ! -f "$CASE_DIR/urls" ]] &&
       grep -q 'supply --version or VERSION' "$CASE_DIR/output" &&
       (VERSION=1.2.3-rc1; detect_target_version; [[ "$VERSION" == 1.2.3-rc1 && "$VERSION_EXPLICIT" == true ]]); then
        pass "ambiguous offline identity fails closed; explicit version preserved"
    else fail "ambiguous offline identity"; fi
done
OFFLINE_TARBALL=""

start_test "FIXTURE repository override validation"
if (GITHUB_REPO=other-owner/repository; validate_release_configuration) &&
   ! (GITHUB_REPO='owner/repo/../../other'; validate_release_configuration) 2>/dev/null; then
    pass "repository syntax constrained"
else fail "repository syntax constrained"; fi

log "=== Results: $TESTS_PASSED passed, $TESTS_FAILED failed, $TESTS_RUN run ==="
[[ $TESTS_FAILED -eq 0 ]] || exit 1
exit 0
