use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type")]
pub enum ProvisionEvent {
  #[serde(rename = "step_start")]
  StepStart {
    run_id: Uuid,
    step: String,
    title: String,
  },

  #[serde(rename = "log")]
  Log {
    run_id: Uuid,
    level: String,
    step: Option<String>,
    line: String,
  },

  #[serde(rename = "step_ok")]
  StepOk {
    run_id: Uuid,
    step: String,
  },

  #[serde(rename = "step_error")]
  StepError {
    run_id: Uuid,
    step: String,
    message: String,
  },

  #[serde(rename = "done")]
  Done {
    run_id: Uuid,
    ok: bool,
  },
}

pub fn emit(app: &AppHandle, ev: ProvisionEvent) {
  // ignore errors when no listener is attached
  let _ = app.emit("provision:event", ev);
}

pub fn step_start(app: &AppHandle, run_id: Uuid, step: &str, title: &str) {
  emit(
    app,
    ProvisionEvent::StepStart {
      run_id,
      step: step.to_string(),
      title: title.to_string(),
    },
  );
}

pub fn step_ok(app: &AppHandle, run_id: Uuid, step: &str) {
  emit(
    app,
    ProvisionEvent::StepOk {
      run_id,
      step: step.to_string(),
    },
  );
}

pub fn step_error(app: &AppHandle, run_id: Uuid, step: &str, msg: impl Into<String>) {
  emit(
    app,
    ProvisionEvent::StepError {
      run_id,
      step: step.to_string(),
      message: msg.into(),
    },
  );
}

pub fn log_line(app: &AppHandle, run_id: Uuid, level: &str, step: Option<&str>, line: impl Into<String>) {
  emit(
    app,
    ProvisionEvent::Log {
      run_id,
      level: level.to_string(),
      step: step.map(|s| s.to_string()),
      line: line.into(),
    },
  );
}

pub fn handle_event_line(app: &AppHandle, run_id: Uuid, default_level: &str, step: &str, line: &str) {
  // accept structured lines from the docker scripts
  const PREFIX: &str = "::SECLUSO_EVENT::";
  if let Some(rest) = line.strip_prefix(PREFIX) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) {
      let level = v.get("level").and_then(|x| x.as_str()).unwrap_or(default_level);
      let rstep = v.get("step").and_then(|x| x.as_str()).unwrap_or(step);
      let msg = v.get("msg").and_then(|x| x.as_str()).unwrap_or(line);
      log_line(app, run_id, level, Some(rstep), msg.to_string());
      return;
    }
  }
  if !line.trim().is_empty() {
    log_line(app, run_id, default_level, Some(step), line.to_string());
  }
}
