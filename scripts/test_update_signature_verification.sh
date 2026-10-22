#!/usr/bin/env bash
# Offline updater trust-policy regression checks. No download/install operations.
set -euo pipefail

case "${1:-}" in
  -h|--help)
    printf '%s\n' 'Usage: scripts/test_update_signature_verification.sh [--source-only]' \
      'Checks fork trust/source policy without network. Default also runs offline Rust unit tests.' \
      '--source-only checks source contracts; it is NOT cryptographic verification.'
    exit 0 ;;
  ''|--source-only) ;;
  *) printf 'Unknown option: %s\n' "$1" >&2; exit 2 ;;
esac

PROJECT_ROOT=$(git rev-parse --show-toplevel)
python3 -I - "$PROJECT_ROOT" <<'PY'
from pathlib import Path
import re
import sys

root = Path(sys.argv[1])
verify = (root / 'rch/src/update/verify.rs').read_text()
check = (root / 'rch/src/update/check.rs').read_text()
download = (root / 'rch/src/update/download.rs').read_text()
workflow = (root / '.github/workflows/release.yml').read_text()
production_verify = verify.split('#[cfg(test)]')[0]
production_check = check.split('#[cfg(test)]')[0]
production_download = download.split('#[cfg(test)]')[0]

def require(condition, message):
    if not condition:
        raise SystemExit('FAIL: ' + message)
    print('PASS: ' + message)

repo = 'tsunheimat/remote_compilation_helper'
require('const REPO_OWNER: &str = "tsunheimat";' in check, 'fork release lookup')
require('version_check-{REPO_OWNER}-{REPO_NAME}.json' in check,
        'cache namespace does not reuse legacy upstream file')
require('cached.is_valid() && validate_release_source(&cached.result).is_ok()' in check,
        'cached release metadata is source-validated')
require('super::check::validate_release_source(update)?;' in download,
        'download validates metadata source before requesting assets')
require('Dicklesworthstone' not in production_check + production_verify + production_download,
        'upstream repository is not production update authority')
require('RELEASE_MINISIGN_PUBLIC_KEY' not in production_verify and
        'verify_minisign' not in production_verify + production_download and
        '.minisig' not in production_download,
        'no inherited minisign trust or fetched release key')

match = re.search(r'const RCH_RELEASE_IDENTITY_PATTERN: &str = r"([^"]+)";', verify)
require(match is not None, 'explicit release identity policy exists')
pattern = match[1]
require(pattern.startswith('^') and pattern.endswith('$'), 'identity policy anchored at both ends')
identity = f'https://github.com/{repo}/.github/workflows/release.yml@refs/tags/v1.2.3'
for candidate in [identity, identity + '-beta.1']:
    require(re.search(pattern, candidate) is not None, f'accept canonical identity {candidate}')
for candidate in [
    identity.replace('tsunheimat', 'Dicklesworthstone'),
    identity.replace('tsunheimat', 'attacker'),
    'evil' + identity,
    identity + '/evil',
    identity + '?evil',
    identity.replace('github.com', 'github.com.evil.example'),
    identity.replace('release.yml', 'other.yml'),
    identity.replace('refs/tags/v1.2.3', 'refs/heads/main'),
    identity.replace('refs/tags/v1.2.3', 'refs/heads/feature'),
    identity.replace('refs/tags/v1.2.3', 'refs/pull/1/merge'),
]:
    require(re.search(pattern, candidate) is None, f'reject foreign identity {candidate}')
require('.arg("--certificate-identity")' in production_verify and
        '.arg("--certificate-oidc-issuer")' in production_verify and
        '"https://token.actions.githubusercontent.com"' in production_verify,
        'cosign requires exact tag identity and GitHub Actions issuer')
require('Some((bundle_path, &release_tag))' in download and
        'let release_tag = exact_release_tag(update)?;' in download and
        'Some(&update.latest_version)' in download,
        'signature identity preserves published tag spelling and checks selected version')
require('require_signature_support(' in production_download and
        'required Sigstore bundle missing' in production_download and
        'signature verification is mandatory' in production_download,
        'missing signing/verifier fails closed without explicit override')
require('verification.signature_valid.or(' not in download,
        'no alternate upstream signature authorization')
require('Dicklesworthstone/remote_compilation_helper' not in workflow and
        '$Repo = "${{ github.repository }}"' in workflow,
        'generated installer and release URLs follow publishing repository')
require('--certificate-identity-regexp=".*"' not in workflow and
        '--certificate-oidc-issuer-regexp=".*"' not in workflow and
        '--certificate-identity="https://github.com/${{ github.repository }}/.github/workflows/release.yml@refs/tags/${{ needs.plan.outputs.tag }}"' in workflow,
        'release instructions require exact workflow/tag identity')
require('"$GITHUB_REF" != "refs/tags/$TAG"' in workflow,
        'manual dispatch must use signing tag, not branch')
print('Source contract checks passed; these are not live signing evidence.')
PY

if [[ "${1:-}" == --source-only ]]; then
    exit 0
fi

cd "$PROJECT_ROOT"
# Real cosign fixture tests are a separate live tier, not hermetic proof.
# Offline Cargo prevents fetching dependencies, and filters avoid network tests.
for filter in update::check:: update::download:: update::verify:: update::tests::update_verification_; do
    cargo test --offline --locked -p rch --bin rch "$filter" -- \
      --skip integration_signature_verifies_with_real_cosign \
      --skip test_signature_invalid_bundle_returns_err
done
