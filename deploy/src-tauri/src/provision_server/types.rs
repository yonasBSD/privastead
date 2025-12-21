//! SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "kind")]
pub enum SshAuth {
  #[serde(rename = "password")]
  Password { password: String },

  #[serde(rename = "keyfile")]
  KeyFile { path: String },

  #[serde(rename = "keytext")]
  KeyText { text: String },
}

#[derive(Debug, Deserialize, Clone)]
pub struct SudoSpec {
  /// same uses login password if available and otherwise assumes passwordless sudo
  /// password uses the provided password
  pub mode: String,
  pub password: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SshTarget {
  pub host: String,
  pub port: u16,
  pub user: String,
  pub auth: SshAuth,
  pub sudo: SudoSpec,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AutoUpdaterPlan {
  pub enable: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SigKey {
  pub name: String,
  pub github_user: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerSecrets {
  pub service_account_key_path: String,
  pub server_url: String,
  pub user_credentials_qr_path: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerPlan {
  pub use_docker: bool,
  pub protect_packages: bool,
  pub auto_updater: AutoUpdaterPlan,
  pub secrets: Option<ServerSecrets>,
  pub overwrite: Option<bool>,
  pub sig_keys: Option<Vec<SigKey>>,
  pub binaries_repo: Option<String>,
}
