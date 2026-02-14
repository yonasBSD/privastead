#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

install -d /root/.cache/tauri

# The Tauri appimage pipeline always looks for linuxdeploy plugins in
# /root/.cache/tauri. We pre-seed deterministic wrappers there so plugin
# resolution behaves consistently in Docker builders.
cat >/root/.cache/tauri/linuxdeploy-plugin-gstreamer.sh <<'EOF' && chmod +x /root/.cache/tauri/linuxdeploy-plugin-gstreamer.sh
#!/bin/sh
set -eu

if [ "${1:-}" = "--plugin-api-version" ]; then
  echo "0"
  exit 0
fi

exit 0
EOF

curl -fsSL https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh \
  -o /root/.cache/tauri/linuxdeploy-plugin-gtk.real.sh
chmod +x /root/.cache/tauri/linuxdeploy-plugin-gtk.real.sh

cat >/root/.cache/tauri/linuxdeploy-plugin-gtk.sh <<'EOF' && chmod +x /root/.cache/tauri/linuxdeploy-plugin-gtk.sh
#!/bin/sh
set -eu

debug_enabled() {
  case "${DEBUG:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

if debug_enabled; then
  log_file="${LINUXDEPLOY_GTK_WRAPPER_LOG:-/tmp/linuxdeploy-plugin-gtk-wrapper.log}"
  {
    printf 'ts=%s pid=%s shell=%s bash_version=%s args=' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$$" "$0" "${BASH_VERSION:-<none>}"
    printf '%s ' "$@"
    printf '\n'
  } >>"$log_file" 2>/dev/null || true
fi

exec bash /root/.cache/tauri/linuxdeploy-plugin-gtk.real.sh "$@"
EOF

cat >/root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage <<'EOF' && chmod +x /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage
#!/bin/sh
set -eu

debug_enabled() {
  case "${DEBUG:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

if debug_enabled; then
  log_file="${LINUXDEPLOY_APPIMAGE_WRAPPER_LOG:-/tmp/linuxdeploy-plugin-appimage-wrapper.log}"
  {
    printf 'ts=%s pid=%s shell=%s args=' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$$" "$0"
    printf '%s ' "$@"
    printf '\n'
    printf 'env APPDIR=%s OUTPUT=%s LINUXDEPLOY=%s ARCH=%s\n' "${APPDIR:-}" "${OUTPUT:-}" "${LINUXDEPLOY:-}" "${ARCH:-}"
  } >>"$log_file" 2>/dev/null || true
fi

while [ "${1:-}" = "--appimage-extract-and-run" ]; do
  shift
done

case "${1:-}" in
  --plugin-api-version)
    echo "0"
    exit 0
    ;;
  --plugin-type)
    echo "output"
    exit 0
    ;;
  --help)
    echo "linuxdeploy appimage wrapper: delegates appimage output to embedded plugin"
    exit 0
    ;;
esac

if [ -z "${APPDIR:-}" ]; then
  echo "linuxdeploy appimage wrapper: APPDIR is not set" >&2
  exit 1
fi

embedded_plugin=""
for candidate in \
  "${APPDIR}/plugins/linuxdeploy-plugin-appimage/AppRun" \
  "${APPDIR}/usr/bin/linuxdeploy-plugin-appimage"
do
  if [ -x "$candidate" ]; then
    embedded_plugin="$candidate"
    break
  fi
done

if [ -n "$embedded_plugin" ]; then
  exec "$embedded_plugin" "$@"
fi

echo "linuxdeploy appimage wrapper: embedded plugin not found in expected locations under $APPDIR" >&2
exit 1
EOF

# Keep extensionless aliases because different linuxdeploy execution modes may
# probe plugins by either extensioned or extensionless names.
ln -sf /root/.cache/tauri/linuxdeploy-plugin-gtk.sh /root/.cache/tauri/linuxdeploy-plugin-gtk
ln -sf /root/.cache/tauri/linuxdeploy-plugin-gstreamer.sh /root/.cache/tauri/linuxdeploy-plugin-gstreamer
ln -sf /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage /root/.cache/tauri/linuxdeploy-plugin-appimage
