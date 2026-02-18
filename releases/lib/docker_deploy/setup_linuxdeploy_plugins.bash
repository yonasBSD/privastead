#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

install -d /root/.cache/tauri

linuxdeploy_url="https://github.com/tauri-apps/binary-releases/releases/download/linuxdeploy/linuxdeploy-x86_64.AppImage"
linuxdeploy_bin="/root/.cache/tauri/linuxdeploy-x86_64.AppImage"
linuxdeploy_tmp="${linuxdeploy_bin}.tmp"

# Keep linuxdeploy at the canonical path as a real AppImage binary.
# The appimage output plugin mutates the running linuxdeploy file with the dd utility
# If this path is a shell wrapper, that mutation corrupts the wrapper shebang
if [ ! -x "$linuxdeploy_bin" ]; then
  curl -fsSL "$linuxdeploy_url" -o "$linuxdeploy_tmp"
  mv "$linuxdeploy_tmp" "$linuxdeploy_bin"
  chmod +x "$linuxdeploy_bin"
fi

# The Tauri appimage pipeline looks for plugins in /root/.cache/tauri.
cat >/root/.cache/tauri/linuxdeploy-plugin-gstreamer.sh <<'EOS' && chmod +x /root/.cache/tauri/linuxdeploy-plugin-gstreamer.sh
#!/bin/sh
set -eu

if [ "${1:-}" = "--plugin-api-version" ]; then
  echo "0"
  exit 0
fi

exit 0
EOS

curl -fsSL https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh \
  -o /root/.cache/tauri/linuxdeploy-plugin-gtk.real.sh
chmod +x /root/.cache/tauri/linuxdeploy-plugin-gtk.real.sh

cat >/root/.cache/tauri/linuxdeploy-plugin-gtk.sh <<'EOS' && chmod +x /root/.cache/tauri/linuxdeploy-plugin-gtk.sh
#!/bin/sh
set -eu

exec bash /root/.cache/tauri/linuxdeploy-plugin-gtk.real.sh "$@"
EOS

cat >/root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage <<'EOS' && chmod +x /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage
#!/bin/sh
set -eu

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

resolved_appdir="${APPDIR:-}"
if [ -z "$resolved_appdir" ]; then
  prev=""
  for arg in "$@"; do
    if [ "$prev" = "--appdir" ]; then
      resolved_appdir="$arg"
      break
    fi
    case "$arg" in
      --appdir=*)
        resolved_appdir="${arg#--appdir=}"
        break
        ;;
    esac
    prev="$arg"
  done
fi

if [ -z "$resolved_appdir" ]; then
  echo "linuxdeploy appimage wrapper: APPDIR is not set and --appdir was not provided" >&2
  exit 1
fi

embedded_plugin=""
for candidate in \
  "${resolved_appdir}/plugins/linuxdeploy-plugin-appimage/AppRun" \
  "${resolved_appdir}/usr/bin/linuxdeploy-plugin-appimage" \
  "${resolved_appdir}/usr/lib/linuxdeploy-plugin-appimage/AppRun" \
  "${resolved_appdir}/usr/lib/linuxdeploy/plugins/linuxdeploy-plugin-appimage"
do
  if [ -x "$candidate" ]; then
    embedded_plugin="$candidate"
    break
  fi
done

if [ -z "$embedded_plugin" ]; then
  # linuxdeploy internals can vary between releases; probe deeper before failing.
  for candidate in $(find "${resolved_appdir}" -maxdepth 6 \( -type f -o -type l \) \
    \( -name 'linuxdeploy-plugin-appimage*' -o -name 'AppRun' \) \
    -perm -111 2>/dev/null | LC_ALL=C sort); do
    case "$(basename "$candidate")" in
      AppRun|linuxdeploy-plugin-appimage|linuxdeploy-plugin-appimage.AppImage)
        embedded_plugin="$candidate"
        break
        ;;
    esac
  done
fi

if [ -z "$embedded_plugin" ] && command -v linuxdeploy-plugin-appimage >/dev/null 2>&1; then
  embedded_plugin="$(command -v linuxdeploy-plugin-appimage)"
fi

if [ -z "$embedded_plugin" ]; then
  echo "linuxdeploy appimage wrapper: embedded plugin not found under $resolved_appdir" >&2
  exit 1
fi

# appimagetool already passes explicit mksquashfs timestamp flags.
# Leaving SOURCE_DATE_EPOCH set causes mksquashfs to abort.
if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
  unset SOURCE_DATE_EPOCH
fi

exec "$embedded_plugin" "$@"
EOS

# Keep extensionless aliases because different linuxdeploy execution modes may
# probe plugins by either extensioned or extensionless names.
ln -sf /root/.cache/tauri/linuxdeploy-plugin-gtk.sh /root/.cache/tauri/linuxdeploy-plugin-gtk
ln -sf /root/.cache/tauri/linuxdeploy-plugin-gstreamer.sh /root/.cache/tauri/linuxdeploy-plugin-gstreamer
ln -sf /root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage /root/.cache/tauri/linuxdeploy-plugin-appimage
