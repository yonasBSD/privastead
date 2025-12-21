mod events;
mod provision;
mod script;
mod ssh;
mod types;

use crate::provision_server::events::{emit, log_line, step_error, step_ok, step_start, ProvisionEvent};
use crate::provision_server::provision::run_provision;
use crate::provision_server::ssh::connect_ssh;
use crate::provision_server::types::{ServerPlan, SshTarget};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::Read;
use tauri::AppHandle;
use uuid::Uuid;

// tauri commands

#[tauri::command]
pub async fn test_server_ssh(app: AppHandle, target: SshTarget) -> Result<(), String> {
  let run_id = Uuid::new_v4();
  step_start(&app, run_id, "ssh_test", "Testing SSH connection");

  let app2 = app.clone();
  let res = tokio::task::spawn_blocking(move || -> Result<()> {
    let (sess, _temps) = connect_ssh(&target)?;
    let mut channel = sess.channel_session().context("Failed to open SSH channel")?;
    channel.exec("bash -lc 'echo SECLUSO_SSH_OK && uname -a'").context("Remote exec failed")?;

    let mut out = String::new();
    channel.read_to_string(&mut out).ok();
    let mut err = String::new();
    channel.stderr().read_to_string(&mut err).ok();

    if !out.trim().is_empty() {
      for line in out.lines() {
        log_line(&app2, run_id, "info", Some("ssh_test"), line.to_string());
      }
    }
    if !err.trim().is_empty() {
      for line in err.lines() {
        log_line(&app2, run_id, "warn", Some("ssh_test"), line.to_string());
      }
    }

    channel.wait_close().ok();
    let code = channel.exit_status().unwrap_or(255);
    if code != 0 {
      bail!("SSH test command failed (exit={code})");
    }
    Ok(())
  })
  .await
  .map_err(|e| e.to_string())
  .and_then(|r| r.map_err(|e| e.to_string()));

  match res {
    Ok(_) => {
      step_ok(&app, run_id, "ssh_test");
      emit(&app, ProvisionEvent::Done { run_id, ok: true });
      Ok(())
    }
    Err(e) => {
      step_error(&app, run_id, "ssh_test", &e);
      emit(&app, ProvisionEvent::Done { run_id, ok: false });
      Err(e)
    }
  }
}

#[derive(Debug, Serialize)]
pub struct ProvisionStart {
  pub run_id: Uuid,
}

#[tauri::command]
pub async fn provision_server(app: AppHandle, target: SshTarget, plan: ServerPlan) -> Result<ProvisionStart, String> {
  let run_id = Uuid::new_v4();

  // return so ui can start listening while work runs in the background
  let app2 = app.clone();
  tokio::task::spawn_blocking(move || {
    if let Err(e) = run_provision(&app2, run_id, target, plan) {
      // ensure we end with done false
      emit(&app2, ProvisionEvent::Done { run_id, ok: false });
      log_line(&app2, run_id, "error", Some("fatal"), format!("{e:#}"));
    } else {
      emit(&app2, ProvisionEvent::Done { run_id, ok: true });
    }
  });

  Ok(ProvisionStart { run_id })
}
