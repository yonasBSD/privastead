#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later

# Deploy application pipeline
#
# Desktop app distribution has fundamentally different mechanics than raw Rust
# binaries... each different OS expects different packaging formats, and host tooling often
# determines what can be produced natively. This pipeline therefore supports a
# dual strategy, seen below...
#
# 1) Native host bundling when the local Tauri CLI supports the target bundle.
# 2) Docker fallback builders for non-Apple targets when native bundling is not
#    available on the current host.
#
# Regardless of path, all artifacts are normalized into:
#   artifacts/<triple>/...
# with ONE manifest schema for compare and verification tooling.

ensure_deploy_workspace_inputs() {
  local deploy_tauri_dir="$1"
  local deploy_cargo_lock="$2"
  local deploy_node_lock="$3"

  [[ -f "$deploy_tauri_dir/Cargo.toml" ]] || die "Missing deploy Cargo.toml at $deploy_tauri_dir/Cargo.toml"
  [[ -f "$deploy_cargo_lock" ]] || die "Missing deploy Cargo.lock at $deploy_cargo_lock"
  [[ -f "$deploy_node_lock" ]] || die "Missing deploy pnpm lockfile at $deploy_node_lock"
}

write_docker_diagnostic_summary() {
  local source_log="$1"
  local summary_log="$2"
  local triple="$3"

  # This summary is intentionally redundant with the full docker log, but it is
  # optimized for quickly sending to peoples in issues / chats. It preserves only the
  # lines that usually explain AppImage packaging failures (linuxdeploy command,
  # plugin probes, subprocess exit codes, and strace status correlation) plus a
  # short tail of the full log for surrounding context.
  {
    echo "Docker buildx diagnostic summary"
    echo "timestamp_utc=$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    echo "triple=$triple"
    echo "source_log=$source_log"
    echo
    echo "== Key AppImage/linuxdeploy signals (with context) =="
    if command -v rg >/dev/null 2>&1; then
      rg -n -C 6 \
        'Bundling \[tauri_bundler::bundle::linux::appimage::linuxdeploy\]|Running \[tauri_bundler::utils\] Command .*/linuxdeploy-x86_64\.AppImage|terminate called after throwing an instance|what\(\):  subprocess failed|manual linuxdeploy replay|exit code \(|appimage wrapper log|strace status=2 correlation|extracted AppRun strace status=2 correlation|ERROR: Could not find plugin|linuxdeploy binary candidate|linuxdeploy-plugin-appimage|linuxdeploy-plugin-gtk|docker buildx failed' \
        "$source_log" || true
    else
      grep -nE \
        'Bundling \[tauri_bundler::bundle::linux::appimage::linuxdeploy\]|Running \[tauri_bundler::utils\] Command .*/linuxdeploy-x86_64\.AppImage|terminate called after throwing an instance|what\(\):  subprocess failed|manual linuxdeploy replay|exit code \(|appimage wrapper log|strace status=2 correlation|extracted AppRun strace status=2 correlation|ERROR: Could not find plugin|linuxdeploy binary candidate|linuxdeploy-plugin-appimage|linuxdeploy-plugin-gtk|docker buildx failed' \
        "$source_log" || true
    fi
    echo
    echo "== Final 400 lines of full docker log =="
    tail -n 400 "$source_log" || true
  } > "$summary_log"
}

run_host_deploy_bundle_for_triple() {
  local run_id="$1"
  local triple="$2"
  local selected_bundle="$3"
  local deploy_dir="$4"
  local deploy_tauri_dir="$5"
  local art_dir="$6"
  local artifacts_json="$7"
  local deploy_version="$8"
  local deploy_lock_sha="$9"
  local digest="${10}"

  local bundle_dir="$deploy_tauri_dir/target/$triple/release/bundle"
  rm -rf "$bundle_dir"

  echo "==> [run $run_id] deploy_tool for $triple (host bundle=$selected_bundle)"
  (
    cd "$deploy_dir" || exit 1
    CARGO_INCREMENTAL=0 CI=true pnpm tauri build -v -v --target "$triple" --bundles "$selected_bundle" --ci --no-sign
  )

  [[ -d "$bundle_dir" ]] || die "Missing deploy bundle output directory: $bundle_dir"

  local found_bundle=0
  while IFS= read -r bundle_file; do
    [[ -z "$bundle_file" ]] && continue
    found_bundle=1

    local rel_path_within_triple="${bundle_file#${bundle_dir}/}"
    local rel_dir
    rel_dir="$(dirname "$rel_path_within_triple")"
    mkdir -p "$art_dir/$rel_dir"
    cp "$bundle_file" "$art_dir/$rel_path_within_triple"

    local copied_path="$art_dir/$rel_path_within_triple"
    local bin_sha
    bin_sha="$(sha256_file "$copied_path")"

    record_deploy_artifact \
      "$artifacts_json" \
      "$triple" \
      "$rel_path_within_triple" \
      "$bin_sha" \
      "$deploy_version" \
      "$deploy_lock_sha" \
      "$digest"
  done < <(
    find "$bundle_dir" -type f \( \
      -name "*.AppImage" -o \
      -name "*.appimage" -o \
      -name "*.deb" -o \
      -name "*.rpm" -o \
      -name "*.msi" -o \
      -name "*.exe" -o \
      -name "*.dmg" -o \
      -name "*.pkg" \
    \) | sort
  )

  [[ "$found_bundle" -eq 1 ]] || die "No desktop bundle files were produced for $triple under $bundle_dir"
}

run_docker_deploy_bundle_for_triple() {
  local run_id="$1"
  local triple="$2"
  local deploy_version="$3"
  local deploy_lock_sha="$4"
  local art_dir="$5"
  local artifacts_json="$6"

  local docker_platform
  docker_platform="$(docker_platform_for_triple "$triple")"

  local docker_digest
  docker_digest="$(rust_digest_for_docker_platform "$docker_platform")"
  [[ -n "$docker_digest" ]] || die "No Rust digest configured for docker platform $docker_platform"

  local docker_runner
  docker_runner="$(deploy_runner_for_triple "$triple")"

  local docker_bundle_targets_json
  docker_bundle_targets_json="$(deploy_bundle_targets_json_for_triple "$triple")"

  # Keep deep Docker diagnostics opt-in for normal CI/local speed and log size.
  local docker_debug="${DEPLOY_DOCKER_DEBUG:-${DEBUG:-0}}"

  local tmp_art_dir
  tmp_art_dir="$(mktemp -d)"

  # We store docker_digest as the effective toolchain identity for artifacts
  # built through fallback containers. This lets compare runs detect
  # toolchain drift even when the host machine itself did not compile the target natively.
  local effective_digest="$docker_digest"
  local docker_build_log="$art_dir/docker-buildx-${triple}.log"
  local docker_summary_log="$art_dir/docker-buildx-${triple}-summary.log"

  echo "==> [run $run_id] deploy_tool for $triple (docker $docker_platform, runner=$docker_runner)"
  echo "==> [run $run_id] docker buildx diagnostic log: $docker_build_log"
  echo "==> [run $run_id] docker buildx copy/paste summary: $docker_summary_log"
  {
    echo "==> docker preflight snapshot"
    echo "timestamp_utc=$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    echo "triple=$triple"
    echo "docker_platform=$docker_platform"
    echo "docker_runner=$docker_runner"
    echo "docker_bundle_targets_json=$docker_bundle_targets_json"
    echo "docker_debug=$docker_debug"
    echo "docker_digest=$docker_digest"
    echo ""
    docker --version || true
    docker buildx version || true
    docker version || true
    docker buildx inspect "$BUILDER" || true
  } > "$docker_build_log"
  # Docker build output can be very long and frequently contains the only
  # actionable failure clue when packaging tools abort internally. We always tee
  # the complete stream into artifacts/<triple>/ so the log is preserved for
  # "post-mortem" analysis even if terminal scrollback truncates for some reason
  if ! docker buildx build \
    --builder "$BUILDER" \
    --progress plain \
    --no-cache \
    --platform "$docker_platform" \
    --target artifact \
    --build-context "proj=${PROJECT_ROOT}" \
    --build-arg "RUST_HASH=${docker_digest}" \
    --build-arg "TAURI_TARGET=${triple}" \
    --build-arg "TAURI_RUNNER=${docker_runner}" \
    --build-arg "TAURI_BUNDLE_TARGETS_JSON=${docker_bundle_targets_json}" \
    --build-arg "DEBUG=${docker_debug}" \
    --output "type=local,dest=${tmp_art_dir}" \
    -f "${RELEASES_DIR}/Dockerfile.deploy" \
    "${RELEASES_DIR}" \
    2>&1 | tee -a "$docker_build_log"; then
    write_docker_diagnostic_summary "$docker_build_log" "$docker_summary_log" "$triple"
    die "Docker buildx failed for $triple. Full diagnostic log: $docker_build_log. Copy/paste summary: $docker_summary_log"
  fi
  write_docker_diagnostic_summary "$docker_build_log" "$docker_summary_log" "$triple"

  local found_any=0
  local docker_bundle_dir="$tmp_art_dir/release/bundle"
  if [[ -d "$docker_bundle_dir" ]]; then
    while IFS= read -r bundle_file; do
      [[ -z "$bundle_file" ]] && continue
      found_any=1

      local rel_path_within_triple="bundle/${bundle_file#${docker_bundle_dir}/}"
      local rel_dir
      rel_dir="$(dirname "$rel_path_within_triple")"
      mkdir -p "$art_dir/$rel_dir"
      cp "$bundle_file" "$art_dir/$rel_path_within_triple"

      local copied_path="$art_dir/$rel_path_within_triple"
      local bin_sha
      bin_sha="$(sha256_file "$copied_path")"

      record_deploy_artifact \
        "$artifacts_json" \
        "$triple" \
        "$rel_path_within_triple" \
        "$bin_sha" \
        "$deploy_version" \
        "$deploy_lock_sha" \
        "$effective_digest"
    done < <(
      find "$docker_bundle_dir" -type f \( \
        -name "*.AppImage" -o \
        -name "*.appimage" -o \
        -name "*.deb" -o \
        -name "*.rpm" -o \
        -name "*.msi" -o \
        -name "*.exe" \
      \) | sort
    )
  fi

  # Some cross-build paths produce a portable binary without an installer. We
  # intentionally keep that binary because it is still valuable for internal
  # verification, and it remains verifiable through manifest
  # hashes like every other artifact.
  if [[ "$found_any" -eq 0 && -d "$tmp_art_dir/release" ]]; then
    local portable_bin=""
    if is_windows_triple "$triple" && [[ -f "$tmp_art_dir/release/secluso-deploy.exe" ]]; then
      portable_bin="$tmp_art_dir/release/secluso-deploy.exe"
    elif is_linux_triple "$triple" && [[ -f "$tmp_art_dir/release/secluso-deploy" ]]; then
      portable_bin="$tmp_art_dir/release/secluso-deploy"
    fi

    if [[ -n "$portable_bin" ]]; then
      found_any=1
      local rel_path_within_triple="portable/$(basename "$portable_bin")"
      mkdir -p "$art_dir/portable"
      cp "$portable_bin" "$art_dir/$rel_path_within_triple"
      local bin_sha
      bin_sha="$(sha256_file "$art_dir/$rel_path_within_triple")"

      record_deploy_artifact \
        "$artifacts_json" \
        "$triple" \
        "$rel_path_within_triple" \
        "$bin_sha" \
        "$deploy_version" \
        "$deploy_lock_sha" \
        "$effective_digest"
    fi
  fi

  rm -rf "$tmp_art_dir"

  if [[ "$found_any" -eq 0 ]]; then
    die "No deploy artifacts were produced for $triple via Docker fallback."
  fi
}

build_deploy_and_manifest() {
  local outdir="$1"
  local run_id="$2"

  mkdir -p "$outdir"

  local artifacts_json
  artifacts_json="$(mktemp)"
  : > "$artifacts_json"

  local deploy_dir="${PROJECT_ROOT}/deploy"
  local deploy_tauri_dir="$deploy_dir/src-tauri"
  local deploy_cargo_lock="$deploy_tauri_dir/Cargo.lock"
  local deploy_node_lock="$deploy_dir/pnpm-lock.yaml"

  ensure_deploy_workspace_inputs "$deploy_tauri_dir" "$deploy_cargo_lock" "$deploy_node_lock"

  local deploy_version
  deploy_version="$({
    cargo metadata --no-deps --format-version 1 --manifest-path "$deploy_tauri_dir/Cargo.toml"
  } | jq -r '.packages[0].version')"
  [[ -n "$deploy_version" && "$deploy_version" != "null" ]] || die "Could not get deploy version from $deploy_tauri_dir/Cargo.toml"

  local deploy_lock_sha
  deploy_lock_sha="$({ cat "$deploy_cargo_lock"; cat "$deploy_node_lock"; } | sha256_stdin)"

  local host_toolchain_sha
  host_toolchain_sha="$({ rustc -Vv; node --version; pnpm --version; } | sha256_stdin)"

  local supported_bundles
  supported_bundles="$(detect_supported_tauri_bundles "$deploy_dir")"
  [[ -n "$supported_bundles" ]] || die "Could not determine supported Tauri bundle types from local CLI. Run: cd ${deploy_dir} && pnpm tauri build --help"

  echo "==> [run $run_id] Installing deploy UI deps (pnpm install --frozen-lockfile)"
  (
    cd "$deploy_dir" || exit 1
    CI=true pnpm install --frozen-lockfile
  )

  if [[ "$BUILD_KIND" == "deploy" && "$DEPLOY_REQUIRES_DOCKER" -eq 1 && ! -f "${RELEASES_DIR}/Dockerfile.deploy" ]]; then
    die "Missing ${RELEASES_DIR}/Dockerfile.deploy required for deploy cross-platform builds."
  fi

  # This triple loop intentionally mixes native and fallback paths so one run
  # can produce a complete desktop matrix where possible (ideally always). Each triple is fully
  # self-contained in artifacts/<triple>, which makes post-build packaging and
  # compare operations straightforward and helps everyone here be less error-prone for operators.
  local triple
  for triple in "${TRIPLES[@]}"; do
    local art_dir
    art_dir="$(artifact_dir_for_triple "$outdir" "$triple")"
    mkdir -p "$art_dir"

    local digest
    digest="$(lookup_rust_digest "$triple")"
    if [[ -z "$digest" ]]; then
      digest="host-toolchain-${host_toolchain_sha}"
    fi

    local bundle_candidates
    bundle_candidates="$(select_deploy_bundles_for_triple "$triple")"

    local selected_bundle=""
    if selected_bundle="$(pick_supported_bundle "$bundle_candidates" "$supported_bundles" 2>/dev/null)"; then
      run_host_deploy_bundle_for_triple \
        "$run_id" \
        "$triple" \
        "$selected_bundle" \
        "$deploy_dir" \
        "$deploy_tauri_dir" \
        "$art_dir" \
        "$artifacts_json" \
        "$deploy_version" \
        "$deploy_lock_sha" \
        "$digest"
      continue
    fi

    if [[ "$DEPLOY_REQUIRES_DOCKER" -ne 1 ]]; then
      die "Host Tauri CLI cannot package target $triple and Docker fallback is disabled. Supported on this host: $supported_bundles"
    fi

    if is_apple_triple "$triple"; then
      die "Target $triple requires macOS host bundling. Supported on this host: $supported_bundles"
    fi

    run_docker_deploy_bundle_for_triple \
      "$run_id" \
      "$triple" \
      "$deploy_version" \
      "$deploy_lock_sha" \
      "$art_dir" \
      "$artifacts_json"
  done

  write_manifest "$outdir" "$run_id" "$artifacts_json"
  rm -f "$artifacts_json"
}
