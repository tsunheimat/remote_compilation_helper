#!/usr/bin/env bash
# ACFS manifest index (auto-generated).
# Format: "<sha256>  <path>" (sha256sum -c compatible)
# Usage:
#   ./manifest_index.sh --print
#   ./manifest_index.sh --verify

set -euo pipefail

manifest_entries() {
  cat <<'MANIFEST_EOF'
a5c1ad85f774beb511cc01eb281efde2fd41083157a1f0dfc98548784157f3fc  .claude/skills/rch/SKILL.md
e072ca840e44150f17a915fe453f70327fa78f38917c639352d7bde3ad867ac2  .claude/skills/rch/assets/workers-template.toml
b01079677fac993511f2a674f1c41122149f46f99bc853e61528c711a08d4ffd  .claude/skills/rch/references/COMMANDS.md
28d9c2017adf2b663c97a173a41010199972b5f2561ec4a89ddd5642990c108c  .claude/skills/rch/references/CONFIGURATION.md
44dabc9368f1a1940480012e5c6e2918d50d53e13591ce587ad364f19b8bbb2f  .claude/skills/rch/references/HOOKS.md
fa21e033180a535de40fe6a46892fb8a3d00849a8fcb1c6db4cc77097ac3ccbf  .claude/skills/rch/references/OPERATIONS.md
a64aaae56172d825eda9a2b9a02b26ff1aacfd03b1b3d7bfbaa8e46651663987  .claude/skills/rch/references/TROUBLESHOOTING.md
a45897450ae751863361fe6b462914b47a418ebbbd3a08c702a58d26a1b58d65  .claude/skills/rch/references/WORKERS.md
b81a9e8d94e4d6e17a4fe8a8d2719cb34815a80e3f31a27eea0174dec351aa48  .claude/skills/rch/scripts/validate-setup.sh
737d8b37b12f7003c90f07a7d62645fcb8035286f6148b5aeea2cdf68d2c3900  .claude/skills/remote-compilation-helper-setup/SKILL.md
b789a127cf6c1274d3ccb7e16b389613e3c058cbe91852d983a3be08bc9f0138  .claude/skills/remote-compilation-helper-setup/assets/workers-template.toml
a9d2b280dc866987029a757debb2f507cd048638ef0ea1d18b2cc8a21a5f22bd  .claude/skills/remote-compilation-helper-setup/references/HOOKS.md
2c6286a6d5f8289c3c7046bbdfa9669205ed7ae616fa38d10622d835f9583305  .claude/skills/remote-compilation-helper-setup/references/TROUBLESHOOTING.md
0dfdc37e17a39a2dbefc096935553df2234efe7c84132f6213f7e9266be872de  .claude/skills/remote-compilation-helper-setup/references/WORKERS.md
21d13636cc465aeeceef2271361666216e72f51dd20edbc9d84d79acda71e8f5  .claude/skills/remote-compilation-helper-setup/scripts/validate-setup.sh
MANIFEST_EOF
}

case "${1:---print}" in
  --print)
    manifest_entries
    ;;
  --verify)
    manifest_entries | sha256sum -c -
    ;;
  *)
    echo "Usage: $0 [--print|--verify]" >&2
    exit 2
    ;;
esac
