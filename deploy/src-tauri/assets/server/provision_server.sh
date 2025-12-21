#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

# expected env vars
#   install_prefix, owner_repo
#   server_unit, updater_service, update_interval_secs
#   sudo_cmd ("" or "sudo -S -p ''" or "sudo"), enable_updater ("1"/"0")

emit() {
  local level="$1" step="$2" msg="$3"
  # minimal json escaping for quotes
  msg="${msg//\"/\\\"}"
  printf '::SECLUSO_EVENT::{"level":"%s","step":"%s","msg":"%s"}\n' "$level" "$step" "$msg"
}

trap 'emit "error" "trap" "Failed at line $LINENO (exit=$?)"' ERR

SUDO="${SUDO_CMD:-}"
if [[ -n "$SUDO" ]]; then
  emit "info" "sudo" "Using sudo wrapper: $SUDO"
else
  emit "info" "sudo" "No sudo wrapper (running as root)"
fi

emit "info" "config" "INSTALL_PREFIX=$INSTALL_PREFIX"
emit "info" "config" "OWNER_REPO=$OWNER_REPO"
emit "info" "config" "SERVER_UNIT=$SERVER_UNIT"
emit "info" "config" "ENABLE_UPDATER=$ENABLE_UPDATER"
emit "info" "config" "OVERWRITE=$OVERWRITE"
emit "info" "config" "FIRST_INSTALL=$FIRST_INSTALL"
emit "info" "config" "SIG_KEYS=${SIG_KEYS:-}"

UPDATE_INTERVAL_SECS="${UPDATE_INTERVAL_SECS:-1800}"

if [[ "${OVERWRITE:-0}" == "1" ]]; then
  emit "warn" "overwrite" "Overwrite enabled: stopping services and deleting $INSTALL_PREFIX"
  ${SUDO} systemctl stop "$UPDATER_SERVICE" 2>/dev/null || true
  ${SUDO} systemctl stop "$SERVER_UNIT" 2>/dev/null || true
  ${SUDO} systemctl disable "$UPDATER_SERVICE" 2>/dev/null || true
  ${SUDO} systemctl disable "$SERVER_UNIT" 2>/dev/null || true
  ${SUDO} rm -rf "$INSTALL_PREFIX"
  ${SUDO} mkdir -p "$INSTALL_PREFIX"
fi

emit "info" "deps" "Installing dependencies (apt-get)..."
${SUDO} apt-get update
${SUDO} apt-get install -y --no-install-recommends ca-certificates curl jq unzip coreutils git pkg-config libssl-dev

emit "info" "install" "Ensuring install dirs..."
${SUDO} mkdir -p "$INSTALL_PREFIX/bin" "$INSTALL_PREFIX/server/user_credentials" "$INSTALL_PREFIX/manifest"

emit "info" "download" "Resolving latest release tag..."
tag="$(curl -fsSL "https://api.github.com/repos/$OWNER_REPO/releases/latest" | jq -r '.tag_name // empty')"
if [[ -z "$tag" || "$tag" == "null" ]]; then
  emit "error" "download" "Missing tag name for $OWNER_REPO"
  exit 1
fi

WORK=/tmp/secluso-src
rm -rf "$WORK"
git clone --depth 1 --branch "$tag" "https://github.com/$OWNER_REPO.git" "$WORK"

emit "info" "build" "Building updater from source..."
cd "$WORK"
git -c protocol.file.allow=always submodule update --init --depth 1 update
cd "$WORK/update"
curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal
export PATH="$HOME/.cargo/bin:$PATH"
rustup toolchain install 1.85.0
cargo +1.85.0 build --release -p secluso-update
updater_bin="target/release/secluso-update"
if [[ ! -x "$updater_bin" ]]; then
  emit "error" "build" "Missing secluso-update binary after build"
  exit 1
fi
updater_name="$(basename "$updater_bin")"
${SUDO} install -m 0755 "$updater_bin" "$INSTALL_PREFIX/bin/$updater_name"

emit "info" "install" "Installing server with updater..."
if ! "$INSTALL_PREFIX/bin/$updater_name" --help 2>/dev/null | grep -q -- "--component"; then
  emit "error" "install" "Updater does not support --component"
  exit 1
fi
SIG_ARGS=""
if [[ -n "${SIG_KEYS:-}" ]]; then
  IFS=',' read -r -a sig_list <<< "$SIG_KEYS"
  for key in "${sig_list[@]}"; do
    if [[ -n "$key" ]]; then
      SIG_ARGS="$SIG_ARGS --sig-key $key"
    fi
  done
fi

${SUDO} timeout 90s "$INSTALL_PREFIX/bin/$updater_name" --component server --interval-secs 60 --github-timeout-secs 20 --github-repo "$OWNER_REPO"$SIG_ARGS || true
if [[ ! -x "$INSTALL_PREFIX/bin/secluso-server" ]]; then
  emit "error" "install" "secluso-server missing after updater run"
  exit 1
fi

emit "info" "install" "Installing bundled updater from release..."
arch="$(uname -m)"
case "$arch" in
  x86_64) archdir="x86_64-unknown-linux-gnu" ;;
  aarch64|arm64) archdir="aarch64-unknown-linux-gnu" ;;
  *) emit "warn" "arch" "Unsupported arch for bundled updater: $arch"; archdir="" ;;
esac
if [[ -n "$archdir" ]]; then
  rel_json="$(curl -fsSL "https://api.github.com/repos/$OWNER_REPO/releases/tags/$tag")"
  asset_name="$(echo "$rel_json" | jq -r '
    .assets | map(select(.name | test("^secluso-v.*\\.zip$"))) | if length==0 then empty else .[0].name end
  ')"
  if [[ -n "$asset_name" && "$asset_name" != "null" ]]; then
    rm -rf /tmp/secluso_bundle && mkdir -p /tmp/secluso_bundle
    curl -fL -o /tmp/secluso_bundle.zip "https://github.com/$OWNER_REPO/releases/download/$tag/$asset_name"
    unzip -o /tmp/secluso_bundle.zip -d /tmp/secluso_bundle >/dev/null
    root="/tmp/secluso_bundle"
    maybe="$(find /tmp/secluso_bundle -maxdepth 2 -type f -name manifest.json | head -n 1 || true)"
    if [[ -n "$maybe" ]]; then
      root="$(dirname "$maybe")"
    fi
    if [[ -x "$root/$archdir/secluso-update" ]]; then
      ${SUDO} install -m 0755 "$root/$archdir/secluso-update" "$INSTALL_PREFIX/bin/secluso-update"
      updater_name="secluso-update"
    else
      emit "warn" "install" "bundled secluso-update missing for $archdir"
    fi
  else
    emit "warn" "install" "No release bundle asset found for updater"
  fi
fi

rm -rf "$WORK"

if [[ "${FIRST_INSTALL:-0}" == "1" ]]; then
  emit "info" "first_install" "Placing secrets + writing systemd units..."

  ${SUDO} mkdir -p "$INSTALL_PREFIX/server/user_credentials"
  ${SUDO} install -m 0600 /tmp/service_account_key.json "$INSTALL_PREFIX/server/service_account_key.json"
  ${SUDO} install -m 0600 /tmp/user_credentials "$INSTALL_PREFIX/server/user_credentials/user_credentials"
  ${SUDO} rm -f /tmp/service_account_key.json /tmp/user_credentials

  emit "info" "systemd" "Writing server unit..."
  ${SUDO} tee "/etc/systemd/system/$SERVER_UNIT" >/dev/null <<EOFUNIT
[Unit]
Description=Secluso Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
WorkingDirectory=$INSTALL_PREFIX/server
ExecStart=$INSTALL_PREFIX/bin/secluso-server
Restart=always
RestartSec=1
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOFUNIT

  if [[ "${ENABLE_UPDATER:-1}" == "1" && -x "$INSTALL_PREFIX/bin/$updater_name" ]]; then
    emit "info" "systemd" "Writing updater service..."
    ${SUDO} tee "/etc/systemd/system/$UPDATER_SERVICE" >/dev/null <<EOFUPD
[Unit]
Description=Secluso Auto Updater

[Service]
Type=simple
ExecStart=$INSTALL_PREFIX/bin/$updater_name --component server --interval-secs $UPDATE_INTERVAL_SECS --github-timeout-secs 20 --restart-unit $SERVER_UNIT --github-repo $OWNER_REPO${SIG_ARGS}
Restart=always
RestartSec=2
EOFUPD
  fi

  emit "info" "systemd" "daemon-reload + enable services..."
  ${SUDO} systemctl daemon-reload
  ${SUDO} systemctl enable --now "$SERVER_UNIT"
  ${SUDO} systemctl restart "$SERVER_UNIT"

  if [[ "${ENABLE_UPDATER:-1}" == "1" ]]; then
    if [[ -x "$INSTALL_PREFIX/bin/$updater_name" ]]; then
      ${SUDO} systemctl enable --now "$UPDATER_SERVICE"
      emit "info" "systemd" "updater service enabled"
    else
      emit "warn" "updater" "secluso-updater not found, skipping auto updates"
    fi
  else
    ${SUDO} systemctl disable --now "$UPDATER_SERVICE" 2>/dev/null || true
    emit "warn" "systemd" "updater disabled"
  fi

else
  emit "info" "restart" "Update-only path: restarting $SERVER_UNIT..."
  ${SUDO} systemctl restart "$SERVER_UNIT" || true
fi

emit "info" "done" "DONE"
