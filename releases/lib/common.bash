#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later

# Shared primitives for the release build system
#
# Avoids build-mode policy. It only exposes generic
# utilities used by every pipeline path (rust binaries, deploy desktop app, and
# compare mode). Keeping these helpers centralized gives us consistent error
# behavior and consistent artifact metadata regardless of what is being built.
#
# our new Output "contract" is every build run now uses this shape:
#   <run-dir>/manifest.json
#   <run-dir>/artifacts/<target-triple>/...
#
# helps removes the ambiguity of some files in the run root and some in
# per-triple directories and makes it obvious what to archive and hand to
# auditors for independent verification.

usage() {
  echo "Usage:" >&2
  echo "  ${PROGRAM_NAME} --target {raspberry|ipcamera|server|all|deploy} --profile <profile> [--test-reproduce]" >&2
  echo "  ${PROGRAM_NAME} --compare <build_dir_run1> <build_dir_run2>" >&2
  echo "" >&2
  echo "Deploy profiles:" >&2
  echo "  all|linux|macos|windows|linux-x64|linux-arm64|macos-x64|macos-arm64|windows-x64|windows-arm64" >&2
}

die() {
  echo "$*" >&2
  exit 1
}

require_tool() {
  local tool="$1"
  if ! command -v "$tool" >/dev/null 2>&1; then
    die "Required tool missing: $tool"
  fi
}

init_sha256_tool() {
  if command -v sha256sum >/dev/null 2>&1; then
    SHA256_TOOL="sha256sum"
    SHA256_ARGS=()
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    SHA256_TOOL="shasum"
    SHA256_ARGS=( -a 256 )
    return
  fi

  die "Required tool missing: sha256sum (or shasum)"
}

sha256_file() {
  local file_path="$1"
  # Bash 3.2 + nounset treats an empty array expansion as unbound. Branch on
  # arg count so we can safely support both sha256sum (no extra args) and
  # shasum -a 256 (extra args) without passing empty argv entries.
  if [[ ${#SHA256_ARGS[@]} -gt 0 ]]; then
    "$SHA256_TOOL" "${SHA256_ARGS[@]}" "$file_path" | awk '{print $1}'
  else
    "$SHA256_TOOL" "$file_path" | awk '{print $1}'
  fi
}

sha256_stdin() {
  if [[ ${#SHA256_ARGS[@]} -gt 0 ]]; then
    "$SHA256_TOOL" "${SHA256_ARGS[@]}" | awk '{print $1}'
  else
    "$SHA256_TOOL" | awk '{print $1}'
  fi
}

lookup_rust_digest() {
  local triple="$1"
  local key
  key="$(printf '%s' "$triple" | tr '[:lower:]' '[:upper:]' | tr '-' '_')"
  local var="RUST_DIGEST__${key}"
  printf '%s' "${!var:-}"
}

artifact_dir_for_triple() {
  local run_dir="$1"
  local triple="$2"
  printf '%s/artifacts/%s' "$run_dir" "$triple"
}

write_manifest() {
  local outdir="$1"
  local run_id="$2"
  local artifacts_json="$3"
  local artifacts_joined

  artifacts_joined="$(paste -sd',' "$artifacts_json")"

  cat > "$outdir/manifest.json" <<JSON
{
  "build": {
    "target": "$TARGET",
    "profile": "$PROFILE",
    "run_id": "$run_id",
    "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  },
  "artifacts": [
$artifacts_joined
  ]
}
JSON
}

finalize_run_output() {
  local run_dir="$1"
  local _run_id="$2"

  echo "Run output"
  echo "- Manifest : $run_dir/manifest.json"
  echo "- Artifacts: $run_dir/artifacts"
  echo ""
}
