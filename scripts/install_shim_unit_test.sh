#!/usr/bin/env bash
# Isolated HOME integration of install.sh with a real, already-built RCH binary.
# Does not compile, alter real toolchains, start services, or delete fixtures.
# Usage: bash scripts/install_shim_unit_test.sh /absolute/path/to/rch
set -euo pipefail
project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
rch_test_binary=${1:?pass an already-built RCH binary}
[[ "$rch_test_binary" = /* && -x "$rch_test_binary" ]]
fixture=$(mktemp -d "${TMPDIR:-/tmp}/rch-install-shim.XXXXXXXX")
printf 'Retained installer fixture: %s\n' "$fixture"
export HOME="$fixture/home" RUSTUP_HOME="$fixture/rustup"
export XDG_CONFIG_HOME="$fixture/xdg-config" XDG_CACHE_HOME="$fixture/xdg-cache"
export XDG_DATA_HOME="$fixture/xdg-data" ZDOTDIR="$HOME"
export RCH_CONFIG_DIR="$fixture/config" RCH_INSTALL_DIR="$fixture/install bin's"
export RCH_INSTALLER_LIB=1 RCH_NO_UPDATE_CHECK=1
export RCH_NO_SELF_HEALING=1 RCH_SOCKET_PATH="$fixture/missing-daemon.sock"
mkdir -p "$HOME" "$RUSTUP_HOME" "$RCH_CONFIG_DIR" "$RCH_INSTALL_DIR"
ln -s "$rch_test_binary" "$RCH_INSTALL_DIR/rch"
# shellcheck source=../install.sh
source "$project_root/install.sh"
export MODE=local
export SHELL=/bin/bash
[[ "$(cd "$project_root" && source_toolchain_channel)" == "nightly-2026-08-31" ]]

# Exercise the installer entrypoint against the real Rust settings writer. Never
# touch the caller's HOME, never compile, and retain all fixtures for inspection.
mkdir -p "$HOME/.claude"
printf '{ malformed user settings\n' > "$HOME/.claude/settings.json"
ACTIVATE=false EASY_MODE=false configure_claude_hook
[[ "$(cat "$HOME/.claude/settings.json")" == '{ malformed user settings' ]]
[[ "$(find "$HOME/.claude" -type f | wc -l)" -eq 1 ]]
if (ACTIVATE=true configure_claude_hook); then
    printf 'FAIL: explicit activation accepted malformed JSON\n' >&2
    exit 1
fi
[[ "$(cat "$HOME/.claude/settings.json")" == '{ malformed user settings' ]]
printf '%s\n' '{"secret":"keep","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"audit"}]}]}}' > "$HOME/.claude/settings.json"
chmod 600 "$HOME/.claude/settings.json"
ACTIVATE=true configure_claude_hook
jq -e '.secret == "keep" and .hooks.PreToolUse[0].hooks[0].command == "audit" and (.hooks.PreToolUse | length) == 2' "$HOME/.claude/settings.json"
[[ "$(stat -c '%a' "$HOME/.claude/settings.json")" == 600 ]]
settings_digest=$(sha256sum "$HOME/.claude/settings.json")
ACTIVATE=true configure_claude_hook
[[ "$(sha256sum "$HOME/.claude/settings.json")" == "$settings_digest" ]]
printf 'PASS: hooks require opt-in; compiled installer preserves malformed JSON, unrelated settings, modes and idempotence\n'
printf '[general]\nrole = "dispatcher"\n' > "$CONFIG_DIR/config.toml"
printf 'workers = []\n' > "$CONFIG_DIR/workers.toml"
doctor_shim_check() {
    local expected=$1 report exit_status=0
    report="$fixture/doctor-$1-$(date +%s%N).json"
    (cd "$HOME" && "$rch_test_binary" --json --no-self-healing doctor) > "$report" || exit_status=$?
    [[ "$exit_status" -le 2 ]]
    jq -e --arg expected "$expected" '.data.checks | map(select(.name == "dispatcher_shim")) |
        length == 1 and .[0].status == $expected and .[0].fix_applied == false' "$report"
}
# No compiler is involved: fake local cargo is a PATH resolution sentinel only.
mkdir -p "$fixture/local-tools"
printf '#!/bin/sh\nprintf LOCAL_SENTINEL\\n\n' > "$fixture/local-tools/cargo"
chmod +x "$fixture/local-tools/cargo"
mkdir -p "$RUSTUP_HOME/toolchains/fixture/bin"
cp "$fixture/local-tools/cargo" "$RUSTUP_HOME/toolchains/fixture/bin/cargo"
original_path="$RCH_INSTALL_DIR:$fixture/local-tools:$PATH"
export PATH="$original_path"
doctor_shim_check warning
configure_dispatcher_shim
[[ -x "$HOME/.rch/shims/cargo" && -x "$HOME/.rch/shims/cargo-clippy" ]]
"$rch_test_binary" --json shim status | jq -e '.up_to_date == true'
doctor_shim_check pass
[[ "$(PATH="$original_path" bash --noprofile --norc -c '. "$HOME/.bashrc"; command -v cargo')" == "$HOME/.rch/shims/cargo" ]]
[[ ! -e "$HOME/.bash_profile" ]]
[[ "$(PATH="$original_path" bash --noprofile --norc -c '. "$HOME/.profile"; exec bash --noprofile --norc -c "command -v cargo"')" == "$HOME/.rch/shims/cargo" ]]
first_rc=$(cat "$HOME/.bashrc")
configure_dispatcher_shim
[[ "$(cat "$HOME/.bashrc")" == "$first_rc" ]]
printf 'PASS: dispatcher install persists first Cargo PATH entry and is idempotent\n'

# Simulate an updater appending a competing PATH entry. Preserve all prior text.
printf '\nexport PATH="%s:$PATH"\n' "$fixture/local-tools" >> "$HOME/.bashrc"
[[ "$(PATH="$original_path" bash --noprofile --norc -c '. "$HOME/.bashrc"; command -v cargo')" == "$fixture/local-tools/cargo" ]]
configure_dispatcher_shim
[[ "$(PATH="$original_path" bash --noprofile --norc -c '. "$HOME/.bashrc"; command -v cargo')" == "$HOME/.rch/shims/cargo" ]]
grep -F "$fixture/local-tools" "$HOME/.bashrc"
printf 'PASS: repeated install repairs updater PATH drift without discarding rc text\n'

before_profile=$(cat "$HOME/.profile")
printf '# Existing login profile owned by the user\n' > "$HOME/.bash_profile"
configure_dispatcher_shim
[[ "$(cat "$HOME/.profile")" == "$before_profile" ]]
[[ "$(PATH="$original_path" bash --noprofile --norc -c '. "$HOME/.bash_profile"; command -v cargo')" == "$HOME/.rch/shims/cargo" ]]
printf 'PASS: Bash uses an existing login profile without shadowing or rewriting .profile\n'

before_rc=$(cat "$HOME/.bashrc")
export RCH_NO_RC=1
configure_dispatcher_shim
[[ "$(cat "$HOME/.bashrc")" == "$before_rc" ]]
unset RCH_NO_RC
printf 'PASS: RCH_NO_RC preserves shell configuration\n'

export NO_SERVICE=true
install_dispatcher_shim_watchdog
[[ -x "$HOME/.rch/shim-watchdog" ]]
[[ ! -e "$XDG_CONFIG_HOME/systemd/user/rch-shim-watchdog.timer" ]]
before_inode=$(stat -c '%i' "$HOME/.rch/shims/cargo")
PATH="$original_path" "$HOME/.rch/shim-watchdog"
PATH="$original_path" "$HOME/.rch/shim-watchdog"
[[ "$(stat -c '%i' "$HOME/.rch/shims/cargo")" == "$before_inode" ]]
[[ "$(cat "$HOME/.bashrc")" == "$before_rc" ]]
printf 'PASS: healthy watchdog cycles preserve the shim inode and shell rc\n'
printf '\nexport PATH="%s:$PATH"\n' "$fixture/local-tools" >> "$HOME/.bashrc"
chmod 644 "$HOME/.rch/shims/cargo"
PATH="$original_path" "$HOME/.rch/shim-watchdog"
[[ -x "$HOME/.rch/shims/cargo" ]]
[[ "$(PATH="$original_path" bash --noprofile --norc -c '. "$HOME/.bashrc"; command -v cargo')" == "$HOME/.rch/shims/cargo" ]]
before_rc=$(cat "$HOME/.bashrc")
printf 'PASS: a watchdog cycle repairs nonexecutable shim and updater PATH drift\n'
chmod 644 "$HOME/.rch/shims/cargo-clippy"
"$rch_test_binary" --json shim status | jq -e '.up_to_date == false'
"$HOME/.rch/shim-watchdog"
[[ -x "$HOME/.rch/shims/cargo-clippy" ]]
"$rch_test_binary" --json shim status | jq -e '.up_to_date == true'
printf 'PASS: watchdog repairs a nonexecutable companion Clippy shim\n'
printf '#!/bin/sh\n# rch-toolchain-wrap-version: 1\nprintf STALE\\n\n' > "$RUSTUP_HOME/toolchains/fixture/bin/cargo"
"$rch_test_binary" --json shim status | jq -e '.up_to_date == false'
env -u RUSTUP_HOME "$HOME/.rch/shim-watchdog"
"$rch_test_binary" --json shim status | jq -e '.up_to_date == true and .toolchains_wrapped == 1'
printf 'PASS: watchdog refreshes a stale absolute-path toolchain wrapper\n'

if command -v zsh >/dev/null 2>&1; then
    export SHELL=/bin/zsh
    configure_dispatcher_shim
    [[ "$(PATH="$original_path" zsh -c 'command -v cargo')" == "$HOME/.rch/shims/cargo" ]]
    export SHELL=/bin/bash
    printf 'PASS: fresh Zsh resolves the installed shim first\n'
fi

for role in worker hybrid; do
    printf '[general]\nrole = "%s"\n' "$role" > "$CONFIG_DIR/config.toml"
    before_shim=$(sha256sum "$HOME/.rch/shims/cargo")
    configure_dispatcher_shim
    "$HOME/.rch/shim-watchdog"
    doctor_shim_check pass
    [[ "$(sha256sum "$HOME/.rch/shims/cargo")" == "$before_shim" ]]
    [[ "$(cat "$HOME/.bashrc")" == "$before_rc" ]]
done
printf 'PASS: worker and hybrid roles are not changed\n'

printf '[general]\nrole = "not-a-role"\n' > "$CONFIG_DIR/config.toml"
if configure_dispatcher_shim; then
    printf 'FAIL: invalid role reported success\n' >&2
    exit 1
fi
[[ "$(cat "$HOME/.bashrc")" == "$before_rc" ]]
printf 'PASS: invalid config refuses without touching shell configuration\n'

# Unit-test service routing without invoking the real user manager. This is
# deliberately not evidence of a live systemd timer or fleet deployment.
printf '[general]\nrole = "dispatcher"\n' > "$CONFIG_DIR/config.toml"
configure_dispatcher_shim
export NO_SERVICE=false ENABLE_SERVICE=true
systemd_user_available() { return 0; }
systemctl() {
    printf '%s\n' "$*" >> "$fixture/systemctl-calls"
    if [[ "$*" == '--user cat rch-fleet-watchdog.service' ]]; then
        [[ "${fixture_existing_fleet:-false}" == true ]]
    fi
}
install_dispatcher_shim_watchdog
grep -Fx 'ExecStart="%h/.rch/shim-watchdog"' "$XDG_CONFIG_HOME/systemd/user/rch-shim-watchdog.service"
grep -Fx 'OnUnitActiveSec=2min' "$XDG_CONFIG_HOME/systemd/user/rch-shim-watchdog.timer"
grep -Fx -- '--user enable --now rch-shim-watchdog.timer' "$fixture/systemctl-calls"
fixture_existing_fleet=true
before_timer=$(cat "$XDG_CONFIG_HOME/systemd/user/rch-shim-watchdog.timer")
install_dispatcher_shim_watchdog
grep -Fx 'ExecStartPost="%h/.rch/shim-watchdog"' "$XDG_CONFIG_HOME/systemd/user/rch-fleet-watchdog.service.d/50-rch-shim.conf"
[[ "$(cat "$XDG_CONFIG_HOME/systemd/user/rch-shim-watchdog.timer")" == "$before_timer" ]]
printf 'PASS: unit fixture routes repair into existing fleet service or creates fallback timer\n'

# Generic frontends are an explicit, independent install: dry runs and status
# cannot mutate Cargo, shell configuration, or toolchains.
generic_home="$fixture/generic home's"
mkdir -p "$generic_home"
HOME="$generic_home" "$rch_test_binary" --json shim install --tools all --dry-run |
    jq -e '.dry_run == true and (.tools | length) == 20 and (.tools | index("rtk")) != null'
[[ ! -e "$generic_home/.rch" ]]
HOME="$generic_home" "$rch_test_binary" --json shim status |
    jq -e '(.generic_tools | length) == 20 and all(.generic_tools[]; .installed == false)'
[[ ! -e "$generic_home/.rch" ]]
HOME="$generic_home" "$rch_test_binary" shim install --tools all
for tool in rtk go swift npm pnpm bun pytest python python3 node make npx bunx vitest jest tsc eslint prettier playwright vite; do
    [[ -x "$generic_home/.rch/shims/$tool" ]]
done
[[ ! -e "$generic_home/.rch/shims/cargo" && ! -e "$generic_home/.bashrc" ]]
HOME="$generic_home" "$rch_test_binary" --json shim status |
    jq -e 'all(.generic_tools[]; .installed == true and .up_to_date == true)'
printf 'PASS: generic install is opt-in, complete, and dry-run/status are read-only\n'

collision_home="$fixture/collision-home"
mkdir -p "$collision_home/.rch/shims"
printf 'USER OWNED\n' > "$collision_home/.rch/shims/python"
if HOME="$collision_home" "$rch_test_binary" shim install --tools go,python; then
    printf 'FAIL: generic installer overwrote an unowned executable\n' >&2
    exit 1
fi
[[ ! -e "$collision_home/.rch/shims/go" ]]
grep -Fx 'USER OWNED' "$collision_home/.rch/shims/python"
printf 'PASS: collisions refuse before any generic shim is written\n'

# Query the actual CLI dispatcher for ordinary interpreter launchers. No real
# language runtime executes: these record the exact argv and never compile.
mkdir -p "$fixture/generic-real" "$fixture/generic-rch"
ln -s "$rch_test_binary" "$fixture/generic-rch/rch"
for tool in python python3 node; do
    printf '#!/bin/sh\nprintf "%%s\\0" "$@"\n' > "$fixture/generic-real/$tool"
    chmod +x "$fixture/generic-real/$tool"
    HOME="$generic_home" PATH="$generic_home/.rch/shims:$fixture/generic-rch:$fixture/generic-real:/usr/bin:/bin" \
        "$generic_home/.rch/shims/$tool" ordinary-script 'two words' '' '$(false)' > "$fixture/$tool-argv"
    printf '%s\0' ordinary-script 'two words' '' '$(false)' > "$fixture/$tool-expected"
    cmp "$fixture/$tool-expected" "$fixture/$tool-argv"
done
printf 'PASS: ordinary Python and Node launches preserve exact argv through real dispatch\n'
