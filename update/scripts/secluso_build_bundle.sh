#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<EOF
Usage (artifact bundle only):
  $0 --tag vX.Y.Z --workdir ./release_work --labels A,B \\
     --sig-a /path/to/manifest.json.A.asc --sig-b /path/to/manifest.json.B.asc \\
     --artifact-dir /path/to/builder_bundle_dir

Outputs:
  <workdir>/<tag>/bundle/
  <workdir>/<tag>/out/secluso-<tag>.zip
EOF
  exit 1
}

TAG=""
WORKDIR=""
LABELS=""
SIG_A=""
SIG_B=""
ARTIFACT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag) TAG="$2"; shift 2;;
    --workdir) WORKDIR="$2"; shift 2;;
    --labels) LABELS="$2"; shift 2;;
    --sig-a) SIG_A="$2"; shift 2;;
    --sig-b) SIG_B="$2"; shift 2;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2;;
    -h|--help) usage;;
    *) echo "Unknown arg: $1" >&2; usage;;
  esac
done

[[ -n "$TAG" && -n "$WORKDIR" && -n "$LABELS" && -n "$SIG_A" && -n "$SIG_B" && -n "$ARTIFACT_DIR" ]] || usage

IFS=',' read -r LABEL_A LABEL_B <<<"$LABELS"
[[ -n "${LABEL_A:-}" && -n "${LABEL_B:-}" ]] || { echo "Need --labels A,B" >&2; exit 1; }

REL_DIR="$WORKDIR/$TAG"
MANIFEST="$REL_DIR/manifest.json"

[[ -f "$MANIFEST" ]] || { echo "Missing manifest: $MANIFEST" >&2; exit 1; }
[[ -f "$SIG_A" ]] || { echo "Missing sig-a: $SIG_A" >&2; exit 1; }
[[ -f "$SIG_B" ]] || { echo "Missing sig-b: $SIG_B" >&2; exit 1; }
[[ -d "$ARTIFACT_DIR" ]] || { echo "Missing --artifact-dir directory: $ARTIFACT_DIR" >&2; exit 1; }
[[ -f "$ARTIFACT_DIR/manifest.json" ]] || { echo "--artifact-dir must contain manifest.json" >&2; exit 1; }

hash_file() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | awk '{print $1}'
  else
    shasum -a 256 "$f" | awk '{print $1}'
  fi
}

echo "Verifying signatures locally..."
gpg --verify "$SIG_A" "$MANIFEST" >/dev/null
gpg --verify "$SIG_B" "$MANIFEST" >/dev/null
echo "OK: both signatures verify"

fpr_from_sig() {
  local sig="$1"
  gpg --status-fd=1 --verify "$sig" "$MANIFEST" 2>/dev/null \
    | awk '$1=="[GNUPG:]" && $2=="VALIDSIG" {print $3; exit}'
}

FPR_A="$(fpr_from_sig "$SIG_A")"
FPR_B="$(fpr_from_sig "$SIG_B")"
[[ -n "$FPR_A" && -n "$FPR_B" ]] || { echo "Could not extract signer fingerprints" >&2; exit 1; }

if [[ "$FPR_A" == "$FPR_B" ]]; then
  echo "ERROR: both sigs are from the same key fingerprint: $FPR_A" >&2
  exit 1
fi

echo "OK: distinct signer fingerprints"
echo "  $LABEL_A -> $FPR_A"
echo "  $LABEL_B -> $FPR_B"

BUNDLE_DIR="$REL_DIR/bundle"
OUT_DIR="$REL_DIR/out"
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR" "$OUT_DIR"

echo "Copying builder artifacts from: $ARTIFACT_DIR"
cp -a "$ARTIFACT_DIR/." "$BUNDLE_DIR/"

# Overwrite manifest.json with the signed one
cp "$MANIFEST" "$BUNDLE_DIR/manifest.json"

# Drop sigs next to manifest with expected names
cp "$SIG_A" "$BUNDLE_DIR/manifest.json.${LABEL_A}.asc"
cp "$SIG_B" "$BUNDLE_DIR/manifest.json.${LABEL_B}.asc"

# Enforce manifest sha256 matches the actual bundle bytes
if command -v jq >/dev/null 2>&1; then
  echo "Checking manifest sha256 fields vs bundle contents..."

  jq -r '.artifacts[] | @base64' "$BUNDLE_DIR/manifest.json" | while read -r row; do
    _jq() { echo "$row" | base64 --decode | jq -r "$1"; }

    bin_path="$(_jq '.bin_path')"
    sha="$(_jq '.sha256')"

    if [[ -z "$bin_path" || "$bin_path" == "null" ]]; then
      echo "ERROR: artifact missing bin_path" >&2
      exit 1
    fi
    if [[ -z "$sha" || "$sha" == "null" ]]; then
      echo "ERROR: artifact $bin_path missing sha256 field in manifest.json" >&2
      exit 1
    fi

    f="$BUNDLE_DIR/$bin_path"
    if [[ ! -f "$f" ]]; then
      echo "ERROR: manifest references missing file: $bin_path" >&2
      exit 1
    fi

    got="$(hash_file "$f")"
    want="$(echo "$sha" | tr -d ' \t\r\n' | sed 's/^sha256://I' | tr 'A-F' 'a-f')"

    if [[ "$got" != "$want" ]]; then
      echo "ERROR: sha256 mismatch for $bin_path" >&2
      echo "  manifest: $want" >&2
      echo "  actual:   $got" >&2
      exit 1
    fi
  done

  echo "OK: manifest sha256 matches all artifact files"
else
  echo "WARN: jq not found, skipping manifest sha256 verification"
fi

ZIP_PATH="$OUT_DIR/secluso-${TAG}.zip"
rm -f "$ZIP_PATH"
( cd "$BUNDLE_DIR" && zip -qr "$ZIP_PATH" . )

echo
echo "Bundle folder: $BUNDLE_DIR"
echo "Zip asset: $ZIP_PATH"
echo "Upload this zip as the Release asset in GitHub UI."