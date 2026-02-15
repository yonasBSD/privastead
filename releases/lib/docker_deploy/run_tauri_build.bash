#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

is_debug_enabled() {
  case "${DEBUG:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

configure_rust_log() {
  if is_debug_enabled; then
    # Keep bundler diagnostics detailed but avoid ureq hex dumps that can
    # overflow BuildKit step logs and hide the real failure signal.
    export RUST_LOG="${RUST_LOG:-tauri_bundler=debug,tauri_cli_node=debug,ureq=warn}"
  else
    export RUST_LOG="${RUST_LOG:-tauri_bundler=info,tauri_cli_node=info,ureq=warn}"
  fi
}

write_bundle_config() {
  printf '{ "bundle": { "targets": %s } }\n' "${TAURI_BUNDLE_TARGETS_JSON}" > /tmp/tauri-bundle-config.json
  echo "==> tauri bundle config"
  cat /tmp/tauri-bundle-config.json
}

configure_deterministic_build_env() {
  local source_date_epoch="${SOURCE_DATE_EPOCH:-1704067200}"
  local deterministic_rustflags="${RUSTFLAGS:-}"
  local remap_flag

  [[ "$source_date_epoch" =~ ^[0-9]+$ ]] || {
    echo "Invalid SOURCE_DATE_EPOCH: $source_date_epoch" >&2
    exit 1
  }

  export SOURCE_DATE_EPOCH="$source_date_epoch"
  export TZ=UTC
  export LC_ALL=C
  export LANG=C
  export ZERO_AR_DATE=1
  export CARGO_INCREMENTAL=0
  export CARGO_BUILD_JOBS=1
  export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
  export CARGO_PROFILE_RELEASE_DEBUG=0
  export CARGO_PROFILE_RELEASE_STRIP=debuginfo

  for remap_flag in \
    "--remap-path-prefix=/app=." \
    "--remap-path-prefix=/root/.cargo=/cargo-home" \
    "--remap-path-prefix=/root=/home/user"
  do
    if [[ "$deterministic_rustflags" != *"$remap_flag"* ]]; then
      deterministic_rustflags="${deterministic_rustflags:+$deterministic_rustflags }$remap_flag"
    fi
  done

  if [[ "$TAURI_TARGET" == *"-pc-windows-"* ]]; then
    local brepro_flag="-C link-arg=/Brepro"
    if [[ "$deterministic_rustflags" != *"$brepro_flag"* ]]; then
      deterministic_rustflags="${deterministic_rustflags:+$deterministic_rustflags }$brepro_flag"
    fi
    local debug_none_flag="-C link-arg=/DEBUG:NONE"
    if [[ "$deterministic_rustflags" != *"$debug_none_flag"* ]]; then
      deterministic_rustflags="${deterministic_rustflags:+$deterministic_rustflags }$debug_none_flag"
    fi
  fi

  export RUSTFLAGS="$deterministic_rustflags"
}

prepare_windows_nsis_cache() {
  [[ "$TAURI_TARGET" == *"-pc-windows-"* ]] || return

  local cache_dir="/root/.cache/tauri"
  local nsis_utils_version="${TAURI_NSIS_UTILS_VERSION:-0.5.3}"
  local dll_path="$cache_dir/nsis_tauri_utils.dll"
  local download_url="https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v${nsis_utils_version}/nsis_tauri_utils.dll"

  mkdir -p "$cache_dir"
  if [[ ! -f "$dll_path" ]]; then
    echo "==> prefetching nsis_tauri_utils.dll (v${nsis_utils_version})"
    curl --fail --location --retry 3 --output "$dll_path" "$download_url"
  fi
  touch -h -d "@${SOURCE_DATE_EPOCH}" "$dll_path"
}

normalize_windows_bundle_inputs_once() {
  [[ "$TAURI_TARGET" == *"-pc-windows-"* ]] || return

  local release_dir="/app/deploy/src-tauri/target/${TAURI_TARGET}/release"
  [[ -d "$release_dir" ]] || return

  # NSIS can capture file metadata from staged bundle inputs. Keep mtime stable
  # while tauri is preparing bundle assets.
  while IFS= read -r -d '' path; do
    touch -h -d "@${SOURCE_DATE_EPOCH}" -- "$path" 2>/dev/null || true
  done < <(find "$release_dir" -mindepth 1 -print0 2>/dev/null | LC_ALL=C sort -z)

  touch -h -d "@${SOURCE_DATE_EPOCH}" /root/.cache/tauri/nsis_tauri_utils.dll 2>/dev/null || true
}

setup_windows_makensis_wrapper() {
  [[ "$TAURI_TARGET" == *"-pc-windows-"* ]] || return

  local real_makensis
  real_makensis="$(command -v makensis || true)"
  [[ -n "$real_makensis" ]] || return

  local wrapper_dir="/tmp/secluso-tool-overrides"
  local wrapper_path="$wrapper_dir/makensis"
  mkdir -p "$wrapper_dir"

  cat > "$wrapper_path" <<EOF
#!/bin/bash
set -euo pipefail
: "\${SOURCE_DATE_EPOCH:=1704067200}"
: "\${TAURI_TARGET:=x86_64-pc-windows-msvc}"

release_dir="/app/deploy/src-tauri/target/\${TAURI_TARGET}/release"
if [[ -d "\$release_dir" ]]; then
  if [[ -f "\$release_dir/secluso-deploy.exe" ]]; then
    if command -v llvm-strip >/dev/null 2>&1; then
      if llvm-strip --strip-debug "\$release_dir/secluso-deploy.exe" \
        || llvm-strip -g "\$release_dir/secluso-deploy.exe"; then
        touch -h -d "@\${SOURCE_DATE_EPOCH}" "\$release_dir/secluso-deploy.exe" 2>/dev/null || true
        echo "==> stripped debug from secluso-deploy.exe for reproducibility"
      else
        echo "==> warning: llvm-strip failed for secluso-deploy.exe; continuing without strip" >&2
      fi
    fi
  fi

  while IFS= read -r -d '' path; do
    touch -h -d "@\${SOURCE_DATE_EPOCH}" -- "\$path" 2>/dev/null || true
  done < <(find "\$release_dir" -mindepth 1 -print0 2>/dev/null | LC_ALL=C sort -z)
fi
touch -h -d "@\${SOURCE_DATE_EPOCH}" /root/.cache/tauri/nsis_tauri_utils.dll 2>/dev/null || true

snapshot_dir="\$release_dir/repro/windows-pre-nsis"
mkdir -p "\$snapshot_dir"
snapshot_file="\$snapshot_dir/input-manifest.tsv"
{
  printf 'type\tmode_hex\tmtime_epoch\tsize_bytes\tsha256\trel_path\n'

  while IFS= read -r -d '' path; do
    rel="\${path#\$release_dir/}"
    mode_hex="\$(stat -c '%f' -- "\$path" 2>/dev/null || echo '-')"
    mtime_epoch="\$(stat -c '%Y' -- "\$path" 2>/dev/null || echo '-')"
    size_bytes="\$(stat -c '%s' -- "\$path" 2>/dev/null || echo '-')"
    sha256="\$(sha256sum -- "\$path" 2>/dev/null | awk '{print \$1}')"
    printf 'file\t%s\t%s\t%s\t%s\t%s\n' "\$mode_hex" "\$mtime_epoch" "\$size_bytes" "\$sha256" "\$rel"
  done < <(find "\$release_dir/nsis" -type f -print0 2>/dev/null | LC_ALL=C sort -z)

  while IFS= read -r -d '' path; do
    rel="\${path#\$release_dir/}"
    mode_hex="\$(stat -c '%f' -- "\$path" 2>/dev/null || echo '-')"
    mtime_epoch="\$(stat -c '%Y' -- "\$path" 2>/dev/null || echo '-')"
    printf 'dir\t%s\t%s\t-\t-\t%s\n' "\$mode_hex" "\$mtime_epoch" "\$rel"
  done < <(find "\$release_dir/nsis" -type d -print0 2>/dev/null | LC_ALL=C sort -z)

  if [[ -f "\$release_dir/secluso-deploy.exe" ]]; then
    mode_hex="\$(stat -c '%f' -- "\$release_dir/secluso-deploy.exe" 2>/dev/null || echo '-')"
    mtime_epoch="\$(stat -c '%Y' -- "\$release_dir/secluso-deploy.exe" 2>/dev/null || echo '-')"
    size_bytes="\$(stat -c '%s' -- "\$release_dir/secluso-deploy.exe" 2>/dev/null || echo '-')"
    sha256="\$(sha256sum -- "\$release_dir/secluso-deploy.exe" 2>/dev/null | awk '{print \$1}')"
    printf 'file\t%s\t%s\t%s\t%s\t%s\n' "\$mode_hex" "\$mtime_epoch" "\$size_bytes" "\$sha256" "secluso-deploy.exe"
  fi
} > "\$snapshot_file"

exec "${real_makensis}" "\$@"
EOF

  chmod +x "$wrapper_path"
  export PATH="$wrapper_dir:$PATH"
}

setup_windows_arm64_clang_wrapper() {
  [[ "$TAURI_TARGET" == "aarch64-pc-windows-msvc" ]] || return
  [[ "${TAURI_RUNNER:-}" == "cargo-xwin" ]] || return

  local wrapper_dir="/tmp/secluso-tool-overrides"
  local clang_wrapper="$wrapper_dir/clang"
  local clang_cl_wrapper="$wrapper_dir/clang-cl"
  local real_clang
  local real_clang_escaped
  real_clang="$(type -P clang || true)"
  [[ -n "$real_clang" && -x "$real_clang" ]] || {
    echo "Unable to resolve real clang binary for arm64 windows wrapper" >&2
    exit 1
  }
  printf -v real_clang_escaped '%q' "$real_clang"
  mkdir -p "$wrapper_dir"

  cat > "$clang_wrapper" <<EOF
#!/bin/bash
set -euo pipefail

real_clang=${real_clang_escaped}

translated=()
while [[ "\$#" -gt 0 ]]; do
  case "\$1" in
    /imsvc)
      shift
      [[ "\$#" -gt 0 ]] || break
      translated+=("-isystem" "\$1")
      ;;
    *)
      translated+=("\$1")
      ;;
  esac
  shift
done

exec "\$real_clang" "\${translated[@]}"
EOF

  cat > "$clang_cl_wrapper" <<EOF
#!/bin/bash
set -euo pipefail

real_clang=${real_clang_escaped}

# Emulate clang-cl when cargo-xwin expects a clang-cl executable in PATH.
exec "\$real_clang" --driver-mode=cl "\$@"
EOF

  chmod +x "$clang_wrapper" "$clang_cl_wrapper"

  export PATH="$wrapper_dir:$PATH"
  hash -r
  command -v clang >/dev/null 2>&1 || {
    echo "clang wrapper not found in PATH after setup" >&2
    exit 1
  }
  command -v clang-cl >/dev/null 2>&1 || {
    echo "clang-cl wrapper not found in PATH after setup" >&2
    exit 1
  }
  "$clang_wrapper" --version >/dev/null 2>&1 || {
    echo "clang wrapper self-test failed" >&2
    "$clang_wrapper" --version || true
    exit 1
  }
  "$clang_cl_wrapper" --version >/dev/null 2>&1 || {
    echo "clang-cl wrapper self-test failed" >&2
    "$clang_cl_wrapper" --version || true
    exit 1
  }
  echo "==> arm64 windows clang wrapper active (real clang: $real_clang)"

  # cargo-xwin injects /imsvc-style include args, but ring/cc-rs still shells
  # out to "clang" for some arm64 Windows C objects. Intercept clang at PATH
  # level and normalize /imsvc include pairs to clang-compatible flags.
}

run_debug_preflight() {
  echo "==> build environment snapshot"
  uname -a || true
  cat /etc/os-release || true
  id || true
  pwd
  ulimit -a || true
  env | sort
  dpkg --print-architecture || true
  dpkg-query -W \
    bash \
    ca-certificates \
    desktop-file-utils \
    libgtk-3-bin \
    libglib2.0-bin \
    libgdk-pixbuf2.0-bin \
    nodejs \
    npm \
    pnpm \
    rpm \
    strace \
    squashfs-tools \
    xdg-utils \
    || true
  echo "==> Tauri/Node/Rust versions"
  pnpm --version
  pnpm tauri --version
  pnpm tauri info || true
  node --version
  rustc --version
  rustc -Vv || true
  cargo --version
  cargo --list | head -n 80 || true
  ldd --version || true
  readelf --version || true
  strace --version || true
  echo "==> workspace snapshot"
  ls -la /app || true
  ls -la /app/deploy || true
  ls -la /app/deploy/src-tauri || true
  if [ -f /app/deploy/src-tauri/tauri.conf.json ]; then
    echo "==> tauri.conf.json"
    sed -n '1,260p' /app/deploy/src-tauri/tauri.conf.json || true
  fi
  echo "==> linuxdeploy cache preflight"
  ls -la /root/.cache/tauri || true
  command -v mksquashfs || true
  command -v unsquashfs || true
  command -v desktop-file-validate || true
  command -v bash || true
  command -v sh || true
  command -v glib-compile-schemas || true
  command -v gdk-pixbuf-query-loaders || true
  command -v gtk-query-immodules-3.0 || true
  : > /tmp/linuxdeploy-plugin-gtk-wrapper.log
  : > /tmp/linuxdeploy-plugin-appimage-wrapper.log
  echo "==> linuxdeploy plugin probe"
  ls -la /root/.cache/tauri || true
  find /root/.cache/tauri -maxdepth 1 -type f -name 'linuxdeploy-*' -print -exec file {} \; 2>/dev/null || true
  ls -la /root/.cache/tauri/linuxdeploy-plugin-* || true
  /root/.cache/tauri/linuxdeploy-plugin-gstreamer.sh --plugin-api-version || true
  sh /root/.cache/tauri/linuxdeploy-plugin-gstreamer.sh --plugin-api-version || true
  /root/.cache/tauri/linuxdeploy-plugin-gtk.sh --plugin-api-version || true
  sh /root/.cache/tauri/linuxdeploy-plugin-gtk.sh --plugin-api-version || true
  bash /root/.cache/tauri/linuxdeploy-plugin-gtk.sh --plugin-api-version || true
  if [ -x /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage ]; then
    file /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage || true
    /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage --plugin-api-version || true
    /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage --help || true
    /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage --appimage-extract-and-run --help || true
    /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage --appimage-extract-and-run --plugin-api-version || true
  fi
  cat /tmp/linuxdeploy-plugin-gtk-wrapper.log || true
  cat /tmp/linuxdeploy-plugin-appimage-wrapper.log || true
}

run_tauri_build() {
  prepare_windows_nsis_cache
  setup_windows_makensis_wrapper
  setup_windows_arm64_clang_wrapper
  normalize_windows_bundle_inputs_once

  pnpm tauri build \
    -v -v \
    --target "${TAURI_TARGET}" \
    --runner "${TAURI_RUNNER}" \
    --config /tmp/tauri-bundle-config.json \
    --ci \
    --no-sign \
    -- \
    --locked
}

dump_failure_diagnostics() {
  echo "==> filesystem snapshot after failure"
  ls -la /app/deploy/src-tauri/target || true
  ls -la /app/deploy/src-tauri/target/"${TAURI_TARGET}" || true
  ls -la /app/deploy/src-tauri/target/"${TAURI_TARGET}"/release || true
  ls -la /app/deploy/src-tauri/target/"${TAURI_TARGET}"/release/bundle || true
  find /app/deploy/src-tauri/target/"${TAURI_TARGET}"/release/bundle -maxdepth 4 -mindepth 1 -print 2>/dev/null | sort || true
  ls -la /root/.cache/tauri || true
  ls -la /root/.cache/tauri/linuxdeploy-plugin-* || true
  for p in /root/.cache/tauri/linuxdeploy-plugin-*.sh; do
    [ -f "$p" ] || continue
    echo "---- plugin: $p ----"
    head -n 120 "$p" || true
    "$p" --plugin-api-version || true
    sh "$p" --plugin-api-version || true
    sh -x "$p" --plugin-api-version || true
  done
  if [ -f /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage ]; then
    echo "---- plugin: /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage ----"
    file /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage || true
    /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage --appimage-extract-and-run --help || true
    /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage --appimage-extract-and-run --plugin-api-version || true
  fi
  echo "---- gtk wrapper log ----"
  cat /tmp/linuxdeploy-plugin-gtk-wrapper.log || true
  echo "---- appimage wrapper log ----"
  cat /tmp/linuxdeploy-plugin-appimage-wrapper.log || true
  if [ -f /root/.cache/tauri/linuxdeploy-plugin-gtk.real.sh ]; then
    echo "---- gtk real plugin head ----"
    head -n 120 /root/.cache/tauri/linuxdeploy-plugin-gtk.real.sh || true
  fi

  local appdir="/app/deploy/src-tauri/target/${TAURI_TARGET}/release/bundle/appimage/secluso-deploy.AppDir"

  echo "==> coredump probe"
  find /app /tmp -maxdepth 3 -type f \( -name 'core' -o -name 'core.*' \) -print 2>/dev/null || true

  if command -v strace >/dev/null 2>&1; then
    echo "==> rerunning tauri build under strace for subprocess chain capture"
    CARGO_INCREMENTAL=0 strace -ff -qq -e trace=execve,clone,fork,vfork,wait4,waitid -s 256 -o /tmp/tauri-build.strace \
      pnpm tauri build \
      -v -v \
      --target "${TAURI_TARGET}" \
      --runner "${TAURI_RUNNER}" \
      --config /tmp/tauri-bundle-config.json \
      --ci \
      --no-sign \
      -- \
      --locked 2>/tmp/tauri-build.strace.stderr || true
    echo "==> tauri strace stderr tail"
    tail -n 120 /tmp/tauri-build.strace.stderr || true
    find /tmp -maxdepth 1 -type f -name 'tauri-build.strace*' | sort > /tmp/tauri-build.strace.files || true
    echo "==> tauri strace shard count"
    wc -l /tmp/tauri-build.strace.files || true
    echo "==> tauri strace shard sample (head/tail)"
    head -n 5 /tmp/tauri-build.strace.files || true
    tail -n 5 /tmp/tauri-build.strace.files || true
    if [ -s /tmp/tauri-build.strace.files ]; then
      xargs -r grep -hE 'linuxdeploy|plugin|execve[(]|wait4[(]|waitid[(]' < /tmp/tauri-build.strace.files | tail -n 600 || true
    fi
  fi

  if [ -d "$appdir" ]; then
    echo "==> AppDir snapshot: $appdir"
    ls -la "$appdir" || true
    find "$appdir" -maxdepth 5 -mindepth 1 -print 2>/dev/null | sort | head -n 400 || true
    find "$appdir" -maxdepth 3 -type f -name '*.desktop' -print -exec sed -n '1,200p' {} \; || true
    find "$appdir" -maxdepth 3 -type f -name '*.desktop' -exec desktop-file-validate {} \; || true
    local app_bin=""
    local candidate
    for candidate in "$appdir/usr/bin/secluso-deploy" "$appdir/usr/bin/deploy_tool" "$appdir/usr/bin/"*; do
      if [ -f "$candidate" ] && [ -x "$candidate" ]; then
        app_bin="$candidate"
        break
      fi
    done
    if [ -n "$app_bin" ]; then
      echo "==> App binary candidate: $app_bin"
      file "$app_bin" || true
      ldd "$app_bin" || true
      readelf -d "$app_bin" || true
    fi
  fi

  local linuxdeploy_bin
  linuxdeploy_bin="$(find /root/.cache/tauri -maxdepth 1 -type f -name 'linuxdeploy-*.AppImage' ! -name 'linuxdeploy-plugin-*' | sort | head -n 1)"
  if [ -z "${linuxdeploy_bin}" ] && [ -f /root/.cache/tauri/linuxdeploy-x86_64.AppImage ]; then
    linuxdeploy_bin=/root/.cache/tauri/linuxdeploy-x86_64.AppImage
  fi
  echo "==> linuxdeploy binary candidate: ${linuxdeploy_bin:-<none>}"
  if [ -n "${linuxdeploy_bin}" ] && [ -x "${linuxdeploy_bin}" ]; then
    ls -la "${linuxdeploy_bin}" || true
    file "${linuxdeploy_bin}" || true
    sha256sum "${linuxdeploy_bin}" || true
  fi
}

main() {
  : "${TAURI_TARGET:?TAURI_TARGET is required}"
  : "${TAURI_RUNNER:=cargo}"
  : "${TAURI_BUNDLE_TARGETS_JSON:=["appimage","deb","rpm"]}"

  configure_rust_log
  configure_deterministic_build_env
  write_bundle_config

  if is_debug_enabled; then
    run_debug_preflight
  fi

  # This stage is intentionally strict... if packaging fails, fail the whole
  # build. In debug mode we emit deep diagnostics to aid trying to figure out what's going on (triage)
  if ! run_tauri_build; then
    echo "==> tauri build failed"
    if is_debug_enabled; then
      dump_failure_diagnostics
    else
      echo "==> Set DEBUG=1 to include deep Docker diagnostics."
    fi
    exit 1
  fi
}

main "$@"
