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
  CARGO_INCREMENTAL=0 pnpm tauri build \
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
