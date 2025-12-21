use crate::pi_hub_provision::docker::{DockerCleanup, run_with_output};
use anyhow::{bail, Result};
use std::path::Path;
use std::process::Command;
use tauri::AppHandle;
use uuid::Uuid;

pub fn generate_secluso_credentials(
  app: &AppHandle,
  run_id: Uuid,
  work_path: &Path,
  repo: &str,
  sig_keys: Option<&[crate::pi_hub_provision::model::SigKey]>,
) -> Result<()> {
  // use a docker volume so we avoid host bind mounts during credential creation
  let volume_name = format!("secluso-cred-{}", run_id);
  let container_name = format!("secluso-cred-copy-{}", run_id);

  let _cleanup = DockerCleanup::new(volume_name.clone(), container_name.clone());

  let sig_keys_env = sig_keys
    .unwrap_or(&[])
    .iter()
    .map(|k| format!("{}:{}", k.name.trim(), k.github_user.trim()))
    .filter(|v| !v.trim().is_empty())
    .collect::<Vec<_>>()
    .join(",");

  // create the volume and run the config tool inside a rust image
  let mut volume_cmd = Command::new("docker");
  volume_cmd.args(["volume", "create", &volume_name]);
  run_with_output(app, run_id, "credentials", &mut volume_cmd)?;

  let mut cmd = Command::new("docker");
  cmd.args(["run", "--rm"])
    .args(["-v", &format!("{}:/out", volume_name)])
    .args(["-e", &format!("SIG_KEYS={}", sig_keys_env)])
    .arg("rust:1.82-bookworm")
    .args(["bash", "-lc"])
    .arg(format!(
      r#"
set -euo pipefail
apt-get update
apt-get install -y --no-install-recommends ca-certificates curl jq unzip git pkg-config libssl-dev
curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal
if [[ -f /usr/local/cargo/env ]]; then
  . /usr/local/cargo/env
elif [[ -f /root/.cargo/env ]]; then
  . /root/.cargo/env
fi
export PATH="/usr/local/cargo/bin:/root/.cargo/bin:$PATH"
rustup toolchain install 1.85.0

curl -fsSL -o /tmp/release.json "https://api.github.com/repos/{repo}/releases/latest" || {{
  echo "Failed to fetch release metadata" >&2
  exit 1
}}
tag="$(jq -r '.tag_name // empty' /tmp/release.json)"
if [[ -z "$tag" || "$tag" == "null" ]]; then
  echo "Missing tag name in release metadata" >&2
  exit 1
fi

rm -rf /tmp/secluso-src
git clone --depth 1 --branch "$tag" "https://github.com/{repo}.git" /tmp/secluso-src
cd /tmp/secluso-src
git -c protocol.file.allow=always submodule update --init --depth 1 update
cd /tmp/secluso-src/update
cargo +1.85.0 build --release -p secluso-update

updater_bin="target/release/secluso-update"
if [[ ! -x "$updater_bin" ]]; then
  echo "Missing secluso-update binary after build" >&2
  ls -la target/release >&2 || true
  exit 1
fi

mkdir -p /tmp/secluso-bin
install -m 0755 "$updater_bin" /tmp/secluso-bin/$(basename "$updater_bin")
cd /tmp/secluso-bin

SIG_ARGS=""
if [[ -n "${{SIG_KEYS:-}}" ]]; then
  IFS=',' read -r -a sig_list <<< "$SIG_KEYS"
  for key in "${{sig_list[@]}}"; do
    if [[ -n "$key" ]]; then
      SIG_ARGS="$SIG_ARGS --sig-key $key"
    fi
  done
fi

if ! ./$(basename "$updater_bin") --help 2>/dev/null | grep -q -- "--component"; then
  echo "Updater does not support --component" >&2
  exit 1
fi
./$(basename "$updater_bin") --component config_tool --interval-secs 60 --github-timeout-secs 20 --github-repo "{repo}"${{SIG_ARGS}}

tool="$(find /tmp/secluso-bin -maxdepth 1 -type f \( -name 'secluso-config-tool' -o -name 'secluso-config' \) | head -n 1)"
if [[ -z "$tool" ]]; then
  echo "Missing config tool binary after updater run" >&2
  ls -la /tmp/secluso-bin >&2 || true
  exit 1
fi

"$tool" --generate-camera-secret --dir /out
"#,
      repo = repo
    ));

  run_with_output(app, run_id, "credentials", &mut cmd)?;

  // copy files out of the volume with a throwaway container
  let mut create_cmd = Command::new("docker");
  create_cmd.args(["create", "--name", &container_name, "-v", &format!("{}:/out", volume_name), "busybox"]);
  run_with_output(app, run_id, "credentials", &mut create_cmd)?;

  let mut copy_cmd = Command::new("docker");
  copy_cmd.args(["cp", &format!("{}:/out/.", container_name), &work_path.display().to_string()]);
  if let Err(_) = run_with_output(app, run_id, "credentials", &mut copy_cmd) {
    let retry_name = format!("{}-retry", container_name);
    let mut retry_create = Command::new("docker");
    retry_create.args(["create", "--name", &retry_name, "-v", &format!("{}:/out", volume_name), "busybox"]);
    run_with_output(app, run_id, "credentials", &mut retry_create)?;

    let mut retry_copy = Command::new("docker");
    retry_copy.args(["cp", &format!("{}:/out/.", retry_name), &work_path.display().to_string()]);
    run_with_output(app, run_id, "credentials", &mut retry_copy)?;
  }

  // sanity check outputs so we fail with a clear error
  for f in ["camera_secret", "camera_secret_qrcode.png"] {
    let p = work_path.join(f);
    if !p.exists() {
      bail!("Expected credential output missing: {}", p.display());
    }
  }

  Ok(())
}

pub fn generate_user_credentials_only(
  app: &AppHandle,
  run_id: Uuid,
  work_path: &Path,
  server_url: &str,
  repo: &str,
  sig_keys: Option<&[crate::pi_hub_provision::model::SigKey]>,
) -> Result<()> {
  // run the config tool with only the user credentials command
  let volume_name = format!("secluso-cred-{}", run_id);
  let container_name = format!("secluso-cred-copy-{}", run_id);

  let _cleanup = DockerCleanup::new(volume_name.clone(), container_name.clone());

  let sig_keys_env = sig_keys
    .unwrap_or(&[])
    .iter()
    .map(|k| format!("{}:{}", k.name.trim(), k.github_user.trim()))
    .filter(|v| !v.trim().is_empty())
    .collect::<Vec<_>>()
    .join(",");

  let mut volume_cmd = Command::new("docker");
  volume_cmd.args(["volume", "create", &volume_name]);
  run_with_output(app, run_id, "credentials", &mut volume_cmd)?;

  let mut cmd = Command::new("docker");
  cmd.args(["run", "--rm"])
    .args(["-v", &format!("{}:/out", volume_name)])
    .args(["-e", &format!("SIG_KEYS={}", sig_keys_env)])
    .arg("rust:1.82-bookworm")
    .args(["bash", "-lc"])
    .arg(format!(
      r#"
set -euo pipefail
apt-get update
apt-get install -y --no-install-recommends ca-certificates curl jq unzip git pkg-config libssl-dev
curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal
if [[ -f /usr/local/cargo/env ]]; then
  . /usr/local/cargo/env
elif [[ -f /root/.cargo/env ]]; then
  . /root/.cargo/env
fi
export PATH="/usr/local/cargo/bin:/root/.cargo/bin:$PATH"
rustup toolchain install 1.85.0

curl -fsSL -o /tmp/release.json "https://api.github.com/repos/{repo}/releases/latest"
tag="$(jq -r '.tag_name // empty' /tmp/release.json)"
if [[ -z "$tag" || "$tag" == "null" ]]; then
  echo "Missing tag name in release metadata" >&2
  exit 1
fi

rm -rf /tmp/secluso-src
git clone --depth 1 --branch "$tag" "https://github.com/{repo}.git" /tmp/secluso-src
cd /tmp/secluso-src
git -c protocol.file.allow=always submodule update --init --depth 1 update
cd /tmp/secluso-src/update
cargo +1.85.0 build --release -p secluso-update

updater_bin="target/release/secluso-update"
if [[ ! -x "$updater_bin" ]]; then
  echo "Missing secluso-update binary after build" >&2
  ls -la target/release >&2 || true
  exit 1
fi

mkdir -p /tmp/secluso-bin
install -m 0755 "$updater_bin" /tmp/secluso-bin/$(basename "$updater_bin")
cd /tmp/secluso-bin

SIG_ARGS=""
if [[ -n "${{SIG_KEYS:-}}" ]]; then
  IFS=',' read -r -a sig_list <<< "$SIG_KEYS"
  for key in "${{sig_list[@]}}"; do
    if [[ -n "$key" ]]; then
      SIG_ARGS="$SIG_ARGS --sig-key $key"
    fi
  done
fi

if ! ./$(basename "$updater_bin") --help 2>/dev/null | grep -q -- "--component"; then
  echo "Updater does not support --component" >&2
  exit 1
fi
./$(basename "$updater_bin") --component config_tool --interval-secs 60 --github-timeout-secs 20 --github-repo "{repo}"${{SIG_ARGS}}

tool="$(find /tmp/secluso-bin -maxdepth 1 -type f \( -name 'secluso-config-tool' -o -name 'secluso-config' \) | head -n 1)"
if [[ -z "$tool" ]]; then
  echo "Missing config tool binary after updater run" >&2
  ls -la /tmp/secluso-bin >&2 || true
  exit 1
fi

"$tool" --generate-user-credentials --server-addr "{server_url}" --dir /out
"#,
      repo = repo,
      server_url = server_url
    ));

  run_with_output(app, run_id, "credentials", &mut cmd)?;

  let mut create_cmd = Command::new("docker");
  create_cmd.args(["create", "--name", &container_name, "-v", &format!("{}:/out", volume_name), "busybox"]);
  run_with_output(app, run_id, "credentials", &mut create_cmd)?;

  let mut copy_cmd = Command::new("docker");
  copy_cmd.args(["cp", &format!("{}:/out/.", container_name), &work_path.display().to_string()]);
  if let Err(_) = run_with_output(app, run_id, "credentials", &mut copy_cmd) {
    let retry_name = format!("{}-retry", container_name);
    let mut retry_create = Command::new("docker");
    retry_create.args(["create", "--name", &retry_name, "-v", &format!("{}:/out", volume_name), "busybox"]);
    run_with_output(app, run_id, "credentials", &mut retry_create)?;

    let mut retry_copy = Command::new("docker");
    retry_copy.args(["cp", &format!("{}:/out/.", retry_name), &work_path.display().to_string()]);
    run_with_output(app, run_id, "credentials", &mut retry_copy)?;
  }

  let creds = work_path.join("user_credentials");
  if !creds.exists() {
    bail!("Expected credential output missing: {}", creds.display());
  }

  Ok(())
}
