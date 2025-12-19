#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<EOF
Usage:
  $0 --manifest /path/to/manifest.json --sha-file /path/to/manifest.sha256 \\
     --label LABEL --key KEYID_OR_FPR --outdir /path/to/output

Produces:
  <outdir>/manifest.json.<label>.asc

LABEL must match what manager chose and what the updater expects in the zip.

EOF
  exit 1
}

MANIFEST=""
SHA_FILE=""
LABEL=""
KEY=""
OUTDIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest) MANIFEST="$2"; shift 2;;
    --sha-file) SHA_FILE="$2"; shift 2;;
    --label) LABEL="$2"; shift 2;;
    --key) KEY="$2"; shift 2;;
    --outdir) OUTDIR="$2"; shift 2;;
    -h|--help) usage;;
    *) echo "Unknown arg: $1" >&2; usage;;
  esac
done

[[ -n "$MANIFEST" && -n "$SHA_FILE" && -n "$LABEL" && -n "$KEY" && -n "$OUTDIR" ]] || usage
[[ -f "$MANIFEST" ]] || { echo "Missing manifest: $MANIFEST" >&2; exit 1; }
[[ -f "$SHA_FILE" ]] || { echo "Missing sha file: $SHA_FILE" >&2; exit 1; }

hash_file() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | awk '{print $1}'
  else
    shasum -a 256 "$f" | awk '{print $1}'
  fi
}

EXPECTED="$(awk '{print $1}' "$SHA_FILE" | head -n1)"
ACTUAL="$(hash_file "$MANIFEST")"

if [[ "$EXPECTED" != "$ACTUAL" ]]; then
  echo "ERROR: manifest SHA mismatch!" >&2
  echo "Expected: $EXPECTED" >&2
  echo "Actual: $ACTUAL" >&2
  echo "Do NOT sign, ask manager to resend files." >&2
  exit 1
fi

mkdir -p "$OUTDIR"
OUTSIG="$OUTDIR/manifest.json.${LABEL}.asc"

echo "SHA check OK. Signing..."
gpg --batch --yes --armor --detach-sign --local-user "$KEY" \
  -o "$OUTSIG" "$MANIFEST"

echo "Created signature: $OUTSIG"
echo
echo "Send this .asc back to the release manager:"
echo "  $OUTSIG"