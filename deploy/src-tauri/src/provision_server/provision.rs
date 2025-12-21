use crate::pi_hub_provision::credentials::generate_user_credentials_only;
use crate::pi_hub_provision::temp::shared_temp_dir;
use crate::provision_server::events::{log_line, step_ok, step_start};
use crate::provision_server::script::remote_provision_script;
use crate::provision_server::ssh::{connect_ssh, exec_remote_script_streaming, scp_upload_bytes, sudo_prefix};
use crate::provision_server::types::{ServerPlan, SshTarget};
use anyhow::{bail, Context, Result};
use ssh2::Session;
use std::io::Read;
use std::path::PathBuf;
use tauri::AppHandle;
use uuid::Uuid;

pub fn run_provision(app: &AppHandle, run_id: Uuid, target: SshTarget, plan: ServerPlan) -> Result<()> {
  fn normalize_repo(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    if let Some(idx) = trimmed.find("github.com/") {
      let repo = &trimmed[idx + "github.com/".len()..];
      return repo.trim_end_matches(".git").to_string();
    }
    trimmed.trim_end_matches(".git").to_string()
  }

  // constants aligned with the bash script
  let install_prefix = "/opt/secluso";
  let server_unit = "secluso-server.service";
  let updater_service = "secluso-updater.service";
  let update_interval_secs = "1800";
  let owner_repo = plan
    .binaries_repo
    .as_ref()
    .map(|repo| normalize_repo(repo))
    .unwrap_or_else(|| "secluso/secluso".to_string());

  if plan.use_docker {
    // this code uses the bundle zip flow today because it matches the script
    // keep the ui moving by warning and continuing
    log_line(app, run_id, "warn", Some("plan"), "useDocker=true is currently ignored; proceeding with bundle-zip installer.".to_string());
  }
  if plan.protect_packages {
    log_line(app, run_id, "warn", Some("plan"), "protectPackages requested; not implemented in this backend yet (noop).".to_string());
  }

  step_start(app, run_id, "ssh_connect", "Connecting via SSH");
  let (sess, _temps) = connect_ssh(&target)?;
  step_ok(app, run_id, "ssh_connect");

  let (sudo_cmd, sudo_pw) = sudo_prefix(&target);

  // detect remote state
  step_start(app, run_id, "detect", "Detecting remote install state");
  let remote_has_bin = remote_test(&sess, &sudo_cmd, &format!("test -x {install_prefix}/bin/secluso-server"))?;
  let remote_has_unit = remote_test(
    &sess,
    &sudo_cmd,
    &format!("systemctl list-unit-files --type=service | awk '{{print $1}}' | grep -qx '{server_unit}'"),
  )?;
  log_line(app, run_id, "info", Some("detect"), format!("REMOTE_HAS_BIN={remote_has_bin}"));
  log_line(app, run_id, "info", Some("detect"), format!("REMOTE_HAS_UNIT={remote_has_unit}"));
  step_ok(app, run_id, "detect");

  let overwrite = plan.overwrite.unwrap_or(false);
  let sig_keys = plan.sig_keys.clone().unwrap_or_default();

  // decide if this is a first install
  let first_install = overwrite || !(remote_has_bin && remote_has_unit);

  // step 2 generate and upload secrets on first install
  step_start(app, run_id, "secrets", "Generating and uploading secrets");
  if first_install {
    let secrets = plan.secrets.as_ref().context("Missing secrets config")?;

    let work_dir = shared_temp_dir("secluso-server-creds").context("creating temp work dir")?;
    let work_path = work_dir.path();
    let sig_keys = plan.sig_keys.as_ref().map(|keys| {
      keys
        .iter()
        .map(|k| crate::pi_hub_provision::model::SigKey {
          name: k.name.trim().to_string(),
          github_user: k.github_user.trim().to_string(),
        })
        .collect::<Vec<_>>()
    });
    generate_user_credentials_only(
      app,
      run_id,
      work_path,
      &secrets.server_url,
      &owner_repo,
      sig_keys.as_deref(),
    )?;

    let sa_path = PathBuf::from(&secrets.service_account_key_path);
    let sa = std::fs::read(&sa_path).with_context(|| format!("Missing service account key at {}", sa_path.display()))?;

    let uc_path = work_path.join("user_credentials");
    let uc = std::fs::read(&uc_path).with_context(|| format!("Missing user credentials at {}", uc_path.display()))?;

    let qr_src = work_path.join("user_credentials_qrcode.png");
    if !qr_src.exists() {
      bail!("Missing QR code at {}", qr_src.display());
    }
    let qr_path = PathBuf::from(&secrets.user_credentials_qr_path);
    if let Some(parent) = qr_path.parent() {
      if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent)?;
      }
    }
    std::fs::copy(&qr_src, &qr_path).with_context(|| format!("Saving QR code to {}", qr_path.display()))?;

    scp_upload_bytes(&sess, "/tmp/service_account_key.json", 0o600, &sa)?;
    scp_upload_bytes(&sess, "/tmp/user_credentials", 0o600, &uc)?;
    step_ok(app, run_id, "secrets");
  } else {
    log_line(app, run_id, "info", Some("secrets"), "Skipping secrets upload for non-first install.".to_string());
    step_ok(app, run_id, "secrets");
  }

  // step 3 run the remote provision script
  step_start(app, run_id, "remote", "Running remote installer");
  let envs = vec![
    ("INSTALL_PREFIX", install_prefix.to_string()),
    ("OWNER_REPO", owner_repo.to_string()),
    ("SERVER_UNIT", server_unit.to_string()),
    ("UPDATER_SERVICE", updater_service.to_string()),
    ("UPDATE_INTERVAL_SECS", update_interval_secs.to_string()),
    ("SUDO_CMD", sudo_cmd.clone()),
    ("ENABLE_UPDATER", if plan.auto_updater.enable { "1".to_string() } else { "0".to_string() }),
    ("OVERWRITE", if overwrite { "1".to_string() } else { "0".to_string() }),
    ("FIRST_INSTALL", if first_install { "1".to_string() } else { "0".to_string() }),
    (
      "SIG_KEYS",
      sig_keys
        .iter()
        .map(|k| format!("{}:{}", k.name.trim(), k.github_user.trim()))
        .filter(|v| !v.trim().is_empty())
        .collect::<Vec<_>>()
        .join(","),
    ),
  ];

  exec_remote_script_streaming(
    app,
    run_id,
    "remote",
    &sess,
    &envs.iter().map(|(k, v)| (*k, v.clone())).collect::<Vec<_>>(),
    sudo_pw,
    remote_provision_script(),
  )?;

  step_ok(app, run_id, "remote");
  Ok(())
}

fn remote_test(sess: &Session, sudo_cmd: &str, cmd: &str) -> Result<bool> {
  // returns true if the command exits 0
  let full = if sudo_cmd.is_empty() {
    format!("bash -lc '{}'", cmd.replace('\'', r"'\''"))
  } else {
    // sudo wrapper may include -s so do not use it here since tests should not prompt
    let sudo_plain = if sudo_cmd.contains("-S") { "sudo".to_string() } else { sudo_cmd.to_string() };
    format!("bash -lc '{} {}'", sudo_plain, cmd.replace('\'', r"'\''"))
  };

  let mut channel = sess.channel_session().context("Failed to open SSH channel")?;
  channel.exec(&full).context("Remote test exec failed")?;
  // consume output and ignore
  let mut sink = String::new();
  channel.read_to_string(&mut sink).ok();
  let mut sink2 = String::new();
  channel.stderr().read_to_string(&mut sink2).ok();
  channel.wait_close().ok();
  let code = channel.exit_status().unwrap_or(255);
  Ok(code == 0)
}
