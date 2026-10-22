#!/usr/bin/env bash
# install.sh - Remote Compilation Helper Installer
#
# A modern, polished installer with Gum UI (with ANSI fallback), checksum
# verification, proxy support, offline mode, and comprehensive toolchain verification.
#
# Installation modes:
#   1. Local:       Install rch + rchd on current machine (default)
#   2. Worker:      Install just rch-wkr on current machine
#
# Usage:
#   bash install.sh                      # Install + optionally enable background service
#   bash install.sh --local              # Same as above (default mode)
#   bash install.sh --worker             # Worker-only install (no daemon)
#   bash install.sh --from-source        # Build from source
#   bash install.sh --easy-mode          # Configure PATH + detect agents + run doctor
#   bash install.sh --no-service         # Install without system service
#   bash install.sh --offline <tarball>  # Install from local tarball
#   bash install.sh --uninstall          # Remove RCH
#
# Environment:
#   RCH_INSTALL_DIR     Where to install binaries (default: ~/.local/bin)
#   RCH_CONFIG_DIR      Where to store config (default: ~/.config/rch)
#   RCH_NO_HOOK         Skip Claude Code hook setup if set
#   RCH_SKIP_FLEET_SYNC Skip worker deploy/health checks after install
#   RCH_SKIP_DAEMON_RESTART Skip rchd restart after install
#   RCH_NO_COLOR        Disable colored output
#   RCH_SKIP_DOCTOR     Skip post-install doctor check
#   HTTP_PROXY          HTTP proxy URL
#   HTTPS_PROXY         HTTPS proxy URL
#   NO_PROXY            Comma-separated list of hosts to bypass proxy

set -euo pipefail

# ============================================================================
# Configuration
# ============================================================================

# Last-resort floor used only when every release-discovery path fails. Keep it in
# step with the newest published release; enforce_no_downgrade() stops a stale
# value here from clobbering a newer install.
INSTALLER_VERSION="2.1.17"
VERSION="${VERSION:-}"  # Set dynamically based on install mode
# Records whether the caller pinned a version (VERSION=... or --version), which
# suppresses the no-downgrade guard so deliberate rollbacks still work.
VERSION_EXPLICIT=false
SKIP_INSTALL_NO_DOWNGRADE=false
# An override is an explicit choice of release publisher, not a download URL.
GITHUB_REPO="${RCH_REPOSITORY:-tsunheimat/remote_compilation_helper}"
REPO_URL="https://github.com/${GITHUB_REPO}"
GITHUB_API="https://api.github.com/repos/${GITHUB_REPO}"
SIGNATURE_POLICY="${RCH_SIGNATURE_POLICY:-optional}"
DOWNLOAD_USER_AGENT="OpenAI File Downloader, XaiImageApiFetch/1.0"

validate_release_configuration() {
    [[ "$GITHUB_REPO" =~ ^[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9][A-Za-z0-9._-]*$ ]] ||
        die "RCH_REPOSITORY must be a GitHub owner/repository"
    case "$SIGNATURE_POLICY" in
        optional|required) ;;
        *) die "RCH_SIGNATURE_POLICY must be optional or required" ;;
    esac
    if [[ -n "${VERSION:-}" ]]; then
        [[ "$VERSION" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([-+][A-Za-z0-9.-]+)?$ ]] ||
            die "Invalid release version: $VERSION"
    fi
}

INSTALL_DIR="${RCH_INSTALL_DIR:-$HOME/.local/bin}"
CONFIG_DIR="${RCH_CONFIG_DIR:-$HOME/.config/rch}"
SOCKET_PATH="/tmp/rch.sock"
LOCK_FILE="/tmp/rch-install.lock"
RUNTIME_DIR="${RCH_RUNTIME_DIR:-}"

# Binaries
HOOK_BIN="rch"
DAEMON_BIN="rchd"
WORKER_BIN="rch-wkr"

# State variables
USE_COLOR=true
USE_GUM=false
TEMP_DIR=""
PROXY_ARGS=()

# Default color values (set properly by setup_ui)
RED='' GREEN='' YELLOW='' BLUE='' CYAN='' BOLD='' DIM='' NC=''

# ============================================================================
# Terminal Detection and UI Setup
# ============================================================================

setup_ui() {
    # Detect terminal capabilities
    if [[ -t 1 ]] && [[ -z "${RCH_NO_COLOR:-}" ]] && [[ "${TERM:-dumb}" != "dumb" ]]; then
        USE_COLOR=true
    else
        USE_COLOR=false
    fi

    # Check for Gum
    if command -v gum >/dev/null 2>&1 && [[ -z "${NO_GUM:-}" ]]; then
        USE_GUM=true
    else
        USE_GUM=false
    fi

    # ANSI color codes (fallback)
    if $USE_COLOR; then
        RED='\033[0;31m'
        GREEN='\033[0;32m'
        YELLOW='\033[1;33m'
        BLUE='\033[0;34m'
        CYAN='\033[0;36m'
        BOLD='\033[1m'
        DIM='\033[2m'
        NC='\033[0m' # No Color
    else
        RED='' GREEN='' YELLOW='' BLUE='' CYAN='' BOLD='' DIM='' NC=''
    fi
}

# ============================================================================
# Output Functions (with Gum support)
# ============================================================================

info() {
    if $USE_GUM; then
        gum style --foreground 212 "→ $*"
    else
        echo -e "${BLUE}→${NC} $*"
    fi
}

success() {
    if $USE_GUM; then
        gum style --foreground 82 "✓ $*"
    else
        echo -e "${GREEN}✓${NC} $*"
    fi
}

warn() {
    if $USE_GUM; then
        gum style --foreground 208 "⚠ $*"
    else
        echo -e "${YELLOW}⚠${NC} $*" >&2
    fi
}

error() {
    if $USE_GUM; then
        gum style --foreground 196 "✗ $*"
    else
        echo -e "${RED}✗${NC} $*" >&2
    fi
}

die() {
    error "$@"
    cleanup_lock
    exit 1
}

# Spinner wrapper for long-running operations
spin() {
    local title="$1"
    shift
    if $USE_GUM; then
        gum spin --spinner dot --title "$title" -- "$@"
    else
        info "$title"
        "$@"
    fi
}

# Confirmation prompt
confirm() {
    local prompt="$1"
    if [[ "${YES:-}" == "true" ]]; then
        return 0
    fi
    if $USE_GUM; then
        gum confirm "$prompt"
    else
        read -rp "$prompt [y/N] " response
        [[ "$response" =~ ^[Yy] ]]
    fi
}

# Return a width estimate for a line that won't undercount emoji/wide chars.
# We intentionally count bytes (C locale) to avoid too-narrow boxes.
line_width() {
    local line="$1"
    LC_ALL=C printf '%s' "$line" | wc -c | tr -d ' '
}

# Draw a Unicode box with auto-calculated width (DCG-style)
# Usage: draw_box "color_code" "line1" "line2" ...
draw_box() {
    local color="$1"
    shift
    local lines=("$@")
    local max_len=0

    # Calculate maximum line length
    for line in "${lines[@]}"; do
        local len
        len=$(line_width "$line")
        [[ $len -gt $max_len ]] && max_len=$len
    done

    # Add padding
    local width=$((max_len + 4))
    local inner_width=$((width - 2))

    # Box characters
    local tl="╭" tr="╮" bl="╰" br="╯" h="─" v="│"

    # Top border
    local top_border="${tl}"
    for ((i=0; i<inner_width; i++)); do top_border+="${h}"; done
    top_border+="${tr}"

    # Bottom border
    local bottom_border="${bl}"
    for ((i=0; i<inner_width; i++)); do bottom_border+="${h}"; done
    bottom_border+="${br}"

    # Print box
    echo ""
    if $USE_COLOR; then
        echo -e "\033[${color}m${top_border}\033[0m"
        for line in "${lines[@]}"; do
            local padding=$((inner_width - $(line_width "$line") - 2))
            local pad_str=""
            for ((i=0; i<padding; i++)); do pad_str+=" "; done
            echo -e "\033[${color}m${v}\033[0m ${line}${pad_str} \033[${color}m${v}\033[0m"
        done
        echo -e "\033[${color}m${bottom_border}\033[0m"
    else
        echo "${top_border}"
        for line in "${lines[@]}"; do
            local padding=$((inner_width - $(line_width "$line") - 2))
            local pad_str=""
            for ((i=0; i<padding; i++)); do pad_str+=" "; done
            echo "${v} ${line}${pad_str} ${v}"
        done
        echo "${bottom_border}"
    fi
    echo ""
}

# Draw a Unicode double-line box with auto-calculated width
# Usage: draw_double_box "color_code" "line1" "line2" ...
draw_double_box() {
    local color="$1"
    shift
    local lines=("$@")
    local max_len=0

    for line in "${lines[@]}"; do
        local len
        len=$(line_width "$line")
        [[ $len -gt $max_len ]] && max_len=$len
    done

    local width=$((max_len + 4))
    local inner_width=$((width - 2))

    local tl="╔" tr="╗" bl="╚" br="╝" h="═" v="║"

    local top_border="${tl}"
    for ((i=0; i<inner_width; i++)); do top_border+="${h}"; done
    top_border+="${tr}"

    local bottom_border="${bl}"
    for ((i=0; i<inner_width; i++)); do bottom_border+="${h}"; done
    bottom_border+="${br}"

    if $USE_COLOR; then
        echo -e "\033[${color}m${top_border}\033[0m"
        for line in "${lines[@]}"; do
            local padding=$((inner_width - $(line_width "$line") - 2))
            local pad_str=""
            for ((i=0; i<padding; i++)); do pad_str+=" "; done
            echo -e "\033[${color}m${v}\033[0m ${line}${pad_str} \033[${color}m${v}\033[0m"
        done
        echo -e "\033[${color}m${bottom_border}\033[0m"
    else
        echo "${top_border}"
        for line in "${lines[@]}"; do
            local padding=$((inner_width - $(line_width "$line") - 2))
            local pad_str=""
            for ((i=0; i<padding; i++)); do pad_str+=" "; done
            echo "${v} ${line}${pad_str} ${v}"
        done
        echo "${bottom_border}"
    fi
}

# Header display with full branding
show_header() {
    echo ""
    if $USE_GUM; then
        gum style \
            --border double \
            --border-foreground 212 \
            --padding "1 3" \
            --align center \
            "⚡ RCH Installer v$VERSION" \
            "" \
            "Remote Compilation Helper" \
            "Transparent compilation offloading for AI agents"
    else
        local header_color="1;35"  # Bold magenta
        echo -e "\033[${header_color}m"
        cat << 'BANNER'
    ██████╗  ██████╗██╗  ██╗
    ██╔══██╗██╔════╝██║  ██║
    ██████╔╝██║     ███████║
    ██╔══██╗██║     ██╔══██║
    ██║  ██║╚██████╗██║  ██║
    ╚═╝  ╚═╝ ╚═════╝╚═╝  ╚═╝
BANNER
        echo -e "\033[0m"
        draw_box "1;35" \
            "RCH Installer v$VERSION" \
            "Remote Compilation Helper" \
            "Transparent compilation offloading for AI agents"
    fi
}

command_exists() {
    command -v "$1" &> /dev/null
}

is_interactive() {
    [[ -t 0 ]]
}

systemd_user_available() {
    if ! command_exists systemctl; then
        return 1
    fi
    if systemctl --user show-environment >/dev/null 2>&1; then
        return 0
    fi
    if systemctl --user is-system-running >/dev/null 2>&1; then
        return 0
    fi
    return 1
}

system_rchd_service_active() {
    if ! command_exists systemctl; then
        return 1
    fi

    systemctl is-active --quiet rchd.service >/dev/null 2>&1
}

detect_service_manager() {
    local os
    os="$(uname -s)"
    case "$os" in
        Linux*)
            if systemd_user_available; then
                echo "systemd"
                return 0
            fi
            ;;
        Darwin*)
            if command_exists launchctl; then
                echo "launchd"
                return 0
            fi
            ;;
        *)
            ;;
    esac
    return 1
}

ensure_command() {
    if ! command_exists "$1"; then
        die "Required command not found: $1. Please install it first."
    fi
}

# ============================================================================
# Lock File Management
# ============================================================================

acquire_lock() {
    if [[ -f "$LOCK_FILE" ]]; then
        local pid
        pid=$(cat "$LOCK_FILE" 2>/dev/null || echo "")
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            die "Another installation is in progress (PID: $pid). If this is incorrect, remove $LOCK_FILE"
        fi
        # Stale lock file, remove it
        rm -f "$LOCK_FILE"
    fi
    echo $$ > "$LOCK_FILE"
    trap cleanup_lock EXIT
}

cleanup_lock() {
    rm -f "$LOCK_FILE"
}

# ============================================================================
# Platform Detection
# ============================================================================

detect_platform() {
    info "Detecting platform..."

    local os arch

    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="darwin" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *)       die "Unsupported OS: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)             die "Unsupported architecture: $(uname -m)" ;;
    esac

    # WSL detection
    if [[ "$os" == "linux" ]] && grep -qi microsoft /proc/version 2>/dev/null; then
        warn "WSL detected. Some features may require additional configuration:"
        echo "  - SSH agent forwarding may need special setup"
        echo "  - File permissions between Windows/Linux may cause issues"
        echo "  - Consider using native Linux for best performance"
    fi

    TARGET="${os}-${arch}"

    # Map to release target triples used by GitHub Actions artifacts.
    # RELEASE_TARGETS is an ordered candidate list: the download step tries each
    # in turn and uses the first asset that actually exists in the release. This
    # matters for linux-x86_64, where the preferred fully-static -musl asset is
    # not always published (e.g. a dsr fallback release that cut only -gnu);
    # falling back to -gnu keeps install working whenever either asset exists
    # (rch#23).
    RELEASE_ARCHIVE_EXT="tar.gz"
    RELEASE_TARGETS=()
    case "${os}-${arch}" in
        linux-x86_64)   RELEASE_TARGETS=("x86_64-unknown-linux-musl" "x86_64-unknown-linux-gnu") ;;
        linux-aarch64)  RELEASE_TARGETS=("aarch64-unknown-linux-gnu") ;;
        darwin-x86_64)  RELEASE_TARGETS=("x86_64-apple-darwin") ;;
        darwin-aarch64) RELEASE_TARGETS=("aarch64-apple-darwin") ;;
        windows-x86_64) RELEASE_TARGETS=("x86_64-pc-windows-msvc"); RELEASE_ARCHIVE_EXT="zip" ;;
    esac
    if [[ ${#RELEASE_TARGETS[@]} -eq 0 ]]; then
        RELEASE_TARGETS=("$TARGET")
    fi

    success "Platform: $TARGET"
}

# ============================================================================
# Proxy Setup
# ============================================================================

setup_proxy() {
    PROXY_ARGS=()
    if [[ -n "${HTTPS_PROXY:-}" ]]; then
        PROXY_ARGS=(--proxy "$HTTPS_PROXY")
        info "Using HTTPS proxy: $HTTPS_PROXY"
    elif [[ -n "${HTTP_PROXY:-}" ]]; then
        PROXY_ARGS=(--proxy "$HTTP_PROXY")
        info "Using HTTP proxy: $HTTP_PROXY"
    fi
}

# ============================================================================
# Prerequisite Checks
# ============================================================================

check_prerequisites() {
    info "Checking prerequisites..."

    local errors=0

    # Required for building from source
    if [[ "${FROM_SOURCE:-false}" == "true" ]]; then
        if command_exists cargo; then
            success "cargo: $(cargo --version)"
        else
            error "cargo not found (required for source build)"
            ((errors++))
        fi

        if command_exists rustc; then
            local rustc_version
            rustc_version=$(rustc --version)
            if echo "$rustc_version" | grep -q nightly; then
                success "rustc: $rustc_version"
            else
                warn "rustc: $rustc_version (nightly recommended)"
            fi
        else
            error "rustc not found (required for source build)"
            ((errors++))
        fi
    fi

    # Required for operation
    if command_exists rsync; then
        success "rsync: available"
    else
        error "rsync not found"
        echo "  Install with: apt install rsync / brew install rsync"
        ((errors++))
    fi

    if command_exists ssh; then
        success "ssh: available"
    else
        error "ssh not found"
        ((errors++))
    fi

    # Optional but recommended
    if command_exists zstd; then
        success "zstd: available"
    else
        warn "zstd not found - compression will be slower"
    fi

    if [[ $errors -gt 0 ]]; then
        die "Prerequisites check failed with $errors errors"
    fi

    success "Prerequisites check passed"
}

# ============================================================================
# Worker Mode - Toolchain Verification
# ============================================================================

verify_worker_toolchain() {
    info "Verifying worker toolchain requirements..."

    local errors=0

    # Check rustup
    if command_exists rustup; then
        local rustup_version
        rustup_version=$(rustup --version 2>/dev/null | head -1)
        success "rustup: $rustup_version"
    else
        error "rustup: not found"
        echo "  Install with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        ((errors++))
    fi

    # Check for Rust nightly
    if command_exists rustup && rustup toolchain list 2>/dev/null | grep -q "nightly"; then
        local nightly_version
        nightly_version=$(rustup run nightly rustc --version 2>/dev/null || echo "unknown")
        success "rust nightly: $nightly_version"
    else
        warn "rust nightly: not installed (recommended for full compatibility)"
        echo "  Install with: rustup toolchain install nightly"
    fi

    # Check GCC/Clang
    if command_exists gcc; then
        success "gcc: $(gcc --version | head -1)"
    elif command_exists clang; then
        success "clang: $(clang --version | head -1)"
    else
        error "No C compiler found (gcc or clang required)"
        ((errors++))
    fi

    # Check rsync
    if command_exists rsync; then
        success "rsync: $(rsync --version | head -1)"
    else
        error "rsync: not found"
        echo "  Install with: apt install rsync / brew install rsync"
        ((errors++))
    fi

    # Check zstd
    if command_exists zstd; then
        success "zstd: $(zstd --version 2>&1 | head -1)"
    else
        error "zstd: not found"
        echo "  Install with: apt install zstd / brew install zstd"
        ((errors++))
    fi

    # Check SSH server (for incoming connections)
    if [[ -f /etc/ssh/sshd_config ]] || command_exists sshd; then
        success "sshd: available"
    else
        warn "sshd: not detected (required for receiving remote builds)"
    fi

    if [[ $errors -gt 0 ]]; then
        error "Worker toolchain verification failed with $errors errors"
        return 1
    fi

    success "Worker toolchain verification passed"
}

# ============================================================================
# Installation Functions
# ============================================================================

create_directories() {
    info "Creating directories..."

    mkdir -p "$INSTALL_DIR"
    mkdir -p "$CONFIG_DIR"

    local runtime_dir="${RUNTIME_DIR:-${RCH_RUNTIME_DIR:-${TMPDIR:-/tmp}/rch}}"
    if [[ -e "$runtime_dir" && ! -d "$runtime_dir" ]]; then
        if [[ -n "${RCH_RUNTIME_DIR:-}" || -n "${RUNTIME_DIR:-}" ]]; then
            die "RCH runtime path exists but is not a directory: $runtime_dir"
        fi
        warn "Default runtime path $runtime_dir is occupied; using a private runtime directory"
        runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/rch-runtime.XXXXXX")"
    else
        mkdir -p "$runtime_dir"
    fi
    RUNTIME_DIR="$runtime_dir"

    success "Directories created"
}

install_binary_file() {
    local source_path="$1"
    local dest_path="$2"
    local temp_path="${dest_path}.tmp.$$"

    install -m 755 "$source_path" "$temp_path"
    mv -f "$temp_path" "$dest_path"
}

# A021 split-profile installer: resolve NAME built under PROFILE, falling back
# to the legacy single-profile target/release layout. Required binaries abort
# the install when missing; optional ones (the RABS spine) are skipped with a
# note so a transient spine breakage can never block the core fleet install.
install_split_binary() {
    local name="$1" profile="$2" required="$3"
    local candidate src=""
    for candidate in "$TARGET_DIR/$profile/$name" "$TARGET_DIR/release/$name"; do
        if [[ -f "$candidate" ]]; then
            src="$candidate"
            break
        fi
    done
    if [[ -z "$src" ]]; then
        if [[ "$required" == "required" ]]; then
            die "Binary not found: $name (looked in $TARGET_DIR/$profile/ and $TARGET_DIR/release/)"
        fi
        info "Optional binary $name not present; skipping"
        return 0
    fi
    install_binary_file "$src" "$INSTALL_DIR/$name"
    success "Installed $name ($profile)"
}

source_toolchain_channel() {
    # Require the checkout's dated channel, including with rustup user overrides.
    local line channel=""
    while IFS= read -r line; do
        if [[ "$line" =~ ^[[:space:]]*channel[[:space:]]*=[[:space:]]*\"(nightly-[0-9]{4}-[0-9]{2}-[0-9]{2})\"[[:space:]]*$ ]]; then
            [[ -z "$channel" ]] || die "Ambiguous channel in rust-toolchain.toml"
            channel="${BASH_REMATCH[1]}"
        fi
    done < rust-toolchain.toml
    [[ -n "$channel" ]] || die "Expected a dated nightly channel in rust-toolchain.toml"
    printf '%s\n' "$channel"
}

build_from_source() {
    info "Building from source..."

    # Find project root
    local project_dir
    if [[ -f "Cargo.toml" ]] && grep -q "remote_compilation_helper" Cargo.toml 2>/dev/null; then
        project_dir="$(pwd)"
    elif [[ -f "$(dirname "${BASH_SOURCE[0]}")/Cargo.toml" ]]; then
        project_dir="$(dirname "${BASH_SOURCE[0]}")"
    else
        die "Cannot find RCH source directory. Run from project root or clone the repo first."
    fi

    project_dir=$(cd "$project_dir" && pwd) || return 1
    cd "$project_dir"
    if [[ "${VERSION_EXPLICIT:-false}" == "true" ]]; then
        local requested_tag
        requested_tag=$(installer_release_tag "$VERSION") || return 1
        verify_source_release "$project_dir" "$requested_tag" || return 1
    fi
    [[ -f rust-toolchain.toml ]] || die "Source build requires rust-toolchain.toml"
    local source_toolchain
    source_toolchain=$(source_toolchain_channel)

    # A021 split profiles: hook CLI under wrapper-release (startup +
    # footprint first), daemon/worker under daemon-release (throughput,
    # symbols retained for crashpacks). The RABS spine builds best-effort.
    if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
        TARGET_DIR="${CARGO_TARGET_DIR%/}"
    else
        TARGET_DIR="$project_dir/target"
    fi

    if $USE_GUM; then
        spin "Building rch (wrapper-release profile)..." cargo "+$source_toolchain" build --locked --profile wrapper-release -p rch
        spin "Building rchd + rch-wkr (daemon-release profile)..." cargo "+$source_toolchain" build --locked --profile daemon-release -p rchd -p rch-wkr
        info "Building RABS spine binaries (best-effort)..."
        if ! { cargo "+$source_toolchain" build --locked --profile wrapper-release -p rabs-wrap \
            && cargo "+$source_toolchain" build --locked --profile daemon-release -p rabsd -p rabs-wkr; }; then
            info "RABS spine build unavailable; installing core binaries only"
        fi
    else
        info "Building rch (wrapper-release profile; this may take a while)..."
        cargo "+$source_toolchain" build --locked --profile wrapper-release -p rch
        info "Building rchd + rch-wkr (daemon-release profile)..."
        cargo "+$source_toolchain" build --locked --profile daemon-release -p rchd -p rch-wkr
        info "Building RABS spine binaries (best-effort)..."
        if ! { cargo "+$source_toolchain" build --locked --profile wrapper-release -p rabs-wrap \
            && cargo "+$source_toolchain" build --locked --profile daemon-release -p rabsd -p rabs-wkr; }; then
            info "RABS spine build unavailable; installing core binaries only"
        fi
    fi

    if [[ "$MODE" == "worker" ]]; then
        install_split_binary "$WORKER_BIN" "daemon-release" required
        install_split_binary "rabs-wkr" "daemon-release" optional
    else
        # Local mode: hook CLI from wrapper-release; daemon and worker from
        # daemon-release (worker binary needed for fleet deploy).
        install_split_binary "$HOOK_BIN" "wrapper-release" required
        install_split_binary "$DAEMON_BIN" "daemon-release" required
        install_split_binary "$WORKER_BIN" "daemon-release" required
        install_split_binary "rabs-wrap" "wrapper-release" optional
        install_split_binary "rabsd" "daemon-release" optional
        install_split_binary "rabs-wkr" "daemon-release" optional
    fi

    success "Build complete"
}

installer_release_tag() {
    local version="${1:-}"
    local tag="v${version#v}"
    if [[ -z "$version" || ! "$tag" =~ ^v[0-9][A-Za-z0-9._+-]*$ ]]; then
        error "Cannot bind installation to release version: $version"
        return 1
    fi
    printf '%s\n' "$tag"
}

release_download_url() {
    local tag="$1" asset="$2"
    if [[ ! "$tag" =~ ^v[0-9][A-Za-z0-9._+-]*$ ||
          ! "$asset" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]]; then
        error "Invalid release tag or asset name"
        return 1
    fi
    printf 'https://github.com/%s/releases/download/%s/%s\n' "$GITHUB_REPO" "$tag" "$asset"
}

verify_source_release() {
    local source_root="$1" source_tag
    local source_commit release_commit source_status
    source_tag=$(installer_release_tag "$2") || return 1
    source_commit=$(git -C "$source_root" rev-parse --verify HEAD) || return 1
    release_commit=$(git -C "$source_root" rev-parse --verify "refs/tags/${source_tag}^{commit}") || return 1
    if [[ "$source_commit" != "$release_commit" ]]; then
        error "Source checkout does not match release tag $source_tag; build refused"
        return 1
    fi
    source_status=$(git -C "$source_root" status --porcelain --untracked-files=all) || return 1
    if [[ -n "$source_status" ]]; then
        error "Source checkout has changes outside release tag $source_tag; build refused"
        return 1
    fi
}

download_binaries() {
    validate_release_configuration
    [[ -n "${VERSION:-}" ]] || die "Resolve a release version before downloading"
    local tag_name
    tag_name=$(installer_release_tag "$VERSION") || return 1
    local release_url="${REPO_URL}/releases/download/${tag_name}"
    local asset_ext="${RELEASE_ARCHIVE_EXT}" asset_name="" candidate
    local downloaded=false
    local candidates=()
    for candidate in "${RELEASE_TARGETS[@]}"; do
        candidates+=("rch-${tag_name}-${candidate}.${asset_ext}")
    done
    for candidate in "${RELEASE_TARGETS[@]}" "$TARGET"; do
        candidates+=("rch-${candidate}.${asset_ext}")
    done

    TEMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TEMP_DIR"; cleanup_lock' EXIT
    local download_dir="$TEMP_DIR/download" extract_dir="$TEMP_DIR/extracted"
    mkdir "$download_dir" "$extract_dir"
    # All candidates, including legacy names, belong to the SAME release.
    for candidate in "${candidates[@]}"; do
        info "Downloading $candidate from $tag_name..."
        if curl -fsSL -A "$DOWNLOAD_USER_AGENT" "${PROXY_ARGS[@]}" \
            "$release_url/$candidate" -o "$download_dir/$candidate"; then
            asset_name="$candidate"
            downloaded=true
            break
        fi
    done
    if [[ "$downloaded" != "true" ]]; then
        warn "No pre-built binaries available for ${TARGET} at ${tag_name}"
        clone_and_build_from_source
        return
    fi

    # Missing sidecar may use the same release's manifest. A present but invalid
    # sidecar is an integrity error, never a reason to try another trust path.
    local checksum_content expected
    if checksum_content=$(curl -fsSL -A "$DOWNLOAD_USER_AGENT" "${PROXY_ARGS[@]}" \
        "$release_url/$asset_name.sha256"); then
        expected=$(extract_checksum_from_manifest "$checksum_content" "$asset_name" true) ||
            die "Malformed checksum for $asset_name"
    else
        checksum_content=$(curl -fsSL -A "$DOWNLOAD_USER_AGENT" "${PROXY_ARGS[@]}" \
            "$release_url/SHA256SUMS") || die "Missing checksum for $asset_name"
        expected=$(extract_checksum_from_manifest "$checksum_content" "$asset_name") ||
            die "Missing or malformed checksum for $asset_name"
    fi
    verify_checksum_value "$download_dir/$asset_name" "$expected" || die "Checksum verification failed"
    verify_sigstore_bundle "$download_dir/$asset_name" "$release_url/$asset_name" "$tag_name" ||
        die "Signature verification failed"

    info "Extracting binaries..."
    if [[ "$asset_ext" == "zip" ]]; then
        command_exists unzip || die "unzip command not found"
        unzip -q "$download_dir/$asset_name" -d "$extract_dir" || die "Failed to extract zip"
    else
        tar -xzf "$download_dir/$asset_name" -C "$extract_dir" || die "Failed to extract tarball"
    fi
    local binaries=("$WORKER_BIN") binary
    if [[ "$MODE" != "worker" ]]; then
        binaries=("$HOOK_BIN" "$DAEMON_BIN" "$WORKER_BIN")
    fi
    # Preflight every required binary before replacing any installed binary.
    for binary in "${binaries[@]}"; do
        [[ -f "$extract_dir/$binary" && ! -L "$extract_dir/$binary" ]] ||
            die "$binary missing or not a regular release binary"
    done
    for binary in "${binaries[@]}"; do
        install_binary_file "$extract_dir/$binary" "$INSTALL_DIR/$binary"
        success "Installed $binary"
    done
    success "Binary installation complete"
}

clone_and_build_from_source() {
    # Only an explicit release request authorizes source fallback. Never build
    # an arbitrary default branch after discovery/download failures.
    validate_release_configuration
    [[ "${VERSION_EXPLICIT:-false}" == "true" && -n "${VERSION:-}" ]] ||
        die "Source fallback requires an explicit VERSION or --version tag"
    [[ "$SIGNATURE_POLICY" != "required" ]] ||
        die "Required artifact signatures cannot be satisfied by a source fallback"
    local source_tag
    source_tag=$(installer_release_tag "$VERSION") || return 1
    ensure_command git

    # Check for Rust toolchain
    if ! command_exists cargo; then
        info "Rust toolchain not found. Installing via rustup..."
        if ! install_rust_toolchain; then
            die "Failed to install Rust. Please install manually: https://rustup.rs"
        fi
    fi

    TEMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TEMP_DIR"; cleanup_lock' EXIT

    info "Cloning repository..."
    if $USE_GUM; then
        if ! spin "Cloning RCH repository..." git clone --depth 1 --branch "$source_tag" "$REPO_URL" "$TEMP_DIR/rch"; then
            die "Failed to clone repository from $REPO_URL"
        fi
    else
        if ! git clone --depth 1 --branch "$source_tag" "$REPO_URL" "$TEMP_DIR/rch"; then
            die "Failed to clone repository from $REPO_URL"
        fi
    fi

    # --branch can also resolve a branch: require the requested tag and detach
    # at that tag before any build, rejecting a branch-only look-alike.
    git -C "$TEMP_DIR/rch" checkout --detach "refs/tags/$source_tag" ||
        die "Requested source tag not found: $source_tag"
    cd "$TEMP_DIR/rch" || die "Failed to change to cloned repository directory"
    verify_source_release "$TEMP_DIR/rch" "$source_tag" || return 1
    [[ -f rust-toolchain.toml ]] || die "Source build requires rust-toolchain.toml"
    local source_toolchain
    source_toolchain=$(source_toolchain_channel)

    # A021 split profiles (see build_from_source for rationale).
    if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
        TARGET_DIR="${CARGO_TARGET_DIR%/}"
    else
        TARGET_DIR="$TEMP_DIR/rch/target"
    fi

    if $USE_GUM; then
        if ! spin "Building rch (wrapper-release profile; this may take a few minutes)..." cargo "+$source_toolchain" build --locked --profile wrapper-release -p rch; then
            die "Build failed. Install the pinned toolchain from rust-toolchain.toml (rustup show)."
        fi
        if ! spin "Building rchd + rch-wkr (daemon-release profile)..." cargo "+$source_toolchain" build --locked --profile daemon-release -p rchd -p rch-wkr; then
            die "Build failed. Install the pinned toolchain from rust-toolchain.toml (rustup show)."
        fi
        info "Building RABS spine binaries (best-effort)..."
        if ! { cargo "+$source_toolchain" build --locked --profile wrapper-release -p rabs-wrap \
            && cargo "+$source_toolchain" build --locked --profile daemon-release -p rabsd -p rabs-wkr; }; then
            info "RABS spine build unavailable; installing core binaries only"
        fi
    else
        info "Building rch (wrapper-release profile; this may take a few minutes)..."
        if ! cargo "+$source_toolchain" build --locked --profile wrapper-release -p rch; then
            die "Build failed. Install the pinned toolchain from rust-toolchain.toml (rustup show)."
        fi
        info "Building rchd + rch-wkr (daemon-release profile)..."
        if ! cargo "+$source_toolchain" build --locked --profile daemon-release -p rchd -p rch-wkr; then
            die "Build failed. Install the pinned toolchain from rust-toolchain.toml (rustup show)."
        fi
        info "Building RABS spine binaries (best-effort)..."
        if ! { cargo "+$source_toolchain" build --locked --profile wrapper-release -p rabs-wrap \
            && cargo "+$source_toolchain" build --locked --profile daemon-release -p rabsd -p rabs-wkr; }; then
            info "RABS spine build unavailable; installing core binaries only"
        fi
    fi

    if [[ "$MODE" == "worker" ]]; then
        install_split_binary "$WORKER_BIN" "daemon-release" required
        install_split_binary "rabs-wkr" "daemon-release" optional
    else
        install_split_binary "$HOOK_BIN" "wrapper-release" required
        install_split_binary "$DAEMON_BIN" "daemon-release" required
        install_split_binary "$WORKER_BIN" "daemon-release" required
        install_split_binary "rabs-wrap" "wrapper-release" optional
        install_split_binary "rabsd" "daemon-release" optional
        install_split_binary "rabs-wkr" "daemon-release" optional
    fi

    success "Build from source complete"
}

install_rust_toolchain() {
    # Install Rust via rustup if not present
    if command_exists rustup; then
        info "rustup found, ensuring nightly toolchain..."
        rustup toolchain install nightly 2>/dev/null || true
        return 0
    fi

    info "Installing Rust via rustup..."
    # Note: rustup respects HTTPS_PROXY/HTTP_PROXY environment variables automatically
    if curl --proto '=https' --tlsv1.2 -sSf "${PROXY_ARGS[@]}" https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly; then
        # Source cargo env for current session
        if [[ -f "$HOME/.cargo/env" ]]; then
            # shellcheck source=/dev/null
            source "$HOME/.cargo/env"
        fi
        success "Rust installed successfully"
        return 0
    else
        return 1
    fi
}

# ============================================================================
# Checksum Verification
# ============================================================================

verify_checksum() {
    local file="$1"
    local checksum_file="$2"
    local filename="$3"

    local expected
    [[ -f "$checksum_file" ]] || { error "Missing checksum file: $checksum_file"; return 1; }
    expected=$(extract_checksum_from_manifest "$(<"$checksum_file")" "$filename") || {
        error "Missing or malformed checksum for $filename"; return 1;
    }
    verify_checksum_value "$file" "$expected"
}

extract_checksum_from_manifest() {
    local manifest_content="$1"
    local filename="$2"
    local allow_bare="${3:-false}"

    printf '%s\n' "$manifest_content" | awk -v file="$filename" -v bare="$allow_bare" '
        {
            sub(/\r$/, "")
            if (!NF) next
            records++
            path = $2
            sub(/^\*/, "", path)
            sub(/^\.\//, "", path)
            # Release jobs hash dist/ASSET before uploading the basename.
            sub(/^dist\//, "", path)
            if (path == file || (bare == "true" && NF == 1)) {
                count++
                if ((NF != 2 && !(bare == "true" && NF == 1)) ||
                    length($1) != 64 || $1 !~ /^[[:xdigit:]]+$/) {
                    invalid = 1
                }
                checksum = tolower($1)
            }
        }
        END {
            if (count != 1 || invalid || (bare == "true" && records != 1)) exit 1
            print checksum
        }
    '
}

verify_checksum_value() {
    local file="$1" expected="$2" computed
    [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] || return 1
    if command_exists sha256sum; then
        computed=$(sha256sum "$file") || return 1
    elif command_exists shasum; then
        computed=$(shasum -a 256 "$file") || return 1
    else
        error "No SHA256 tool found"
        return 1
    fi
    computed="${computed%% *}"
    [[ "$computed" == "$expected" ]] || return 1
    success "Checksum verified"
}

# ============================================================================
# Version Detection
# ============================================================================

detect_target_version() {
    # If VERSION already set (e.g., from environment), use it
    if [[ -n "${VERSION:-}" ]]; then
        VERSION_EXPLICIT=true
        return 0
    fi

    # Offline identity comes from this archive, never the current checkout,
    # latest release, or installer floor. Strip only known target suffixes so
    # prerelease identifiers (including hyphens) remain part of the tag.
    if [[ -n "${OFFLINE_TARBALL:-}" ]]; then
        local archive_name="${OFFLINE_TARBALL##*/}" suffix recognized_target=false
        case "$archive_name" in
            *.tar.gz) archive_name="${archive_name%.tar.gz}" ;;
            *.zip) archive_name="${archive_name%.zip}" ;;
            *) die "Cannot infer offline release tag; supply --version or VERSION" ;;
        esac
        for suffix in x86_64-unknown-linux-musl x86_64-unknown-linux-gnu \
            aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin \
            x86_64-pc-windows-msvc linux-x86_64 linux-aarch64 \
            darwin-x86_64 darwin-aarch64 windows-x86_64; do
            if [[ "$archive_name" == *"-$suffix" ]]; then
                archive_name="${archive_name%"-$suffix"}"
                recognized_target=true
                break
            fi
        done
        # Require a recognized target suffix: without one a suffix could be
        # either a prerelease identifier or an unknown platform name.
        if [[ "$recognized_target" == true &&
              "$archive_name" =~ ^rch-v?([0-9]+\.[0-9]+\.[0-9]+([-+][A-Za-z0-9.-]+)?)$ ]]; then
            VERSION="${BASH_REMATCH[1]}"
            return 0
        fi
        die "Cannot infer offline release tag; supply --version or VERSION"
    fi

    # If building from source, read from Cargo.toml
    if [[ "${FROM_SOURCE:-false}" == "true" ]] || \
       { [[ -f "Cargo.toml" ]] && grep -q "remote_compilation_helper" Cargo.toml 2>/dev/null; }; then
        local cargo_version=""

        # Check if this is a Cargo workspace (root Cargo.toml has [workspace])
        # In workspaces, version is often defined in [workspace.package] section
        if [[ -f "Cargo.toml" ]] && grep -q '^\[workspace\]' Cargo.toml 2>/dev/null; then
            # Extract version from [workspace.package] section using awk
            cargo_version=$(awk '
                /^\[workspace\.package\]/ { in_section=1; next }
                /^\[/ { in_section=0 }
                in_section && /^version[[:space:]]*=[[:space:]]*"/ {
                    gsub(/^version[[:space:]]*=[[:space:]]*"/, "")
                    gsub(/".*$/, "")
                    print
                    exit
                }
            ' Cargo.toml)
        fi

        # If workspace extraction failed or not a workspace, try direct extraction
        if [[ -z "$cargo_version" ]]; then
            if [[ -f "rch/Cargo.toml" ]]; then
                cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' rch/Cargo.toml | head -1)
            fi
        fi

        # Last resort: try root Cargo.toml with simple pattern
        if [[ -z "$cargo_version" ]] && [[ -f "Cargo.toml" ]]; then
            cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
        fi

        if [[ -n "$cargo_version" ]]; then
            VERSION="$cargo_version"
            return 0
        fi
    fi

    # Try to fetch latest from GitHub (with timeout)
    local latest
    # NB: use "v*" not "v\?" - \? is a GNU sed extension that BSD sed (macOS)
    # does not understand, where it silently yields an empty match and sends us
    # down the stale-fallback path.
    latest=$(curl -fsSL -A "$DOWNLOAD_USER_AGENT" --connect-timeout 5 "${GITHUB_API}/releases/latest" | \
             sed -n 's/.*"tag_name": *"v*\([^"]*\)".*/\1/p' | head -1) || latest=""
    if [[ -n "$latest" ]]; then
        VERSION="$latest"
        return 0
    fi

    # The api.github.com endpoint above is rate limited to 60 requests/hour for
    # unauthenticated clients, per source IP. A host that installs many tools in
    # one batch (or several hosts behind one NAT) exhausts that budget, and the
    # rate-limited response carries no "tag_name" — so the parse above yields an
    # empty string and we silently fall through. Retry via the github.com
    # releases/latest redirect, which is not part of the API rate limit: it 302s
    # to .../releases/tag/vX.Y.Z, so the resolved URL carries the version.
    local latest_url
    latest_url=$(curl -fsSL -A "$DOWNLOAD_USER_AGENT" -o /dev/null --connect-timeout 5 --max-time 20 \
                 -w '%{url_effective}' "${REPO_URL}/releases/latest") || latest_url=""
    latest=$(printf '%s' "$latest_url" | sed -n 's|.*/releases/tag/v*\([0-9][0-9.]*\).*|\1|p')
    if [[ -n "$latest" ]]; then
        VERSION="$latest"
        return 0
    fi

    # Fallback to installer version. This is a floor, not a target: it goes stale
    # every release, so enforce_no_downgrade() below refuses to replace a newer
    # existing install with it.
    VERSION="$INSTALLER_VERSION"
}

# Refuse to replace an existing install with an older one.
#
# Version discovery is best-effort (network, API rate limits, offline installs),
# and every fallback path is stale by construction. Without this guard a
# transient lookup failure turns a routine "keep tools updated" run into a
# silent DOWNGRADE of a working binary — which is exactly how the fleet ended up
# running rch 1.0.52 while 1.0.57 was the published release.
#
# An explicitly requested version (VERSION=... or --version) is always honoured,
# so intentional rollbacks still work.
version_gt() {
    [[ "$1" != "$2" ]] && \
        [[ "$(printf '%s\n%s\n' "$1" "$2" | sort -V 2>/dev/null | tail -1)" == "$1" ]]
}

enforce_no_downgrade() {
    [[ "${VERSION_EXPLICIT:-false}" == "true" ]] && return 0
    [[ -n "${OFFLINE_TARBALL:-}" ]] && return 0
    [[ "${FROM_SOURCE:-false}" == "true" ]] && return 0
    [[ -x "$INSTALL_DIR/rch" ]] || return 0

    local current
    current=$("$INSTALL_DIR/rch" --version 2>/dev/null | head -1 | sed 's/rch //' | awk '{print $1}')
    [[ -n "$current" ]] || return 0
    [[ -n "${VERSION:-}" ]] || return 0

    if version_gt "$current" "$VERSION"; then
        warn "Installed rch v${current} is newer than resolved target v${VERSION}."
        warn "This usually means the release lookup failed (GitHub API rate limit) and"
        warn "the installer fell back to its built-in floor. Keeping v${current}."
        info "To force a specific version anyway: VERSION=${VERSION} $0"
        VERSION="$current"
        SKIP_INSTALL_NO_DOWNGRADE=true
    fi
    return 0
}

# ============================================================================
# Upgrade Detection
# ============================================================================

check_existing_install() {
    if [[ -x "$INSTALL_DIR/rch" ]]; then
        local current
        current=$("$INSTALL_DIR/rch" --version 2>/dev/null | head -1 | sed 's/rch //' || echo "")
        if [[ -n "$current" ]]; then
            if $USE_GUM; then
                gum style \
                    --border rounded \
                    --border-foreground 208 \
                    --padding "0 2" \
                    "Upgrading RCH" \
                    "Current: v$current" \
                    "Target:  v$VERSION"
            else
                draw_box "1;33" \
                    "Upgrading RCH" \
                    "Current: v$current" \
                    "Target:  v$VERSION"
            fi
            return 0
        fi
    fi
    return 1
}

# ============================================================================
# Sigstore Verification
# ============================================================================

verify_sigstore_bundle() {
    local file="$1" artifact_url="$2" tag="$3"
    validate_release_configuration
    if ! command_exists cosign; then
        [[ "$SIGNATURE_POLICY" != "required" ]] || {
            error "Required signature verification needs cosign"; return 1;
        }
        warn "Signature NOT verified: cosign unavailable (optional policy)"
        return 0
    fi

    local bundle_file="${file}.sigstore.json"
    # An empty URL is the offline path: use the adjacent, supplied bundle only.
    if [[ -n "$artifact_url" ]]; then
        if ! curl -fsSL -A "$DOWNLOAD_USER_AGENT" "${PROXY_ARGS[@]}" \
            "${artifact_url}.sigstore.json" -o "$bundle_file"; then
            [[ "$SIGNATURE_POLICY" != "required" ]] || {
                error "Required Sigstore bundle unavailable"; return 1;
            }
            warn "Signature NOT verified: bundle unavailable (optional policy)"
            return 0
        fi
    elif [[ ! -f "$bundle_file" ]]; then
        [[ "$SIGNATURE_POLICY" != "required" ]] || return 1
        warn "Signature NOT verified: offline bundle unavailable (optional policy)"
        return 0
    fi
    [[ -n "$tag" ]] || { error "Signature verification requires a release tag"; return 1; }
    # Exact identity, not a wildcard or upstream public key. This accepts the
    # tag-triggered release workflow only; a main-branch dispatch is not proof
    # that the workflow identity was bound to this release tag.
    local identity="https://github.com/${GITHUB_REPO}/.github/workflows/release.yml@refs/tags/${tag}"
    if cosign verify-blob --bundle "$bundle_file" \
        --certificate-identity="$identity" \
        --certificate-oidc-issuer="https://token.actions.githubusercontent.com" "$file"; then
        success "Sigstore signature verified"
    else
        error "Sigstore signature verification failed!"
        return 1
    fi
}

# ============================================================================
# Skill Installation
# ============================================================================

# A destination already holds a *full* skill (not the inline stub) when SKILL.md
# is present and at least one companion reference file exists beside it — the
# stub only ever writes a lone SKILL.md. Used to keep the stub fallback from
# downgrading a real skill on unattended re-runs (nightly updaters). See GH#45.
_rch_skill_is_full() {
    local dest="$1"
    [[ -f "${dest}/SKILL.md" ]] || return 1
    local f
    for f in "${dest}"/references/*.md "${dest}"/assets/*.toml; do
        [[ -f "$f" ]] && return 0
    done
    return 1
}

install_skill() {
    local claude_dest="$HOME/.claude/skills/rch"
    local codex_dest="$HOME/.codex/skills/rch"
    local installed_claude=false
    local installed_codex=false
    local repo_skill_path=""
    local skill_raw_base="https://raw.githubusercontent.com/${GITHUB_REPO}/main/.claude/skills/rch"
    local skill_stage="${TEMP_DIR:-/tmp}/rch-skill-stage.$$"
    local raw_skill_ok=true
    local rel_path=""
    local script_dir
    local skill_url="https://github.com/${GITHUB_REPO}/releases/latest/download/skill.tar.gz"
    local skill_temp="${TEMP_DIR:-/tmp}/skill.tar.gz"
    # Optional companion files fetched alongside SKILL.md. Keep this in sync with
    # the actual .claude/skills/rch tree, but a stale entry here is no longer
    # fatal: only SKILL.md is required (see the fetch loop below), so a renamed
    # or removed companion degrades to a warning instead of discarding the whole
    # raw-tree install and falling through to the inline stub.
    local skill_files=(
        "assets/workers-template.toml"
        "references/COMMANDS.md"
        "references/CONFIGURATION.md"
        "references/HOOKS.md"
        "references/OPERATIONS.md"
        "references/TROUBLESHOOTING.md"
        "references/WORKERS.md"
        "scripts/validate-setup.sh"
    )

    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

    info "Installing RCH skill for AI coding agents..."

    # Create both skill directories
    mkdir -p "$claude_dest/references"
    mkdir -p "$codex_dest/references"

    if [[ -d "${script_dir}/.claude/skills/rch" ]] && [[ -f "${script_dir}/.claude/skills/rch/SKILL.md" ]]; then
        repo_skill_path="${script_dir}/.claude/skills/rch"
    fi

    if [[ -n "$repo_skill_path" ]] && cp -R "${repo_skill_path}/." "$claude_dest/" && cp -R "${repo_skill_path}/." "$codex_dest/"; then
        success "Installed RCH skill from repository source at $repo_skill_path"
        show_skill_info true true
        return 0
    fi

    mkdir -p "$skill_stage/assets" "$skill_stage/references" "$skill_stage/scripts"
    # SKILL.md is mandatory; without it the raw tree cannot yield a usable skill.
    if ! curl -fsSL "${PROXY_ARGS[@]}" "${skill_raw_base}/SKILL.md" -o "${skill_stage}/SKILL.md" 2>/dev/null; then
        raw_skill_ok=false
    fi
    # Companion files are best-effort: a single missing/renamed file must not
    # discard the whole install (that regression is what forced every tarball
    # user onto the inline stub — see GH#45).
    if $raw_skill_ok; then
        for rel_path in "${skill_files[@]}"; do
            mkdir -p "${skill_stage}/$(dirname "$rel_path")"
            if ! curl -fsSL "${PROXY_ARGS[@]}" "${skill_raw_base}/${rel_path}" -o "${skill_stage}/${rel_path}" 2>/dev/null; then
                warn "Skill companion file not found (skipping): ${rel_path}"
                rm -f "${skill_stage}/${rel_path}" 2>/dev/null || true
            fi
        done
    fi

    if $raw_skill_ok && cp -R "${skill_stage}/." "$claude_dest/" && cp -R "${skill_stage}/." "$codex_dest/"; then
        success "Installed RCH skill from repository raw skill tree"
        show_skill_info true true
        return 0
    fi

    # Try release-packaged skill only after repo-local and raw repo sources.
    if curl -fsSL "${PROXY_ARGS[@]}" "$skill_url" -o "$skill_temp" 2>/dev/null; then
        if tar -xzf "$skill_temp" -C "$HOME/.claude/skills" 2>/dev/null; then
            installed_claude=true
        fi
        if tar -xzf "$skill_temp" -C "$HOME/.codex/skills" 2>/dev/null; then
            installed_codex=true
        fi
        rm -f "$skill_temp"

        if $installed_claude || $installed_codex; then
            [[ $installed_claude == true ]] && success "Installed RCH skill to $claude_dest"
            [[ $installed_codex == true ]] && success "Installed RCH skill to $codex_dest"
            show_skill_info "$installed_claude" "$installed_codex"
            return 0
        fi
    fi

    # Fallback: create minimal skill inline
    info "Creating minimal skill (download failed)..."

    local skill_content
    skill_content=$(cat << 'SKILL_EOF'
---
name: rch
description: >-
  Offload cargo/gcc/bun builds to remote workers. Use when compilation is slow,
  workers are unhealthy, hook routing is unclear, or remote sync/execution is
  failing.
---

# RCH — Remote Compilation Helper

Use this skill for remote compilation offload, worker fleet health checks, and hook incident recovery.

## Quick Start

```bash
rch check
rch status --workers --jobs
rch workers probe --all
rch hook status
rch diagnose --dry-run "cargo check --workspace --all-targets"
rch exec -- env CARGO_TARGET_DIR=${TMPDIR:-/tmp}/rch_target_<name> cargo check --workspace --all-targets
```

If `rch exec -- ...` succeeds, remote offload is healthy and remaining failures are likely project/toolchain specific.

If `rch status` shows storage pressure, always check both `/` and `/tmp` on the worker before deciding what to fix:

```bash
ssh ubuntu@<host> 'df -h / /tmp && free -h && cat /proc/pressure/memory && cat /proc/pressure/io'
```

---

## Fast Triage Order

Run in this order and stop at the first failing stage:

1. **Availability**
```bash
rch check
rch status --workers --jobs
rch workers probe --all
rch queue
```

2. **Config + socket consistency**
```bash
rch config show --sources
rch --json config get general.socket_path
rch --json daemon status
```

3. **Hook integration**
```bash
rch hook status
rch agents status
rch hook install
```

4. **Command classification + path closure**
```bash
rch diagnose "cargo build --release"
rch diagnose --dry-run "cargo test --workspace"
```

5. **Remote compile proof**
```bash
rch exec -- env CARGO_TARGET_DIR=${TMPDIR:-/tmp}/rch_target_<name> cargo check --workspace --all-targets
```

6. **If sync fails or storage looks bad, inspect the worker directly**
```bash
ssh ubuntu@<host> 'df -h / /tmp'
ssh ubuntu@<host> 'du -sh /tmp/rch-* /tmp/rch_target_* 2>/dev/null | sort -h'
ssh ubuntu@<host> 'find /data/projects -maxdepth 2 -type d \\( -name "target_rch_*" -o -name "target_*" -o -name "target-*" -o -name target \\) -exec du -sh {} + 2>/dev/null | sort -h | tail'
```

---

## Quick Fixes

| Symptom | Command |
|---------|---------|
| Hook not installed | `rch hook install && rch hook status` |
| Daemon not running | `rch daemon start` |
| Socket mismatch / stale daemon state | `rch daemon restart -y` then `rch --json daemon status` |
| No workers configured | `rch workers discover --add --yes && rch workers setup --all` |
| Workers unreachable | `rch workers probe --all` then fix SSH key/host reachability |
| Transfer churn under target dirs | Add excludes in `~/.config/rch/config.toml`, then `rch daemon reload` |
| Path dependency missing remotely | Ensure required sibling repos exist on workers under canonical project roots, then retry `rch exec -- ...` |
| Sync fails with `Permission denied` in `/data/projects/<repo>` | Fix remote mirror ownership: `ssh ubuntu@<host> 'sudo chown -R ubuntu:ubuntu /data/projects/<repo> && sudo chmod 775 /data/projects/<repo>'` |
| Worker shows pressure warning | Check `/` and `/tmp` separately, then inspect stale `rch_target_*`, `rch-*`, and `target_rch_*` dirs before broader cleanup |
| Need full environment diagnosis | `rch doctor` and `rch config doctor` |

---

## Reference Index

Use these files for full depth:

- **Runbooks + operational playbooks**: `references/OPERATIONS.md`
- **Troubleshooting flow + failure signatures**: `references/TROUBLESHOOTING.md`
- **Worker lifecycle operations**: `references/WORKERS.md`
- **Config hierarchy + environment controls**: `references/CONFIGURATION.md`
- **PreToolUse hook protocol and behavior**: `references/HOOKS.md`
- **Workers config template**: `assets/workers-template.toml`
- **Project docs**: https://github.com/Dicklesworthstone/remote_compilation_helper
SKILL_EOF
)

    # Write to both destinations — but never downgrade an existing full skill to
    # the stub. On unattended re-runs (e.g. the nightly ACFS updater) a transient
    # fetch failure must not clobber a real skill already on disk (GH#45).
    if _rch_skill_is_full "$claude_dest"; then
        warn "Keeping existing full RCH skill at $claude_dest (not overwriting with stub)"
    else
        echo "$skill_content" > "$claude_dest/SKILL.md"
        installed_claude=true
        success "Created RCH skill at $claude_dest"
    fi
    if _rch_skill_is_full "$codex_dest"; then
        warn "Keeping existing full RCH skill at $codex_dest (not overwriting with stub)"
    else
        echo "$skill_content" > "$codex_dest/SKILL.md"
        installed_codex=true
        success "Created RCH skill at $codex_dest"
    fi
    show_skill_info "$installed_claude" "$installed_codex"
}

# Show info about the installed skill
show_skill_info() {
    local installed_claude="${1:-true}"
    local installed_codex="${2:-false}"

    echo ""

    # Build location lines based on what was installed
    local loc_lines=()
    [[ "$installed_claude" == "true" ]] && loc_lines+=("  📂 Claude Code:  ~/.claude/skills/rch/")
    [[ "$installed_codex" == "true" ]] && loc_lines+=("  📂 Codex CLI:    ~/.codex/skills/rch/")

    if $USE_GUM; then
        gum style \
            --border rounded \
            --border-foreground 141 \
            --padding "1 2" \
            --align left \
            "🤖 AI Agent Skills Installed" \
            "" \
            "The RCH skill is now available to AI coding agents." \
            "It provides setup guidance, troubleshooting, and quick fixes." \
            "" \
            "Installed to:" \
            "${loc_lines[@]}" \
            "" \
            "How Skills Work:" \
            "  When you describe an RCH problem to your AI agent," \
            "  the skill activates automatically. No slash commands" \
            "  needed—just describe the issue naturally:" \
            "" \
            "    \"rch doctor is showing errors\"" \
            "    \"no workers available\"" \
            "    \"how do I add a new worker?\"" \
            "" \
            "  The agent will use the skill's knowledge to help." \
            "" \
            "Trigger phrases: rch doctor, workers.toml, fleet deploy," \
            "                 no workers, compilation slow, self-test"
    else
        echo ""
        if $USE_COLOR; then
            echo -e "${BOLD}${CYAN}╭───────────────────────────────────────────────────────────╮${NC}"
            echo -e "${BOLD}${CYAN}│${NC}  ${BOLD}🤖 AI Agent Skills Installed${NC}                             ${BOLD}${CYAN}│${NC}"
            echo -e "${BOLD}${CYAN}╰───────────────────────────────────────────────────────────╯${NC}"
        else
            echo "╭───────────────────────────────────────────────────────────╮"
            echo "│  AI Agent Skills Installed                                │"
            echo "╰───────────────────────────────────────────────────────────╯"
        fi
        echo ""
        echo "  The RCH skill is now available to AI coding agents."
        echo "  It provides setup guidance, troubleshooting, and quick fixes."
        echo ""
        echo "  Installed to:"
        for line in "${loc_lines[@]}"; do
            echo "$line"
        done
        echo ""
        if $USE_COLOR; then
            echo -e "  ${BOLD}How Skills Work:${NC}"
        else
            echo "  How Skills Work:"
        fi
        echo "  ─────────────────"
        echo "  When you describe an RCH problem to your AI agent,"
        echo "  the skill activates automatically. No slash commands"
        echo "  needed—just describe the issue naturally:"
        echo ""
        if $USE_COLOR; then
            echo -e "    ${DIM}\"rch doctor is showing errors\"${NC}"
            echo -e "    ${DIM}\"no workers available\"${NC}"
            echo -e "    ${DIM}\"how do I add a new worker?\"${NC}"
        else
            echo "    \"rch doctor is showing errors\""
            echo "    \"no workers available\""
            echo "    \"how do I add a new worker?\""
        fi
        echo ""
        echo "  The agent will use the skill's knowledge to help."
        echo ""
        if $USE_COLOR; then
            echo -e "  ${BOLD}Trigger phrases:${NC} rch doctor, workers.toml, fleet deploy,"
        else
            echo "  Trigger phrases: rch doctor, workers.toml, fleet deploy,"
        fi
        echo "                   no workers, compilation slow, self-test"
    fi
}

# ============================================================================
# Offline Installation
# ============================================================================

install_from_tarball() {
    local tarball="$1"

    if [[ ! -f "$tarball" ]]; then
        die "Tarball not found: $tarball"
    fi

    validate_release_configuration
    local checksum_file="${tarball}.sha256" expected
    [[ -f "$checksum_file" ]] || die "Missing offline checksum: $checksum_file"
    expected=$(extract_checksum_from_manifest "$(<"$checksum_file")" "$(basename "$tarball")") ||
        die "Malformed offline checksum"
    verify_checksum_value "$tarball" "$expected" || die "Offline checksum verification failed"
    local tag=""
    [[ -z "${VERSION:-}" ]] || tag="v${VERSION#v}"
    verify_sigstore_bundle "$tarball" "" "$tag" || die "Offline signature verification failed"
    info "Installing from offline tarball: $tarball"

    TEMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TEMP_DIR"; cleanup_lock' EXIT

    # Extract tarball
    if $USE_GUM; then
        spin "Extracting tarball..." tar -xzf "$tarball" -C "$TEMP_DIR"
    else
        info "Extracting tarball..."
        tar -xzf "$tarball" -C "$TEMP_DIR"
    fi

    # Install binaries from tarball
    if [[ "$MODE" == "worker" ]]; then
        if [[ -f "$TEMP_DIR/$WORKER_BIN" ]]; then
            install_binary_file "$TEMP_DIR/$WORKER_BIN" "$INSTALL_DIR/$WORKER_BIN"
            success "Installed $WORKER_BIN"
        else
            die "Worker binary not found in tarball"
        fi
    else
        for binary in "$HOOK_BIN" "$DAEMON_BIN"; do
            if [[ -f "$TEMP_DIR/$binary" ]]; then
                install_binary_file "$TEMP_DIR/$binary" "$INSTALL_DIR/$binary"
                success "Installed $binary"
            else
                warn "$binary not found in tarball"
            fi
        done

        if [[ -f "$TEMP_DIR/$WORKER_BIN" ]]; then
            install_binary_file "$TEMP_DIR/$WORKER_BIN" "$INSTALL_DIR/$WORKER_BIN"
            success "Installed $WORKER_BIN"
        else
            if [[ "${RCH_SKIP_FLEET_SYNC:-}" == "1" ]]; then
                warn "Worker binary not found in tarball (fleet sync disabled)"
            else
                die "Worker binary not found in tarball (required for fleet deploy)"
            fi
        fi
    fi

    success "Offline installation complete"
}

install_binaries() {
    # Check for offline tarball first
    if [[ -n "${OFFLINE_TARBALL:-}" ]]; then
        install_from_tarball "$OFFLINE_TARBALL"
        return
    fi

    if [[ "${VERSION_EXPLICIT:-false}" == "true" ]]; then
        if [[ "${FROM_SOURCE:-false}" == "true" ]]; then
            clone_and_build_from_source
        else
            download_binaries
        fi
    elif [[ "${FROM_SOURCE:-false}" == "true" ]]; then
        [[ "$SIGNATURE_POLICY" != "required" ]] || die "Local source has no release artifact signature"
        build_from_source
    else
        # Required signatures and explicit versions must not silently use a checkout.
        if [[ "$SIGNATURE_POLICY" != "required" && -f "Cargo.toml" ]] && grep -q "remote_compilation_helper" Cargo.toml 2>/dev/null; then
            info "Found source directory, building from source..."
            FROM_SOURCE=true
            build_from_source
        else
            # Try binary download (handles fallback to source build internally)
            download_binaries
        fi
    fi
}

# ============================================================================
# Configuration
# ============================================================================

generate_default_config() {
    local daemon_config="$CONFIG_DIR/daemon.toml"
    local workers_config="$CONFIG_DIR/workers.toml"

    if [[ ! -f "$daemon_config" ]]; then
        info "Generating default daemon configuration..."
        cat > "$daemon_config" << 'EOF'
# RCH Daemon Configuration
# See: https://github.com/Dicklesworthstone/remote_compilation_helper

# Unix socket path for hook communication.
# Default: $XDG_RUNTIME_DIR/rch.sock (if available), else ~/.cache/rch/rch.sock
# Uncomment to override:
# socket_path = "/tmp/rch.sock"

# Health check interval in seconds
health_check_interval_secs = 30

# Worker timeout before marking as unreachable (seconds)
worker_timeout_secs = 10

# Maximum concurrent jobs per worker slot
max_jobs_per_slot = 1

# Enable SSH connection pooling
connection_pooling = true

# Log level: trace, debug, info, warn, error
log_level = "info"
EOF
        success "Created $daemon_config"
    else
        info "Daemon config already exists: $daemon_config"
    fi

    if [[ ! -f "$workers_config" ]]; then
        info "Generating example workers configuration..."
        cat > "$workers_config" << 'EOF'
# RCH Workers Configuration
# Define your remote compilation workers here.
# Uncomment and modify the examples below.

# Example worker definition:
# [[workers]]
# id = "server1"
# host = "192.168.1.100"
# user = "ubuntu"
# identity_file = "~/.ssh/id_rsa"
# total_slots = 16
# priority = 100
# tags = ["rust", "fast"]
# enabled = true

# [[workers]]
# id = "server2"
# host = "192.168.1.101"
# user = "ubuntu"
# identity_file = "~/.ssh/id_rsa"
# total_slots = 8
# priority = 80
# tags = ["rust"]
# enabled = true
EOF
        success "Created $workers_config"
        warn "Edit $workers_config to add your workers"
    else
        info "Workers config already exists: $workers_config"
    fi
}

# ============================================================================
# Claude Code Hook Integration
# ============================================================================

get_claude_code_settings_path() {
    local paths=(
        "$HOME/.claude/settings.json"
        "$HOME/.config/claude/settings.json"
        "$HOME/Library/Application Support/Claude/settings.json"
    )

    for path in "${paths[@]}"; do
        if [[ -f "$path" ]]; then
            echo "$path"
            return 0
        fi
    done

    echo "$HOME/.claude/settings.json"
}

configure_claude_hook() {
    if [[ "${ACTIVATE:-false}" != "true" && "${EASY_MODE:-false}" != "true" ]]; then
        info "Hooks unchanged (use --activate or --easy-mode to opt in)"
        return 0
    fi
    if [[ -n "${RCH_NO_HOOK:-}" ]]; then
        info "Skipping Claude Code hook setup (RCH_NO_HOOK set)"
        return 0
    fi
    # The built binary is the single settings writer. It validates JSON, rejects
    # unsafe files, preserves unrelated handlers and modes, and quotes its path.
    # No jq substring ownership heuristic and no destructive malformed-JSON repair.
    "$INSTALL_DIR/$HOOK_BIN" agents install-hook claude-code ||
        die "Hook activation refused; settings preserved. Preview legacy migration with: rch agents migrate --agent claude-code --dry-run"
}

# ============================================================================
# Service Opt-In Prompt
# ============================================================================

maybe_prompt_service() {
    if [[ "$MODE" == "worker" ]]; then
        ENABLE_SERVICE="false"
        return
    fi

    if [[ "${NO_SERVICE:-}" == "true" ]]; then
        ENABLE_SERVICE="false"
        return
    fi

    if system_rchd_service_active; then
        ENABLE_SERVICE="false"
        info "System-level rchd.service is active; skipping duplicate user daemon setup."
        return
    fi

    local service_manager=""
    service_manager="$(detect_service_manager 2>/dev/null || true)"
    if [[ -z "$service_manager" ]]; then
        ENABLE_SERVICE="false"
        if [[ "${EASY_MODE:-}" == "true" ]] || [[ "${YES:-}" == "true" ]] || [[ "${INSTALL_SERVICE:-}" == "true" ]]; then
            warn "Background service requested but no supported service manager detected; continuing without service."
        else
            info "No supported service manager detected - skipping background service setup"
        fi
        return
    fi

    if [[ "${INSTALL_SERVICE:-}" == "true" ]]; then
        ENABLE_SERVICE="true"
        return
    fi

    if [[ "${EASY_MODE:-}" == "true" ]] || [[ "${YES:-}" == "true" ]]; then
        ENABLE_SERVICE="true"
        return
    fi

    if ! is_interactive; then
        ENABLE_SERVICE="false"
        info "Non-interactive install without opt-in; skipping background daemon setup (use --install-service or --easy-mode)."
        return
    fi

    if confirm "Run rchd automatically in the background? (falls back to local if unavailable)"; then
        ENABLE_SERVICE="true"
    else
        ENABLE_SERVICE="false"
        info "Skipping background daemon setup (you can run 'rch daemon start' later)."
    fi
}

# ============================================================================
# Systemd/Launchd Service
# ============================================================================

setup_systemd_service() {
    if [[ "$MODE" == "worker" ]]; then
        return
    fi

    if ! systemd_user_available; then
        info "systemd user service unavailable - skipping service setup"
        return
    fi

    # Skip only if explicitly disabled
    if [[ "${NO_SERVICE:-}" == "true" ]]; then
        info "Skipping systemd service setup (--no-service)"
        return
    fi

    if [[ "${ENABLE_SERVICE:-true}" != "true" ]]; then
        info "Skipping systemd service setup (background daemon disabled)"
        return
    fi

    info "Setting up systemd service..."

    local service_file="$HOME/.config/systemd/user/rchd.service"
    mkdir -p "$(dirname "$service_file")"

    cat > "$service_file" << EOF
[Unit]
Description=RCH Remote Compilation Helper Daemon
After=network.target network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=$INSTALL_DIR/$DAEMON_BIN --foreground --workers-config $CONFIG_DIR/workers.toml
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
EOF

    systemctl --user daemon-reload
    systemctl --user enable --now rchd.service || systemctl --user enable rchd.service

    success "Systemd service installed: rchd.service"

    # Enable lingering for reboot persistence (requires sudo)
    local current_user
    current_user=$(whoami)
    if command_exists loginctl; then
        if loginctl show-user "$current_user" 2>/dev/null | grep -q "Linger=yes"; then
            info "Lingering already enabled for $current_user"
        else
            info "Enabling lingering for $current_user (for reboot persistence)..."
            if sudo -n loginctl enable-linger "$current_user" 2>/dev/null; then
                success "Lingering enabled - service will start on boot"
            else
                warn "Could not enable lingering automatically"
                warn "Run: sudo loginctl enable-linger $current_user"
            fi
        fi
    fi

    # Start or restart the service if it is not already running
    if ! systemctl --user is-active rchd.service &>/dev/null; then
        info "Starting rchd service..."
        if systemctl --user start rchd.service; then
            success "rchd service started"
        else
            warn "Could not start rchd service automatically"
            info "Start manually with: systemctl --user start rchd"
        fi
    fi

    info "Check status: systemctl --user status rchd"
    info "Follow logs:  journalctl --user -u rchd -f"
}

setup_launchd_service() {
    if [[ "$MODE" == "worker" ]]; then
        return
    fi

    if [[ "$(uname -s)" != "Darwin" ]]; then
        return
    fi

    if ! command_exists launchctl; then
        info "launchctl not found - skipping service setup"
        return
    fi

    # Skip only if explicitly disabled
    if [[ "${NO_SERVICE:-}" == "true" ]]; then
        info "Skipping launchd service setup (--no-service)"
        return
    fi

    if [[ "${ENABLE_SERVICE:-true}" != "true" ]]; then
        info "Skipping launchd service setup (background daemon disabled)"
        return
    fi

    info "Setting up launchd service..."

    local plist_file="$HOME/Library/LaunchAgents/com.rch.daemon.plist"
    mkdir -p "$(dirname "$plist_file")"
    mkdir -p "$CONFIG_DIR/logs"

    # Unload existing service if present
    if launchctl list 2>/dev/null | grep -q "com.rch.daemon"; then
        info "Unloading existing service..."
        launchctl unload "$plist_file" 2>/dev/null || true
    fi

    cat > "$plist_file" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.rch.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>$INSTALL_DIR/$DAEMON_BIN</string>
        <string>--foreground</string>
        <string>--workers-config</string>
        <string>$CONFIG_DIR/workers.toml</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>
    <key>StandardOutPath</key>
    <string>$CONFIG_DIR/logs/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>$CONFIG_DIR/logs/daemon.err</string>
</dict>
</plist>
EOF

    success "Launchd service installed: $plist_file"

    # Load and start the service
    info "Loading launchd service..."
    if launchctl load "$plist_file" 2>/dev/null; then
        success "rchd service loaded and started"
        info "Service will start automatically on login"
    else
        warn "Could not load launchd service automatically"
        info "Load manually with: launchctl load $plist_file"
    fi

    info "Check status: launchctl list | grep rch"
}

# ============================================================================
# PATH Setup
# ============================================================================

configure_dispatcher_shim() {
    DISPATCHER_SHIM_CONFIGURED=false
    [[ "$MODE" == "local" ]] || return 0
    local rch_bin config_dir role
    rch_bin=$(cd "$INSTALL_DIR" && pwd) || return 1
    rch_bin="$rch_bin/$HOOK_BIN"
    config_dir=$(cd "$CONFIG_DIR" && pwd) || return 1
    # Resolve the machine configuration with RCH's TOML parser, outside the
    # checkout that happened to invoke this installer. Never infer a role from
    # grep matches in comments or a project's override.
    if ! role=$(cd "$HOME" && env -u RCH_OUTPUT_FORMAT -u TOON_DEFAULT_FORMAT \
        RCH_CONFIG_DIR="$config_dir" RCH_JSON=0 \
        "$rch_bin" --color never config get general.role); then
        warn "Cannot determine machine role; dispatcher shim setup was not completed"
        warn "This installer requires a CLI with 'config get general.role' (the initial v2.0.0 release lacks it). Upgrade installer and binaries together, or fix the configuration error above."
        return 1
    fi
    [[ "$role" == "dispatcher" ]] || return 0
    info "Restoring dispatcher Cargo interception..."
    # A service manager does not inherit shell rc PATH. Establish the repair
    # process's own lookup before status; repair future shells separately below.
    export PATH="$HOME/.rch/shims:${rch_bin%/*}:$PATH"
    local shim_current=false
    if [[ "${RCH_SHIM_WATCHDOG:-}" == "1" ]]; then
        # Status is retained in the journal. Without jq we conservatively
        # reassert, rather than treating an unparsed response as healthy.
        local shim_status
        shim_status=$(cd "$HOME" && RCH_CONFIG_DIR="$config_dir" \
            "$rch_bin" --json shim status) || return 1
        printf '%s\n' "$shim_status"
        if command -v jq >/dev/null 2>&1 &&
            jq -e '(.data // .) | .installed and .up_to_date and
                .clippy_shim_installed and .on_path_ahead_of_cargo and
                (.toolchains_wrapped == .toolchains_total)' <<< "$shim_status" >/dev/null; then
            shim_current=true
        fi
    fi
    if [[ "$shim_current" != "true" ]]; then
        (cd "$HOME" && RCH_CONFIG_DIR="$config_dir" "$rch_bin" shim install) || return 1
    fi
    DISPATCHER_SHIM_CONFIGURED=true

    # The installer's environment cannot repair its parent's PATH. Persist the
    # prepend for new shells.
    if [[ "${RCH_NO_RC:-}" == "1" ]]; then
        info "RCH_NO_RC=1 set; skipping dispatcher shell rc modification"
        return 0
    fi
    local shell_rc path_line quoted_install
    local shell_files=()
    case "${SHELL:-/bin/bash}" in
        */zsh)
            shell_files=("${ZDOTDIR:-$HOME}/.zshenv" "${ZDOTDIR:-$HOME}/.zprofile" "${ZDOTDIR:-$HOME}/.zshrc")
            ;;
        */fish) shell_files=("${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish") ;;
        */bash)
            shell_files=("$HOME/.bashrc")
            # Bash reads only the first existing login file in this order.
            # Do not create .bash_profile and thereby shadow an existing .profile.
            if [[ -f "$HOME/.bash_profile" ]]; then
                shell_files+=("$HOME/.bash_profile")
            elif [[ -f "$HOME/.bash_login" ]]; then
                shell_files+=("$HOME/.bash_login")
            else
                shell_files+=("$HOME/.profile")
            fi
            ;;
        *) shell_files=("$HOME/.profile") ;;
    esac
    quoted_install="${rch_bin%/*}"
    quoted_install="${quoted_install//\'/\'\\\'\'}"
    if [[ "${SHELL:-/bin/bash}" == */fish ]]; then
        path_line="set -gx PATH \"\$HOME/.rch/shims\" '$quoted_install' \$PATH"
    else
        path_line="export PATH=\"\$HOME/.rch/shims\":'$quoted_install':\"\$PATH\""
    fi
    # Reassert at the end: an updater may have appended its own PATH prepend
    # after an older RCH line. Consecutive installs remain idempotent.
    for shell_rc in "${shell_files[@]}"; do
        mkdir -p "$(dirname "$shell_rc")"
        if [[ ! -f "$shell_rc" ]] || [[ "$(tail -n 1 "$shell_rc")" != "$path_line" ]]; then
            printf '\n# RCH dispatcher: keep Cargo offload ahead of toolchain binaries\n%s\n' "$path_line" >> "$shell_rc"
        fi
    done
    success "Dispatcher shim restored; shell startup files: ${shell_files[*]}"
    warn "Existing shells must refresh PATH or restart; this installer cannot change their environment"
}

install_dispatcher_shim_watchdog() {
    [[ "${DISPATCHER_SHIM_CONFIGURED:-false}" == "true" ]] || return 0
    local watchdog="$HOME/.rch/shim-watchdog" watchdog_tmp
    mkdir -p "$HOME/.rch"
    watchdog_tmp=$(mktemp "$HOME/.rch/shim-watchdog.XXXXXXXX") || return 1
    # Persist only this self-contained repair function, not the downloader or
    # installer entry point. %q preserves spaces and shell metacharacters.
    {
        printf '#!/usr/bin/env bash\nset -euo pipefail\n'
        printf 'MODE=local\nHOOK_BIN=rch\nRCH_SHIM_WATCHDOG=1\n'
        printf 'INSTALL_DIR=%q\nCONFIG_DIR=%q\n' "$(cd "$INSTALL_DIR" && pwd)" "$(cd "$CONFIG_DIR" && pwd)"
        printf 'SHELL=%q\n' "${SHELL:-/bin/bash}"
        printf 'ZDOTDIR=%q\n' "${ZDOTDIR:-$HOME}"
        printf 'RCH_NO_RC=%q\n' "${RCH_NO_RC:-0}"
        if [[ -n "${RUSTUP_HOME:-}" ]]; then
            printf 'export RUSTUP_HOME=%q\n' "$RUSTUP_HOME"
        fi
        if [[ -n "${XDG_CONFIG_HOME:-}" ]]; then
            printf 'export XDG_CONFIG_HOME=%q\n' "$XDG_CONFIG_HOME"
        fi
        printf 'info() { printf "%%s\\n" "$*"; }\n'
        printf 'warn() { printf "%%s\\n" "$*" >&2; }\n'
        printf 'success() { printf "%%s\\n" "$*"; }\n'
        declare -f configure_dispatcher_shim
        printf '\nconfigure_dispatcher_shim\n'
    } > "$watchdog_tmp"
    chmod 755 "$watchdog_tmp"
    mv -f "$watchdog_tmp" "$watchdog"
    if [[ "${NO_SERVICE:-false}" == "true" || "${ENABLE_SERVICE:-true}" != "true" ]]; then
        info "Dispatcher repair installed at $watchdog; timer disabled by service preference"
        return 0
    fi
    if ! systemd_user_available; then
        warn "No user systemd; schedule $watchdog to repair dispatcher drift periodically"
        return 0
    fi
    local unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
    mkdir -p "$unit_dir"
    if systemctl --user cat rch-fleet-watchdog.service >/dev/null 2>&1; then
        mkdir -p "$unit_dir/rch-fleet-watchdog.service.d"
        cat > "$unit_dir/rch-fleet-watchdog.service.d/50-rch-shim.conf" << 'EOF'
[Service]
ExecStartPost="%h/.rch/shim-watchdog"
EOF
        systemctl --user daemon-reload
        success "Dispatcher repair attached to each fleet-watchdog cycle"
        return 0
    fi
    cat > "$unit_dir/rch-shim-watchdog.service" << 'EOF'
[Unit]
Description=Restore dispatcher Cargo interception
[Service]
Type=oneshot
ExecStart="%h/.rch/shim-watchdog"
EOF
    cat > "$unit_dir/rch-shim-watchdog.timer" << 'EOF'
[Unit]
Description=Periodically restore dispatcher Cargo interception
[Timer]
OnBootSec=2min
OnUnitActiveSec=2min
[Install]
WantedBy=timers.target
EOF
    systemctl --user daemon-reload
    systemctl --user enable --now rch-shim-watchdog.timer
    success "Dispatcher repair timer enabled (two-minute interval)"
}

setup_path() {
    # Test harnesses and automation MUST set RCH_NO_RC=1: modifying the
    # user's real shell rc from a non-interactive/CI context is how a test
    # fixture's stub `rch` ended up shadowing the real binary on PATH for
    # five days (2026-07-11..16 incident). Never remove this guard.
    if [[ "${RCH_NO_RC:-}" == "1" ]]; then
        info "RCH_NO_RC=1 set; skipping shell rc PATH modification"
        return 0
    fi
    if [[ ":$PATH:" == *":$INSTALL_DIR:"* ]]; then
        info "$INSTALL_DIR already in PATH"
        return 0
    fi

    warn "$INSTALL_DIR is not in PATH"

    local shell_rc=""
    case "${SHELL:-/bin/bash}" in
        */bash) shell_rc="$HOME/.bashrc" ;;
        */zsh)  shell_rc="$HOME/.zshrc" ;;
        */fish) shell_rc="$HOME/.config/fish/config.fish" ;;
        *)      shell_rc="$HOME/.profile" ;;
    esac

    local path_line="export PATH=\"$INSTALL_DIR:\$PATH\""

    if [[ -f "$shell_rc" ]] && grep -qF "$INSTALL_DIR" "$shell_rc"; then
        info "PATH already configured in $shell_rc"
        return 0
    fi

    if [[ "${EASY_MODE:-}" == "true" ]] || confirm "Add $INSTALL_DIR to PATH in $shell_rc?"; then
        {
            echo ""
            echo "# RCH - Remote Compilation Helper"
            echo "$path_line"
        } >> "$shell_rc"
        success "PATH configured in $shell_rc"
        warn "Run 'source $shell_rc' or restart your shell"
    fi
}

# ============================================================================
# Shell Completions
# ============================================================================

setup_shell_completions() {
    if [[ "$MODE" == "worker" ]]; then
        return
    fi

    info "Setting up shell completions..."

    local rch_bin="$INSTALL_DIR/$HOOK_BIN"
    if [[ ! -x "$rch_bin" ]]; then
        warn "RCH binary not found, skipping completion setup"
        return
    fi

    local current_shell
    current_shell=$(basename "${SHELL:-}")

    case "$current_shell" in
        bash)
            if "$rch_bin" completions install bash 2>/dev/null; then
                success "Installed bash completions"
            else
                warn "Could not install bash completions"
            fi
            ;;
        zsh)
            if "$rch_bin" completions install zsh 2>/dev/null; then
                success "Installed zsh completions"
            else
                warn "Could not install zsh completions"
            fi
            ;;
        fish)
            if "$rch_bin" completions install fish 2>/dev/null; then
                success "Installed fish completions"
            else
                warn "Could not install fish completions"
            fi
            ;;
        *)
            info "Unknown shell ($current_shell), skipping completion setup"
            info "Run 'rch completions install <shell>' manually to set up completions"
            ;;
    esac
}

# ============================================================================
# Post-Install Doctor Check
# ============================================================================

run_doctor() {
    if [[ "${RCH_SKIP_DOCTOR:-}" == "1" ]]; then
        info "Skipping doctor check (RCH_SKIP_DOCTOR=1)"
        return 0
    fi

    if [[ "$MODE" == "worker" ]]; then
        return 0
    fi

    info "Running post-install diagnostics..."

    local rch_bin="$INSTALL_DIR/$HOOK_BIN"
    if [[ -x "$rch_bin" ]]; then
        if "$rch_bin" doctor 2>&1; then
            success "Doctor check passed"
        else
            warn "Doctor check reported issues (this may be expected on fresh install)"
        fi
    else
        warn "Cannot run doctor: rch binary not found"
    fi
}

# ============================================================================
# Post-Install Worker Harmonization
# ============================================================================

restart_daemon_if_needed() {
    if [[ "${RCH_SKIP_DAEMON_RESTART:-}" == "1" ]]; then
        info "Skipping daemon restart (RCH_SKIP_DAEMON_RESTART=1)"
        return 0
    fi

    if [[ "$MODE" == "worker" ]]; then
        return 0
    fi

    if system_rchd_service_active; then
        info "System-level rchd.service is active; skipping user-daemon restart."
        return 0
    fi

    local rch_bin="$INSTALL_DIR/$HOOK_BIN"
    if [[ ! -x "$rch_bin" ]]; then
        warn "Cannot restart daemon: rch binary not found"
        return 0
    fi

    info "Restarting rchd to apply updates..."
    if "$rch_bin" daemon restart 2>&1; then
        success "rchd restarted"
        return 0
    fi

    warn "rch daemon restart failed; trying service manager fallback"
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user restart rchd.service 2>/dev/null && {
            success "rchd restarted via systemd"
            return 0
        }
    fi
    if command -v launchctl >/dev/null 2>&1; then
        local plist="$HOME/Library/LaunchAgents/com.rch.daemon.plist"
        if [[ -f "$plist" ]]; then
            launchctl unload "$plist" 2>/dev/null || true
            if launchctl load "$plist" 2>/dev/null; then
                success "rchd restarted via launchctl"
                return 0
            fi
        fi
    fi
    warn "Could not restart rchd automatically"
}

harmonize_workers() {
    if [[ "${RCH_SKIP_FLEET_SYNC:-}" == "1" ]]; then
        info "Skipping worker fleet sync (RCH_SKIP_FLEET_SYNC=1)"
        return 0
    fi

    if [[ "$MODE" == "worker" ]]; then
        return 0
    fi

    local rch_bin="$INSTALL_DIR/$HOOK_BIN"
    if [[ ! -x "$rch_bin" ]]; then
        warn "Cannot sync workers: rch binary not found"
        return 0
    fi

    local workers_config="$CONFIG_DIR/workers.toml"
    if [[ ! -f "$workers_config" ]]; then
        info "No workers config found; skipping fleet sync"
        return 0
    fi

    if ! grep -q '^\s*\[\[workers\]\]' "$workers_config"; then
        warn "No workers defined in $workers_config; skipping fleet sync"
        return 0
    fi

    info "Upgrading worker fleet (rch-wkr)..."
    if "$rch_bin" fleet deploy --yes --verify 2>&1; then
        success "Worker fleet deployed"
    else
        warn "Fleet deploy failed (workers may be offline or misconfigured)"
    fi

    info "Verifying worker fleet..."
    if "$rch_bin" fleet verify 2>&1; then
        success "Worker fleet verified"
        return 0
    fi

    warn "Fleet verify failed; falling back to workers probe"
    if "$rch_bin" workers probe --all 2>&1; then
        success "Worker probe passed"
    else
        warn "Worker probe reported issues"
    fi
}

# ============================================================================
# Easy Mode: Agent Detection
# ============================================================================

detect_agents() {
    info "Detecting AI coding agents..."

    local errexit_enabled=false
    case "$-" in
        *e*) errexit_enabled=true; set +e ;;
    esac
    restore_errexit() {
        if $errexit_enabled; then
            set -e
        fi
    }

    local rch_bin="$INSTALL_DIR/$HOOK_BIN"
    if [[ -x "$rch_bin" ]]; then
        local detect_output=""
        if detect_output=$("$rch_bin" agents detect 2>&1); then
            [[ -n "$detect_output" ]] && echo "$detect_output"
            restore_errexit
            return 0
        fi

        if echo "$detect_output" | grep -qi "unrecognized subcommand"; then
            warn "Built-in agent detection unavailable; using heuristics"
        else
            warn "Agent detection failed; using heuristics"
            [[ -n "$detect_output" ]] && echo "$detect_output"
        fi
    fi

    # Manual detection
    local agents_found=0

    if command_exists claude; then
        success "Claude Code: detected"
        agents_found=$((agents_found + 1))
    fi

    if [[ -d "$HOME/.cursor" ]] || command_exists cursor; then
        success "Cursor: detected"
        agents_found=$((agents_found + 1))
    fi

    if command_exists codex; then
        success "Codex CLI: detected"
        agents_found=$((agents_found + 1))
    fi

    if [[ $agents_found -eq 0 ]]; then
        warn "No AI coding agents detected"
    fi

    restore_errexit
    return 0
}

# ============================================================================
# Uninstall
# ============================================================================

uninstall() {
    info "Uninstalling RCH..."

    # Stop daemon if running via systemd
    if command_exists systemctl; then
        systemctl --user stop rchd.service 2>/dev/null || true
        systemctl --user disable rchd.service 2>/dev/null || true
        rm -f "$HOME/.config/systemd/user/rchd.service"
        systemctl --user daemon-reload 2>/dev/null || true
    fi

    # Stop launchd service on macOS
    if [[ "$(uname -s)" == "Darwin" ]]; then
        local plist_file="$HOME/Library/LaunchAgents/com.rch.daemon.plist"
        if [[ -f "$plist_file" ]]; then
            launchctl unload "$plist_file" 2>/dev/null || true
            rm -f "$plist_file"
        fi
    fi

    # Remove binaries
    rm -f "$INSTALL_DIR/$HOOK_BIN"
    rm -f "$INSTALL_DIR/$DAEMON_BIN"
    rm -f "$INSTALL_DIR/$WORKER_BIN"

    # Remove socket
    rm -f "$SOCKET_PATH"

    success "Binaries removed"

    echo ""
    echo -e "${YELLOW}Config files preserved at: $CONFIG_DIR${NC}"
    echo "To remove config: rm -rf $CONFIG_DIR"
    echo ""
    echo "To remove Claude Code hook, edit your settings and remove the RCH hook entry."
}

# ============================================================================
# Verification
# ============================================================================

verify_installation() {
    info "Verifying installation..."

    local failed=0

    if [[ "$MODE" == "worker" ]]; then
        if [[ -x "$INSTALL_DIR/$WORKER_BIN" ]]; then
            local version
            version=$("$INSTALL_DIR/$WORKER_BIN" --version 2>/dev/null | head -1 || echo "unknown")
            success "$WORKER_BIN: $version"
        else
            error "Worker binary not found or not executable"
            ((failed++))
        fi
    else
        for binary in "$HOOK_BIN" "$DAEMON_BIN"; do
            local path="$INSTALL_DIR/$binary"
            if [[ -x "$path" ]]; then
                local version
                version=$("$path" --version 2>/dev/null | head -1 || echo "unknown")
                success "$binary: $version"
            else
                error "$binary: not found or not executable"
                ((failed++))
            fi
        done

        if [[ -x "$INSTALL_DIR/$WORKER_BIN" ]]; then
            local worker_version
            worker_version=$("$INSTALL_DIR/$WORKER_BIN" --version 2>/dev/null | head -1 || echo "unknown")
            success "$WORKER_BIN: $worker_version"
        else
            if [[ "${RCH_SKIP_FLEET_SYNC:-}" == "1" ]]; then
                warn "$WORKER_BIN: not installed (fleet sync disabled)"
            else
                error "$WORKER_BIN: not found or not executable"
                ((failed++))
            fi
        fi

        if [[ ! -f "$CONFIG_DIR/daemon.toml" ]]; then
            error "Daemon config not found"
            ((failed++))
        fi
    fi

    if [[ $failed -gt 0 ]]; then
        die "Installation verification failed with $failed errors"
    fi

    success "Installation verified"
}

# ============================================================================
# Summary
# ============================================================================

print_summary() {
    echo ""
    if $USE_GUM; then
        gum style \
            --border double \
            --border-foreground 82 \
            --padding "1 3" \
            --align center \
            "✅ Installation Complete!"
    else
        draw_box "1;32" \
            "Installation Complete!" \
            "" \
            "RCH is ready to use"
    fi
    echo ""

    # Installation details
    if $USE_COLOR; then
        echo -e "${BOLD}Installation Details:${NC}"
    else
        echo "Installation Details:"
    fi
    echo ""
    echo "  📁 Binaries:    $INSTALL_DIR"
    echo "  ⚙️  Config:      $CONFIG_DIR"
    echo ""

    if [[ "$MODE" == "worker" ]]; then
        echo "  🔧 Worker binary: $INSTALL_DIR/$WORKER_BIN"
    else
        echo "  📦 Installed:"
        echo "      • rch      (hook CLI)"
        echo "      • rchd     (daemon)"
        echo "      • rch-wkr  (worker)"
        echo ""

        if [[ "${ENABLE_SERVICE:-true}" == "true" ]] && [[ "${NO_SERVICE:-}" != "true" ]]; then
            if systemd_user_available; then
                # Check if daemon is actually running
                local daemon_status="unknown"
                if systemctl --user is-active rchd.service &>/dev/null; then
                    daemon_status="running"
                else
                    daemon_status="stopped"
                fi

                if [[ "$daemon_status" == "running" ]]; then
                    echo "  🔄 Service: rchd.service (systemd) - ✅ RUNNING"
                else
                    echo "  🔄 Service: rchd.service (systemd) - ⚠️ NOT RUNNING"
                fi
                echo ""
                if $USE_COLOR; then
                    echo -e "  ${DIM}Commands:${NC}"
                    echo -e "    ${CYAN}systemctl --user status rchd${NC}     # Check status"
                    echo -e "    ${CYAN}journalctl --user -u rchd -f${NC}     # Follow logs"
                    echo -e "    ${CYAN}systemctl --user restart rchd${NC}    # Restart"
                    echo -e "    ${CYAN}systemctl --user stop rchd${NC}       # Stop daemon"
                else
                    echo "  Commands:"
                    echo "    systemctl --user status rchd     # Check status"
                    echo "    journalctl --user -u rchd -f     # Follow logs"
                    echo "    systemctl --user restart rchd    # Restart"
                    echo "    systemctl --user stop rchd       # Stop daemon"
                fi
            elif [[ "$(uname -s)" == "Darwin" ]] && command_exists launchctl; then
                # Check if daemon is running on macOS
                local daemon_status="unknown"
                if launchctl list 2>/dev/null | grep -q "com.rch.daemon"; then
                    daemon_status="running"
                else
                    daemon_status="stopped"
                fi

                if [[ "$daemon_status" == "running" ]]; then
                    echo "  🔄 Service: com.rch.daemon (launchd) - ✅ RUNNING"
                else
                    echo "  🔄 Service: com.rch.daemon (launchd) - ⚠️ NOT RUNNING"
                fi
                echo ""
                if $USE_COLOR; then
                    echo -e "  ${DIM}Commands:${NC}"
                    echo -e "    ${CYAN}launchctl list | grep rch${NC}        # Check status"
                    echo -e "    ${CYAN}tail -f $CONFIG_DIR/logs/daemon.log${NC}"
                    echo -e "    ${CYAN}launchctl unload ~/Library/LaunchAgents/com.rch.daemon.plist${NC}  # Stop"
                else
                    echo "  Commands:"
                    echo "    launchctl list | grep rch        # Check status"
                    echo "    tail -f $CONFIG_DIR/logs/daemon.log"
                    echo "    launchctl unload ~/Library/LaunchAgents/com.rch.daemon.plist  # Stop"
                fi
            fi
            echo ""
        else
            echo "  🔄 Service: disabled"
            echo "     Run 'rch daemon start' when ready"
            echo ""
        fi

        # Next steps box
        if $USE_GUM; then
            gum style \
                --border rounded \
                --border-foreground 208 \
                --padding "0 2" \
                "Next Steps:" \
                "" \
                "1. Edit workers:   vim $CONFIG_DIR/workers.toml" \
                "2. Run diagnostics: rch doctor" \
                "3. Test workers:    rch workers probe --all"
        else
            draw_box "1;33" \
                "Next Steps:" \
                "" \
                "1. Edit workers:    vim $CONFIG_DIR/workers.toml" \
                "2. Run diagnostics: rch doctor" \
                "3. Test workers:    rch workers probe --all"
        fi
    fi
    echo ""

    # Supported commands section
    if [[ "$MODE" != "worker" ]]; then
        if $USE_COLOR; then
            echo -e "${BOLD}Supported Commands (auto-offloaded):${NC}"
        else
            echo "Supported Commands (auto-offloaded):"
        fi
        echo ""
        echo "  🦀 Rust:    cargo build, cargo test, cargo check, rustc"
        echo "  🍞 Bun:     bun test, bun typecheck"
        echo "  🔨 C/C++:   gcc, g++, clang, make, cmake, ninja"
        echo ""
        if $USE_COLOR; then
            echo -e "${DIM}Commands like 'cargo fmt', 'cargo install', 'ls' run locally.${NC}"
        else
            echo "Commands like 'cargo fmt', 'cargo install', 'ls' run locally."
        fi
        echo ""

        # AI Agent skill info
        local has_claude_skill=false
        local has_codex_skill=false
        [[ -f "$HOME/.claude/skills/rch/SKILL.md" ]] && has_claude_skill=true
        [[ -f "$HOME/.codex/skills/rch/SKILL.md" ]] && has_codex_skill=true

        if $has_claude_skill || $has_codex_skill; then
            local skill_locations=""
            $has_claude_skill && skill_locations+="Claude Code (~/.claude/skills/rch/)"
            if $has_codex_skill; then
                [[ -n "$skill_locations" ]] && skill_locations+=", "
                skill_locations+="Codex CLI (~/.codex/skills/rch/)"
            fi

            if $USE_GUM; then
                gum style \
                    --border rounded \
                    --border-foreground 141 \
                    --padding "0 2" \
                    "🤖 AI Agent Integration" \
                    "" \
                    "Skills installed for: $skill_locations" \
                    "" \
                    "Your AI coding agents now have RCH expertise built-in." \
                    "Just describe your issue naturally—no commands needed:" \
                    "" \
                    "  \"rch doctor is failing\"" \
                    "  \"no workers available\"" \
                    "  \"how do I configure a new worker?\"" \
                    "" \
                    "The agent's skill auto-activates on RCH topics."
            else
                draw_box "1;36" \
                    "AI Agent Integration" \
                    "" \
                    "Skills installed for:" \
                    "  $skill_locations" \
                    "" \
                    "Your AI coding agents now have RCH expertise built-in." \
                    "Just describe your issue naturally—no commands needed:" \
                    "" \
                    "  \"rch doctor is failing\"" \
                    "  \"no workers available\"" \
                    "  \"how do I configure a new worker?\"" \
                    "" \
                    "The agent's skill auto-activates on RCH topics."
            fi
            echo ""
        fi

        # Critical reminder about restarting Claude Code session
        if $USE_GUM; then
            gum style \
                --border double \
                --border-foreground 196 \
                --padding "1 3" \
                --align center \
                "!! IMPORTANT: Restart Required !!" \
                "" \
                "You MUST start a NEW Claude Code session" \
                "for the RCH hook to be active." \
                "" \
                "Exit your current session and run 'claude' again."
        else
            echo ""
            draw_double_box "1;31" \
                "IMPORTANT: Restart Required" \
                "" \
                "You MUST start a NEW Claude Code session" \
                "for the RCH hook to be active." \
                "" \
                "Exit your current session and run 'claude' again."
        fi
        echo ""
    fi
}

# ============================================================================
# Help
# ============================================================================

usage() {
    cat << EOF
RCH Installer v$VERSION

Usage: $0 [OPTIONS]

Options:
  --local              Install hook and daemon (default)
  --worker             Install worker agent only
  --from-source        Build from source (requires Rust)
  --version <version>  Pin release; source fallback uses only its tag
  --offline <tarball>  Install local tarball with adjacent .sha256 (airgap mode)
  --activate           Opt in to hooks and dispatcher PATH/shims (no legacy migration)
  --easy-mode          Activate + detect agents + run doctor
  --verify-only        Verify existing installation
  --install-service    Enable background service without prompting
  --no-service         Skip systemd/launchd service setup
  --uninstall          Remove RCH installation
  --no-gum             Disable Gum UI (use ANSI fallback)
  --yes                Skip confirmation prompts
  --help, -h           Show this help

Environment:
  RCH_REPOSITORY       GitHub owner/repo (default: tsunheimat/remote_compilation_helper)
  RCH_SIGNATURE_POLICY optional (default) or required; invalid signatures always fail
  RCH_INSTALL_DIR      Installation directory (default: ~/.local/bin)
  RCH_CONFIG_DIR       Config directory (default: ~/.config/rch)
  RCH_NO_HOOK          Skip Claude Code hook setup if set
  RCH_NO_COLOR         Disable colored output
  RCH_SKIP_DOCTOR      Skip post-install doctor check
  RCH_SKIP_DAEMON_RESTART  Skip rchd restart after install
  RCH_SKIP_FLEET_SYNC  Skip worker deploy/health checks after install
  HTTP_PROXY           HTTP proxy URL
  HTTPS_PROXY          HTTPS proxy URL
  NO_PROXY             Hosts to bypass proxy

Service Setup:
  On Linux:  systemd user service is offered during install (auto-accept in --easy-mode/--yes).
             Use --install-service to opt in without prompting.
             An active system-level rchd.service suppresses duplicate user-daemon setup/restart.
             Lingering is enabled for reboot persistence when possible (requires sudo).
  On macOS:  launchd service is offered during install (auto-accept in --easy-mode/--yes).
             Use --install-service to opt in without prompting.
             Service starts on login when enabled.

Examples:
  ./install.sh                         # Install locally (auto-detects source)
  ./install.sh --from-source           # Force build from source
  ./install.sh --worker                # Install on a worker machine
  ./install.sh --easy-mode             # Full setup with PATH and detection
  ./install.sh --offline rch.tar.gz    # Install from local tarball
  ./install.sh --install-service       # Enable background service without prompt
  ./install.sh --no-service            # Install without system service
  ./install.sh --uninstall             # Remove installation

EOF
}

# ============================================================================
# Main
# ============================================================================

main() {
    # Parse arguments
    MODE="local"
    FROM_SOURCE="false"
    UNINSTALL="false"
    VERIFY_ONLY="false"
    EASY_MODE="false"
    ACTIVATE="false"
    NO_SERVICE="false"
    INSTALL_SERVICE="false"
    ENABLE_SERVICE="true"
    YES="false"
    OFFLINE_TARBALL=""

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --local)
                MODE="local"
                shift
                ;;
            --worker|--worker-only)
                MODE="worker"
                shift
                ;;
            --version)
                [[ -n "${2:-}" ]] || die "--version requires a release version"
                VERSION="$2"
                VERSION_EXPLICIT=true
                shift 2
                ;;
            --from-source)
                FROM_SOURCE="true"
                shift
                ;;
            --offline)
                if [[ -n "${2:-}" ]]; then
                    OFFLINE_TARBALL="$2"
                    shift 2
                else
                    die "--offline requires a tarball path"
                fi
                ;;
            --activate)
                ACTIVATE="true"
                shift
                ;;
            --easy-mode)
                ACTIVATE="true"
                EASY_MODE="true"
                shift
                ;;
            --verify-only)
                VERIFY_ONLY="true"
                shift
                ;;
            --install-service)
                INSTALL_SERVICE="true"
                shift
                ;;
            --no-service)
                NO_SERVICE="true"
                shift
                ;;
            --uninstall)
                UNINSTALL="true"
                shift
                ;;
            --no-gum)
                NO_GUM="true"
                shift
                ;;
            --yes|-y)
                YES="true"
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done

    # Setup UI
    setup_ui

    # Reject malformed publisher/version/policy before any network or installation.
    validate_release_configuration
    # Detect target version (needed for header and upgrade banner)
    detect_target_version
    validate_release_configuration

    show_header

    # Handle special modes
    if [[ "$UNINSTALL" == "true" ]]; then
        uninstall
        exit 0
    fi

    if [[ "$VERIFY_ONLY" == "true" ]]; then
        verify_installation
        exit $?
    fi

    # Acquire lock
    acquire_lock

    # Never let a failed release lookup downgrade a working install
    enforce_no_downgrade
    if [[ "$SKIP_INSTALL_NO_DOWNGRADE" == "true" ]]; then
        success "rch v${VERSION} is already installed and newer than the resolved target; nothing to do."
        exit 0
    fi

    # Check for existing installation (shows upgrade banner)
    check_existing_install || true

    info "Installation mode: $MODE"
    echo ""

    # Setup proxy
    setup_proxy

    # Detect platform
    detect_platform

    # Worker mode toolchain verification
    if [[ "$MODE" == "worker" ]]; then
        verify_worker_toolchain || exit 1
    fi

    # Main installation
    check_prerequisites
    create_directories
    install_binaries
    generate_default_config

    if [[ "$MODE" == "local" ]]; then
        configure_claude_hook
        install_skill
        maybe_prompt_service
        setup_systemd_service
        setup_launchd_service
    fi

    if [[ "$ACTIVATE" == "true" ]]; then
        setup_path
        configure_dispatcher_shim
        install_dispatcher_shim_watchdog
    else
        info "PATH and shims unchanged; activate explicitly after acceptance with --activate"
    fi
    setup_shell_completions
    verify_installation

    # Easy mode extras
    if [[ "$EASY_MODE" == "true" ]]; then
        detect_agents
        restart_daemon_if_needed
        harmonize_workers
        run_doctor
    fi

    print_summary
}

if [[ "${RCH_INSTALLER_LIB:-}" == "1" ]]; then
    # shellcheck disable=SC2317
    return 0 2>/dev/null || exit 0
fi

main "$@"
