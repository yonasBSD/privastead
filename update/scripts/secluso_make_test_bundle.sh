#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<EOF
Usage:
  $0 --tag vX.Y.Z --workdir ./release_work --labels L1,L2 [--test-gnupg-home /tmp/secluso_test_gnupg]

Creates a realistic builder-style bundle with dummy binaries:
  <workdir>/<tag>/builder_out/manifest.json
  <workdir>/<tag>/builder_out/{aarch64-unknown-linux-gnu,x86_64-unknown-linux-gnu}/...
  <workdir>/<tag>/manifest.json + manifest.sha256   (manager copy + hash)
  <workdir>/<tag>/sigs/                             (both .asc)
  <workdir>/<tag>/out/secluso-<tag>.zip             (to be uploaded as test release)

Reuses the same two test keys across runs by keeping GNUPGHOME stable.
EOF
  exit 1
}

TAG=""
WORKDIR=""
LABELS=""
TEST_GNUPG_HOME="/tmp/secluso_test_gnupg"

# Validate args
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag) TAG="$2"; shift 2;;
    --workdir) WORKDIR="$2"; shift 2;;
    --labels) LABELS="$2"; shift 2;;
    --test-gnupg-home) TEST_GNUPG_HOME="$2"; shift 2;;
    -h|--help) usage;;
    *) echo "Unknown arg: $1" >&2; usage;;
  esac
done

[[ -n "$TAG" && -n "$WORKDIR" && -n "$LABELS" ]] || usage
IFS=',' read -r L1 L2 <<<"$LABELS"
[[ -n "${L1:-}" && -n "${L2:-}" ]] || { echo "Need --labels L1,L2" >&2; exit 1; }
[[ "$L1" != "$L2" ]] || { echo "Labels must be distinct" >&2; exit 1; }

# Require a v-prefixed tag
if [[ "$TAG" != v* ]]; then
  echo "Tag must start with 'v' (example: v0.1.0). Got: $TAG" >&2
  exit 1
fi

REL_DIR="$WORKDIR/$TAG"
mkdir -p "$REL_DIR"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Stable test keyring
GNUPG_HOME="$TEST_GNUPG_HOME"
mkdir -p "$GNUPG_HOME"
chmod 700 "$GNUPG_HOME"
export GNUPGHOME="$GNUPG_HOME"

# Sha256 helper (mac + linux)
sha256_file() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | awk '{print $1}'
  else
    shasum -a 256 "$f" | awk '{print $1}'
  fi
}

# Create fake "builder output" directory
ARTIFACT_DIR="$REL_DIR/builder_out"
rm -rf "$ARTIFACT_DIR"
mkdir -p "$ARTIFACT_DIR/aarch64-unknown-linux-gnu" "$ARTIFACT_DIR/x86_64-unknown-linux-gnu"

# Dummy binaries (make them executable, updater will install them)
cat > "$ARTIFACT_DIR/aarch64-unknown-linux-gnu/secluso-config-tool" <<EOF
#!/usr/bin/env sh
echo "dummy secluso-config-tool aarch64 for ${TAG}"
EOF
chmod +x "$ARTIFACT_DIR/aarch64-unknown-linux-gnu/secluso-config-tool"

cat > "$ARTIFACT_DIR/x86_64-unknown-linux-gnu/secluso-config-tool" <<EOF
#!/usr/bin/env sh
echo "dummy secluso-config-tool x86_64 for ${TAG}"
EOF
chmod +x "$ARTIFACT_DIR/x86_64-unknown-linux-gnu/secluso-config-tool"

# Compute sha256s that will be embedded in the signed manifest
SHA_A="$(sha256_file "$ARTIFACT_DIR/aarch64-unknown-linux-gnu/secluso-config-tool")"
SHA_X="$(sha256_file "$ARTIFACT_DIR/x86_64-unknown-linux-gnu/secluso-config-tool")"

# Create builder-style manifest.json w/ per-artifact sha256
cat > "$ARTIFACT_DIR/manifest.json" <<EOF
{
  "build": {
    "target": "test",
    "profile": "release",
    "run_id": "1",
    "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  },
  "artifacts": [
    {
      "package": "config_tool",
      "target": "aarch64-unknown-linux-gnu",
      "bin": "secluso-config-tool",
      "bin_path": "aarch64-unknown-linux-gnu/secluso-config-tool",
      "crate": "config_tool",
      "version": "${TAG#v}",
      "crate_lock_sha256": "test",
      "rust_digest": "test",
      "sha256": "${SHA_A}"
    },
    {
      "package": "config_tool",
      "target": "x86_64-unknown-linux-gnu",
      "bin": "secluso-config-tool",
      "bin_path": "x86_64-unknown-linux-gnu/secluso-config-tool",
      "crate": "config_tool",
      "version": "${TAG#v}",
      "crate_lock_sha256": "test",
      "rust_digest": "test",
      "sha256": "${SHA_X}"
    }
  ]
}
EOF

echo "Created fake builder_out at: $ARTIFACT_DIR"
echo "  aarch64 sha256: $SHA_A"
echo "  x86_64  sha256: $SHA_X"
echo

# Manager step: copy builder manifest into <workdir>/<tag>/manifest.json + write manifest.sha256
"$SCRIPT_DIR/secluso_prepare_release_dir.sh" \
  --tag "$TAG" \
  --workdir "$WORKDIR" \
  --labels "$LABELS" \
  --artifact-dir "$ARTIFACT_DIR"

MANIFEST="$REL_DIR/manifest.json"
SHA_FILE="$REL_DIR/manifest.sha256"
SIG_DIR="$REL_DIR/sigs"
mkdir -p "$SIG_DIR"

# Find an existing key by UID fragment, otherwisewe can create it.
ensure_key() {
  local uid="$1"
  local existing
  existing="$(gpg --with-colons --list-keys "$uid" 2>/dev/null | awk -F: '$1=="fpr" {print $10; exit}' || true)"
  if [[ -n "$existing" ]]; then
    echo "$existing"
    return
  fi

  gpg --batch --pinentry-mode loopback --passphrase "" \
    --quick-generate-key "$uid" ed25519 sign 5y >/dev/null

  gpg --with-colons --fingerprint "$uid" | awk -F: '$1=="fpr" {print $10; exit}'
}

UID1="secluso-test-${L1} <secluso-test-${L1}@example.invalid>"
UID2="secluso-test-${L2} <secluso-test-${L2}@example.invalid>"

FPR1="$(ensure_key "$UID1")"
FPR2="$(ensure_key "$UID2")"

if [[ "$FPR1" == "$FPR2" ]]; then
  echo "ERROR: both labels resolved to the same key fingerprint; aborting" >&2
  exit 1
fi

echo "Using test keys from GNUPGHOME=$GNUPG_HOME"
echo "  $L1 -> $FPR1"
echo "  $L2 -> $FPR2"
echo

# Sign as each label (signer script verifies sha-file etc)
"$SCRIPT_DIR/secluso_sign_manifest.sh" \
  --manifest "$MANIFEST" \
  --sha-file "$SHA_FILE" \
  --label "$L1" \
  --key "$FPR1" \
  --outdir "$SIG_DIR"

"$SCRIPT_DIR/secluso_sign_manifest.sh" \
  --manifest "$MANIFEST" \
  --sha-file "$SHA_FILE" \
  --label "$L2" \
  --key "$FPR2" \
  --outdir "$SIG_DIR"

# Build a real bundle (copies builder_out, overlays the signed manifest + signature files)
"$SCRIPT_DIR/secluso_build_bundle.sh" \
  --tag "$TAG" \
  --workdir "$WORKDIR" \
  --labels "$LABELS" \
  --sig-a "$SIG_DIR/manifest.json.${L1}.asc" \
  --sig-b "$SIG_DIR/manifest.json.${L2}.asc" \
  --artifact-dir "$ARTIFACT_DIR"

echo
echo "Test bundle ready at: $REL_DIR/out/secluso-${TAG}.zip"
echo "Keys are persisted at: $GNUPG_HOME"
echo
echo "If you want the updater to accept these via GitHub keys, export + add them to your GitHub account:"
echo "  gpg --armor --export $FPR1"
echo "  gpg --armor --export $FPR2"