#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
IFS=$'\n\t'

# This is where the 'entry point' is for reproducible builds for Secluso.
#
# This file is like an orchestration layer. It wires together policy,
# pipelines, and comparison while leaving implementation details in our 'modules'.
#
# High-level lifecycle for build:
# 1) Parse args and resolve a build plan.
# 2) Load digests/tooling prerequisites.
# 3) Execute one or two runs (self-test).
# 4) Materialize a self-contained verification bundle per run.
#
# High-level lifecycle for compare:
# - Validate two existing run directories without requiring build tooling.

PROGRAM_NAME="$(basename "$0")"
RELEASES_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd -- "${RELEASES_DIR}/.." && pwd)"
DIGESTS_LOCK_FILE="${RELEASES_DIR}/digests.lock.env"

# Mutable runtime state shared with sourced modules.
TARGET=""
PROFILE=""
TEST_REPRODUCE=0
COMPARE_DIR1=""
COMPARE_DIR2=""
TRIPLES=()
PKGS=()
BUILD_KIND="rust"
DEPLOY_REQUIRES_DOCKER=0
BUILDER=""
SHA256_TOOL=""
SHA256_ARGS=()

# shellcheck disable=SC1091
source "${RELEASES_DIR}/lib/common.bash"
# shellcheck disable=SC1091
source "${RELEASES_DIR}/lib/deploy_helpers.bash"
# shellcheck disable=SC1091
source "${RELEASES_DIR}/lib/plan.bash"
# shellcheck disable=SC1091
source "${RELEASES_DIR}/lib/rust_pipeline.bash"
# shellcheck disable=SC1091
source "${RELEASES_DIR}/lib/deploy_pipeline.bash"
# shellcheck disable=SC1091
source "${RELEASES_DIR}/lib/compare.bash"

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --target)
        TARGET="${2:?}"
        shift 2
        ;;
      --profile)
        PROFILE="${2:?}"
        shift 2
        ;;
      --test-reproduce)
        TEST_REPRODUCE=1
        shift 1
        ;;
      --compare)
        COMPARE_DIR1="${2:?}"
        COMPARE_DIR2="${3:?}"
        shift 3
        ;;
      *)
        usage
        die "Unknown option: $1"
        ;;
    esac
  done
}

run_single_build() {
  local run_dir="$1"
  local run_id="$2"

  if [[ "$BUILD_KIND" == "deploy" ]]; then
    build_deploy_and_manifest "$run_dir" "$run_id"
  else
    build_and_manifest "$run_dir" "$run_id"
  fi

  finalize_run_output "$run_dir" "$run_id"
}

main() {
  parse_args "$@"

  if [[ -z "$COMPARE_DIR1" && ( -z "$TARGET" || -z "$PROFILE" ) ]]; then
    usage
    exit 1
  fi

  init_sha256_tool
  require_tool jq

  # if an auditor receives two previously built run directories, they should
  # NOT need Rust, Docker, or Node installed just to perform cryptographic and metadata validation.
  if [[ -n "$COMPARE_DIR1" ]]; then
    echo "Compare-only mode:"
    echo "- run1: $COMPARE_DIR1"
    echo "- run2: $COMPARE_DIR2"
    echo ""
    compare_runs "$COMPARE_DIR1" "$COMPARE_DIR2"
    exit $?
  fi

  resolve_build_plan
  load_locked_digests_if_needed
  ensure_required_tools_for_mode
  print_build_configuration
  setup_builder_if_needed

  local timestamp
  timestamp="$(date +%s)"
  local base_dir="${RELEASES_DIR}/builds/${timestamp}"

  # Self-test mode executes two independent runs to prove determinism on the
  # same machine, with fresh output directories and explicit compare output.
  if [[ "$TEST_REPRODUCE" -eq 1 ]]; then
    echo "Reproducibility test: two builds"

    run_single_build "$base_dir/run1" 1
    run_single_build "$base_dir/run2" 2

    echo ""
    compare_runs "$base_dir/run1" "$base_dir/run2"
    exit $?
  fi

  run_single_build "$base_dir" 1
  echo "Build complete. Output: $base_dir"
}

main "$@"
