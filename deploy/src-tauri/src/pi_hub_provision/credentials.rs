//! SPDX-License-Identifier: GPL-3.0-or-later
use crate::pi_hub_provision::docker::{run_with_output, DockerCleanup};
use crate::pi_hub_provision::model::SigKey;
use anyhow::{bail, Result};
use secluso_update::{
    build_github_client, default_signers, download_and_verify_component, fetch_latest_release,
    parse_sig_keys, Component, DEFAULT_OWNER_REPO,
};
use std::fs;
use std::path::Path;
use std::process::Command;
use tauri::AppHandle;
use uuid::Uuid;

// Deploy accepts optional signer overrides from the UI/config as label:user pairs.
// We normalize and 'strictly' parse those values here so downstream verification can rely on
// non-empty signer identities. If no explicit signers are provided, we fall back
// to the updater's default signer policy so deploy and updater enforce the same trust set.
fn effective_signers(sig_keys: Option<&[SigKey]>) -> Result<Vec<secluso_update::Signer>> {
    let values = sig_keys
        .unwrap_or(&[])
        .iter()
        .map(|k| format!("{}:{}", k.name.trim(), k.github_user.trim()))
        .filter(|v| !v.trim().is_empty())
        .collect::<Vec<_>>();

    let parsed = parse_sig_keys(&values)?;
    if parsed.is_empty() {
        Ok(default_signers())
    } else {
        Ok(parsed)
    }
}

// The config tool binary is selected by architecture inside the signed bundle. Deploy might run on
// one host architecture while Docker executes for another platform (ex: cross-arch builds).
// We prefer DOCKER_DEFAULT_PLATFORM when available so we fetch the same artifact architecture that
// the containerized execution path expects, and only fall back to host architecture when no explicit
// Docker platform is configured.
fn config_tool_arch() -> String {
    if let Ok(platform) = std::env::var("DOCKER_DEFAULT_PLATFORM") {
        let normalized = platform.to_ascii_lowercase();
        if normalized.contains("amd64") || normalized.contains("x86_64") {
            return "x86_64".to_string();
        }
        if normalized.contains("arm64") || normalized.contains("aarch64") {
            return "aarch64".to_string();
        }
    }

    match std::env::consts::ARCH {
        "arm64" => "aarch64".to_string(),
        arch => arch.to_string(),
    }
}

// This function is the trust anchor for deploy credential generation. Instead of bootstrapping and
// executing a separate updater binary, deploy directly performs the same signed-release verification
// pipeline, where it fetchs latest release metadata, enforcs signer policy, verifies bundle integrity/signatures,
// and extracts the exact config_tool artifact for the target architecture. The returned bytes are only
// accepted AFTER all cryptographic checks succeed.
fn fetch_verified_config_tool(
    repo: &str,
    sig_keys: Option<&[SigKey]>,
    github_token: Option<&str>,
) -> Result<secluso_update::VerifiedComponent> {
    let owner_repo = if repo.trim().is_empty() {
        DEFAULT_OWNER_REPO.to_string()
    } else {
        repo.trim().to_string()
    };
    let token = github_token.map(|v| v.trim()).filter(|v| !v.is_empty());

    let client = build_github_client(20, token, "secluso-deploy")?;
    let release = fetch_latest_release(&client, &owner_repo)?;
    let signers = effective_signers(sig_keys)?;
    let arch = config_tool_arch();

    download_and_verify_component(
        &client,
        &release,
        Component::ConfigTool,
        &arch,
        None,
        &signers,
    )
}

// We seed a fresh Docker volume with two verified inputs, the config tool executable and the
// release bundle zip that produced it. Copying these pre-verified bytes into the volume keeps runtime
// execution deterministic and **avoids introducing additional network/download steps inside the container**
// Helps to limit moving parts during credential generation and preserves the verified-bytes trust chain.
fn seed_volume_with_inputs(
    app: &AppHandle,
    run_id: Uuid,
    work_path: &Path,
    volume_name: &str,
    container_name: &str,
    config_tool_bytes: &[u8],
    bundle_bytes: &[u8],
) -> Result<()> {
    let mut volume_cmd = Command::new("docker");
    volume_cmd.args(["volume", "create", volume_name]);
    run_with_output(app, run_id, "credentials", &mut volume_cmd)?;

    let mut create_cmd = Command::new("docker");
    create_cmd.args([
        "create",
        "--name",
        container_name,
        "-v",
        &format!("{}:/out", volume_name),
        "busybox",
    ]);
    run_with_output(app, run_id, "credentials", &mut create_cmd)?;

    let local_tool = work_path.join("secluso-config-tool");
    let local_bundle = work_path.join("secluso_bundle.zip");
    fs::write(&local_tool, config_tool_bytes)?;
    fs::write(&local_bundle, bundle_bytes)?;

    let mut cp_tool = Command::new("docker");
    cp_tool.args([
        "cp",
        &local_tool.display().to_string(),
        &format!("{}:/out/secluso-config-tool", container_name),
    ]);
    run_with_output(app, run_id, "credentials", &mut cp_tool)?;

    let mut cp_bundle = Command::new("docker");
    cp_bundle.args([
        "cp",
        &local_bundle.display().to_string(),
        &format!("{}:/out/secluso_bundle.zip", container_name),
    ]);
    run_with_output(app, run_id, "credentials", &mut cp_bundle)?;

    Ok(())
}

// The container runtime is used as an isolated executor for the config tool commands. The script string
// is supplied by trusted deploy code paths and runs against the seeded volume, so the container is really only
// responsible for deterministic credential generation from already-verified binaries.
fn run_config_tool_in_volume(
    app: &AppHandle,
    run_id: Uuid,
    volume_name: &str,
    script: &str,
) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.args(["run", "--rm"])
        .args(["-v", &format!("{}:/out", volume_name)])
        .arg("rust:1.82-bookworm")
        .args(["bash", "-lc", script]);

    run_with_output(app, run_id, "credentials", &mut cmd)
}

// After credential generation, we copy all output artifacts from the isolated volume into the host
// work directory in one step. Helps to keep the host-side interface simple and allows subsequent explicit
// output checks to decide whether the run produced the required files.
fn copy_volume_outputs_to_work(
    app: &AppHandle,
    run_id: Uuid,
    container_name: &str,
    work_path: &Path,
) -> Result<()> {
    let mut copy_cmd = Command::new("docker");
    copy_cmd.args([
        "cp",
        &format!("{}:/out/.", container_name),
        &work_path.display().to_string(),
    ]);
    run_with_output(app, run_id, "credentials", &mut copy_cmd)
}

// 1) Resolve and cryptographically verify config_tool from signed release artifacts.
// 2) Execute only that verified tool inside an isolated Docker volume.
// 3) Enforce expected outputs before returning success.
// This preserves the updater-grade verification guarantees
pub fn generate_secluso_credentials(
    app: &AppHandle,
    run_id: Uuid,
    work_path: &Path,
    repo: &str,
    sig_keys: Option<&[SigKey]>,
    github_token: Option<&str>,
) -> Result<()> {
    let verified = fetch_verified_config_tool(repo, sig_keys, github_token)?;

    let volume_name = format!("secluso-cred-{}", run_id);
    let container_name = format!("secluso-cred-copy-{}", run_id);
    let _cleanup = DockerCleanup::new(volume_name.clone(), container_name.clone());

    seed_volume_with_inputs(
        app,
        run_id,
        work_path,
        &volume_name,
        &container_name,
        &verified.component_bytes,
        &verified.bundle_bytes,
    )?;

    // The tool is copied in as bytes, so we explicitly mark it executable in the container before use.
    run_config_tool_in_volume(
        app,
        run_id,
        &volume_name,
        r#"
set -euo pipefail
chmod 0755 /out/secluso-config-tool
tmp_out=/tmp/secluso-generated
rm -rf "$tmp_out"
/out/secluso-config-tool --generate-camera-secret --dir "$tmp_out"
cp -a "$tmp_out"/. /out/
"#,
    )?;

    copy_volume_outputs_to_work(app, run_id, &container_name, work_path)?;

    // if required outputs are missing, treat the entire run as invalid.
    for f in ["camera_secret", "camera_secret_qrcode.png"] {
        let p = work_path.join(f);
        if !p.exists() {
            bail!("Expected credential output missing: {}", p.display());
        }
    }

    Ok(())
}

// mirrors camera secret generation and keeps the same trust model, verified binary bytes first,
// isolated execution second, strict output validation last.
pub fn generate_user_credentials_only(
    app: &AppHandle,
    run_id: Uuid,
    work_path: &Path,
    server_url: &str,
    repo: &str,
    sig_keys: Option<&[SigKey]>,
    github_token: Option<&str>,
) -> Result<()> {
    let verified = fetch_verified_config_tool(repo, sig_keys, github_token)?;

    let volume_name = format!("secluso-cred-{}", run_id);
    let container_name = format!("secluso-cred-copy-{}", run_id);
    let _cleanup = DockerCleanup::new(volume_name.clone(), container_name.clone());

    seed_volume_with_inputs(
        app,
        run_id,
        work_path,
        &volume_name,
        &container_name,
        &verified.component_bytes,
        &verified.bundle_bytes,
    )?;

    // Escape single quotes for shell-safe embedding inside the non-interactive bash command.
    let escaped_server_url = server_url.replace('\'', r#"'\''"#);
    run_config_tool_in_volume(
        app,
        run_id,
        &volume_name,
        &format!(
            r#"
set -euo pipefail
chmod 0755 /out/secluso-config-tool
tmp_out=/tmp/secluso-generated
rm -rf "$tmp_out"
/out/secluso-config-tool --generate-user-credentials --server-addr '{}' --dir "$tmp_out"
cp -a "$tmp_out"/. /out/
"#,
            escaped_server_url
        ),
    )?;

    copy_volume_outputs_to_work(app, run_id, &container_name, work_path)?;

    // Fail closed on missing output to prevent partially-generated credential state.
    let creds = work_path.join("user_credentials");
    if !creds.exists() {
        bail!("Expected credential output missing: {}", creds.display());
    }

    Ok(())
}
