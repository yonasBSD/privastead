//! SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use docopt::Docopt;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use semver::Version;
use serde::Deserialize;
use serde_json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;
use zip::ZipArchive;

use openpgp::cert::Cert;
use openpgp::parse::stream::{
    DetachedVerifierBuilder, GoodChecksum, MessageLayer, MessageStructure, VerificationHelper,
};
use openpgp::parse::Parse;
use openpgp::policy::StandardPolicy;
use openpgp::{Fingerprint, KeyHandle};
use sequoia_openpgp as openpgp;

// Binaries stored in INSTALL_ROOT/bin/BINARY_NAME
const INSTALL_ROOT: &str = "/opt/secluso";

// Where we fetch releases from (default).
const DEFAULT_OWNER_REPO: &str = "secluso/secluso";

/// A signer entry... the label field controls the signature filename in the zip,
/// and the github_user field controls which GitHub account’s keys we accept for that signature.
#[derive(Clone)]
struct Signer {
    label: String,
    github_user: String,
}

/// Primary use point here two signatures, two github contributors, two different keys
/// The zip will contain:
///   manifest.json.jkaczman-1.asc
///   manifest.json.arrdalan-2.asc
///
/// And both will be verified against keys from:
///   https://github.com/jkaczman.gpg, https://github.com/arrdalan.gpg
const DEFAULT_SIGNERS: [(&str, &str); 2] = [("jkaczman", "jkaczman"), ("arrdalan", "arrdalan")];

fn is_bundle_zip_asset(name: &str) -> bool {
    // Avoid the source code (zip), try to target real asset (e.g. "secluso-v0.1.0.zip")
    name.starts_with("secluso-v") && name.ends_with(".zip")
}

const MANIFEST_PATH: &str = "manifest.json";

fn manifest_sig_path_for(label: &str) -> String {
    format!("{}.{}.asc", MANIFEST_PATH, label) // manifest.json.jkaczman-1.asc
}

fn default_signers() -> Vec<Signer> {
    DEFAULT_SIGNERS
        .iter()
        .map(|(label, github_user)| Signer {
            label: (*label).to_string(),
            github_user: (*github_user).to_string(),
        })
        .collect()
}

fn parse_sig_keys(values: &[String]) -> Result<Vec<Signer>> {
    let mut signers = Vec::with_capacity(values.len());
    for raw in values {
        let mut parts = raw.splitn(2, ':');
        let label = parts.next().unwrap_or("").trim();
        let github_user = parts.next().unwrap_or("").trim();
        if label.is_empty() || github_user.is_empty() {
            bail!(
                "Invalid --sig-key value {}. Expected NAME:GITHUB_USER with both parts non-empty.",
                raw
            );
        }
        signers.push(Signer {
            label: label.to_string(),
            github_user: github_user.to_string(),
        });
    }
    Ok(signers)
}

const USAGE: &str = r#"
Secluso updater.

Usage:
  secluso-update --component COMPONENT [--once] [--bundle-path PATH] [--interval-secs N] [--github-timeout-secs N] [--restart-unit UNIT] [--github-repo <OWNER/REPO>] [--sig-key <NAME:GITHUB_USER>]...
  secluso-update (--help | -h)
  secluso-update (--version | -v)

Options:
  --component COMPONENT      Which single binary to update:
                             server | raspberry_camera_hub | config_tool
  --restart-unit UNIT        systemd unit to restart after install (optional).
                             If omitted, no service is restarted.
  --interval-secs N          Poll interval seconds [default: 60].
  --github-timeout-secs N    HTTP timeout seconds [default: 20].
  --github-repo <OWNER/REPO>  GitHub repo to poll for releases [default: secluso/secluso].
  --sig-key <NAME:GITHUB_USER>  Signature label + GitHub user (repeatable).
  --once                     Run a single update check then exit.
  --bundle-path PATH         Use a local bundle zip instead of downloading from GitHub.
  --version, -v              Show tool version.
  --help, -h                 Show this screen.
"#;

#[derive(Debug, Deserialize)]
struct Args {
    flag_component: String,
    flag_restart_unit: Option<String>,
    flag_interval_secs: u64,
    flag_github_timeout_secs: u64,
    flag_github_repo: String,
    flag_sig_key: Vec<String>,
    flag_once: bool,
    flag_bundle_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
    published_at: Option<String>,

    #[serde(default)]
    draft: bool,

    #[serde(default)]
    immutable: bool,
}

#[derive(Debug, Deserialize, Clone)]
struct GhAsset {
    id: u64,
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum Component {
    Server,
    RaspberryCameraHub,
    ConfigTool,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Manifest {
    build: BuildInfo,
    artifacts: Vec<Artifact>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct BuildInfo {
    target: String,
    profile: String,
    run_id: String,
    timestamp: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Artifact {
    package: String,
    target: String,
    bin: String,
    bin_path: String,
    #[serde(rename = "crate")]
    crate_name: String,
    version: String,
    crate_lock_sha256: String,
    rust_digest: String,
    sha256: String,
}

impl Component {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "server" => Ok(Self::Server),
            "raspberry_camera_hub" => Ok(Self::RaspberryCameraHub),
            "config_tool" => Ok(Self::ConfigTool),
            _ => bail!(
                "Unknown component {}. Use one of: server | raspberry_camera_hub | config_tool",
                s
            ),
        }
    }

    /// Path to the binary inside the zip, per arch.
    fn zip_path(self, arch: &str) -> Result<&'static str> {
        match (self, arch) {
            (Self::Server, "x86_64") => Ok("x86_64-unknown-linux-gnu/secluso-server"),
            (Self::Server, "aarch64") => Ok("aarch64-unknown-linux-gnu/secluso-server"),
            (Self::Server, _) => bail!("component=server not supported on arch={}", arch),

            (Self::RaspberryCameraHub, "aarch64") => {
                Ok("aarch64-unknown-linux-gnu/secluso-raspberry-camera-hub")
            }
            (Self::RaspberryCameraHub, _) => {
                bail!(
                    "component=raspberry_camera_hub not supported on arch={}",
                    arch
                )
            }

            (Self::ConfigTool, "x86_64") => Ok("x86_64-unknown-linux-gnu/secluso-config-tool"),
            (Self::ConfigTool, "aarch64") => Ok("aarch64-unknown-linux-gnu/secluso-config-tool"),
            (Self::ConfigTool, _) => bail!("component=config_tool not supported on arch={}", arch),
        }
    }

    /// Where to install on disk (single destination per component).
    fn install_path(self) -> String {
        let bin = match self {
            Self::Server => "secluso-server",
            Self::RaspberryCameraHub => "secluso-raspberry-camera-hub",
            Self::ConfigTool => "secluso-config-tool",
        };

        // Format based on the install root
        format!("{}/bin/{}", INSTALL_ROOT.trim_end_matches('/'), bin)
    }

    /// The version file location which we maintain per-component to identify if updates are necessary
    fn version_file(self) -> String {
        let name = match self {
            Self::Server => "server",
            Self::RaspberryCameraHub => "raspberry_camera_hub",
            Self::ConfigTool => "config_tool",
        };

        format!(
            "{}/current_version/{}",
            INSTALL_ROOT.trim_end_matches('/'),
            name
        )
    }
}

fn main() -> ! {
    let version = format!(
        "{}, version: {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let args: Args = Docopt::new(USAGE)
        .map(|d| d.help(true))
        .map(|d| d.version(Some(version)))
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());

    if args.flag_once {
        println!("Going to check for updates.");
        if let Err(e) = check_update(&args) {
            eprintln!("Update check failed: {:#}", e);
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    loop {
        println!("Going to check for updates.");
        if let Err(e) = check_update(&args) {
            eprintln!("Update check failed: {:#}", e);
        }
        sleep(Duration::from_secs(args.flag_interval_secs));
    }
}

fn check_update(args: &Args) -> Result<()> {
    let component = Component::parse(&args.flag_component)?;
    let cli_signers = parse_sig_keys(&args.flag_sig_key)?;
    let signers = if cli_signers.is_empty() {
        default_signers()
    } else {
        cli_signers
    };

    let current_version = match get_current_version(component) {
        Ok(version) => version,
        Err(_) => Version::parse("0.0.0")?,
    };
    println!("Current Version = {current_version}");

    let github_token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("SECLUSO_GITHUB_TOKEN"))
        .ok()
        .and_then(|v| {
            let trimmed = v.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

    let mut headers = HeaderMap::new();
    if let Some(token) = github_token {
        let value = format!("Bearer {}", token);
        if let Ok(hv) = HeaderValue::from_str(&value) {
            headers.insert(AUTHORIZATION, hv);
        }
    }

    let client = Client::builder()
        .user_agent("secluso-updater")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(args.flag_github_timeout_secs))
        .default_headers(headers)
        .build()?;

    let github_repo = if args.flag_github_repo.trim().is_empty() {
        DEFAULT_OWNER_REPO.to_string()
    } else {
        args.flag_github_repo.clone()
    };

    // Fetch latest release metadata
    let release = fetch_latest_release(&client, &github_repo)?;
    println!("Latest Tag = {}", release.tag_name);
    if let Some(p) = &release.published_at {
        println!("Published At = {}", p);
    }

    let latest_version = Version::parse(release.tag_name.trim_start_matches('v'))?;
    if latest_version <= current_version {
        println!("Already on latest version ({current_version}).");
        return Ok(());
    }
    println!("Found newer version: {latest_version}");

    // Utilize a tokenless immutability check using release.immutable from the latest release
    require_release_is_immutable(&release)?;

    let bundle_path = args
        .flag_bundle_path
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());

    let zip_bytes: Bytes = if let Some(path) = bundle_path {
        println!("Using local bundle: {}", path);
        Bytes::from(fs::read(path).with_context(|| format!("Failed reading bundle at {}", path))?)
    } else {
        // Select bundle asset (zip)
        let bundle = release
            .assets
            .iter()
            .find(|a| is_bundle_zip_asset(&a.name))
            .cloned()
            .ok_or_else(|| anyhow!("could not find bundle zip asset in latest release"))?;

        println!(
            "Bundle Asset: name={} id={} size={} url={}",
            bundle.name, bundle.id, bundle.size, bundle.browser_download_url
        );

        // Require GitHub to provide the sha256 digest for this asset
        let bundle_digest = bundle
            .digest
            .as_deref()
            .ok_or_else(|| anyhow!("github asset {} missing digest field", bundle.name))?;

        // Download zip bytes
        let zip_bytes = fetch_bytes(&client, &bundle.browser_download_url)
            .with_context(|| format!("Failed downloading {}", bundle.name))?;
        println!("Downloaded zip bytes: {}", zip_bytes.len());

        // Verify downloaded bytes match GitHub's recorded sha256 digest for this asset
        require_asset_sha256_digest_matches_download(&bundle.name, bundle_digest, &zip_bytes)?;
        zip_bytes
    };

    // Open zip, read manifest + signature files
    let mut zip =
        ZipArchive::new(Cursor::new(zip_bytes.clone())).context("Failed to parse zip archive")?;

    let manifest_bytes =
        read_zip_file(&mut zip, MANIFEST_PATH).context("Missing manifest.json in bundle")?;
    println!("Read manifest.json ({} bytes)", manifest_bytes.len());

    // Read all signature files (by label)
    let mut sigs: Vec<(Signer, Vec<u8>)> = Vec::with_capacity(signers.len());
    for signer in &signers {
        let sig_path = manifest_sig_path_for(&signer.label);
        let sig_bytes = read_zip_file(&mut zip, &sig_path)
            .with_context(|| format!("Missing signature file in zip: {}", sig_path))?;
        println!(
            "Read sig {} (label={}, github_user={}) ({} bytes)",
            sig_path,
            signer.label,
            signer.github_user,
            sig_bytes.len()
        );
        sigs.push((signer.clone(), sig_bytes));
    }

    // Fetch GitHub keyrings and verify each signature over manifest.json
    let mut key_cache: HashMap<String, (Vec<Cert>, HashSet<Fingerprint>)> = HashMap::new();

    for (signer, sig_bytes) in &sigs {
        let (certs, allowed_fprs) = match key_cache.get(&signer.github_user) {
            Some(v) => v.clone(),
            None => {
                let v = fetch_github_user_keyring(&client, &signer.github_user)?;
                key_cache.insert(signer.github_user.clone(), v.clone());
                v
            }
        };

        verify_manifest_sig_requires_user(
            &manifest_bytes,
            sig_bytes,
            &certs,
            &allowed_fprs,
            &signer.github_user,
            &signer.label,
        )
        .with_context(|| {
            format!(
                "Signature verification failed (label={}, github_user={})",
                signer.label, signer.github_user
            )
        })?;
    }

    println!("All required OpenPGP signatures verified for manifest.json.");

    // Parse signed manifest JSON
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("manifest.json is not valid JSON")?;

    // Sanity check that artifacts claim the same version as the release tag
    let tag_semver = release.tag_name.trim_start_matches('v');
    if manifest
        .artifacts
        .iter()
        .any(|a| a.version.trim() != tag_semver)
    {
        bail!(
            "manifest artifacts contain a version that doesn't match release tag {}",
            release.tag_name
        );
    }

    // Extract a singular target binary bytes from zip
    let arch = std::env::consts::ARCH;
    let target_rel = component.zip_path(arch)?;
    let target_bytes = read_zip_file(&mut zip, target_rel)
        .with_context(|| format!("Missing target binary in zip: {}", target_rel))?;
    println!(
        "Extracted component={} from {} ({} bytes)",
        args.flag_component,
        target_rel,
        target_bytes.len()
    );

    // Verify sha256 of extracted binary against signed manifest
    let art = manifest
        .artifacts
        .iter()
        .find(|a| a.bin_path == target_rel)
        .ok_or_else(|| anyhow!("manifest missing artifact entry for {}", target_rel))?;

    let expected = normalize_hex(&art.sha256);
    let got = sha256_hex(&target_bytes);

    if expected != normalize_hex(&got) {
        bail!(
            "sha256 mismatch for {}: expected={}, got={}",
            target_rel,
            art.sha256,
            got
        );
    }

    println!("OK... sha256 verified for {}", target_rel);

    // Install + estart
    let tmp_path = "/tmp/secluso-binary-tmp";
    let final_path = component.install_path();

    let final_dir = Path::new(&final_path)
        .parent()
        .ok_or_else(|| anyhow!("invalid install path: {}", final_path))?;

    fs::create_dir_all(final_dir)
        .with_context(|| format!("Failed to create install dir {}", final_dir.display()))?;

    fs::write(tmp_path, &target_bytes)?;
    fs::set_permissions(tmp_path, fs::Permissions::from_mode(0o755))?;

    if let Some(unit) = args.flag_restart_unit.as_deref() {
        println!("Stopping unit: {}", unit);
        run(&format!("systemctl stop {}", shell_escape(unit)));
    }

    println!("Installing: {} -> {}", tmp_path, final_path);
    fs::rename(tmp_path, &final_path)?;

    if let Some(unit) = args.flag_restart_unit.as_deref() {
        println!("Starting unit: {}", unit);
        run(&format!("systemctl start {}", shell_escape(unit)));
    }

    write_current_version(component, latest_version.clone())?;

    println!(
        "Updated to version {latest_version} (component={})",
        args.flag_component
    );
    Ok(())
}

fn require_release_is_immutable(release: &GhRelease) -> Result<()> {
    if release.draft {
        bail!(
            "Refusing update: latest release {} is a draft.",
            release.tag_name
        );
    }
    if release.published_at.is_none() {
        bail!(
            "Refusing update: latest release {} is not published (missing published_at).",
            release.tag_name
        );
    }
    if !release.immutable {
        bail!(
            "Refusing update: latest release {} is not marked immutable by GitHub (immutable=false).",
            release.tag_name
        );
    }
    Ok(())
}

fn require_asset_sha256_digest_matches_download(
    asset_name: &str,
    asset_digest: &str,
    downloaded_bytes: &[u8],
) -> Result<()> {
    let expected = normalize_hex(asset_digest);

    // We only accept GitHub's sha256 digests here.
    if !asset_digest
        .trim()
        .to_ascii_lowercase()
        .starts_with("sha256:")
    {
        bail!(
            "Refusing update: asset {} has unsupported digest format {}",
            asset_name,
            asset_digest
        );
    }

    let got = sha256_hex(downloaded_bytes);

    if expected != got {
        bail!(
            "Refusing update: GitHub asset digest mismatch for {}: expected={}, got=sha256:{}",
            asset_name,
            asset_digest,
            got
        );
    }

    Ok(())
}

fn fetch_latest_release(client: &Client, owner_repo: &str) -> Result<GhRelease> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        owner_repo
    );
    let resp = client.get(&url).send()?.error_for_status()?;
    Ok(resp.json::<GhRelease>()?)
}
fn fetch_bytes(client: &Client, url: &str) -> Result<Bytes> {
    Ok(client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()?
        .error_for_status()?
        .bytes()?)
}

fn zip_root_prefix(zip: &mut ZipArchive<Cursor<Bytes>>) -> Option<String> {
    let mut prefix: Option<String> = None;
    for i in 0..zip.len() {
        let name = match zip.by_index(i) {
            Ok(f) => f.name().to_string(),
            Err(_) => continue,
        };
        let mut parts = name.splitn(2, '/');
        let top = parts.next().unwrap_or("");
        let rest = parts.next();
        if rest.is_none() {
            return None;
        }
        if top.is_empty() {
            return None;
        }
        match &prefix {
            None => prefix = Some(top.to_string()),
            Some(existing) if existing != top => return None,
            _ => {}
        }
    }
    prefix.map(|p| format!("{}/", p))
}

fn read_zip_file(zip: &mut ZipArchive<Cursor<Bytes>>, path: &str) -> Result<Vec<u8>> {
    match zip.by_name(path) {
        Ok(mut f) => {
            let mut buf = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut buf)?;
            return Ok(buf);
        }
        Err(_) => {}
    }

    let prefix = zip_root_prefix(zip).ok_or_else(|| anyhow!("zip missing entry {}", path))?;
    let alt = format!("{}{}", prefix, path);
    let mut f = zip
        .by_name(&alt)
        .with_context(|| format!("zip missing entry {} (also tried {})", path, alt))?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

// Fetch the current version from the component within INSTALL_ROOT
fn get_current_version(component: Component) -> Result<Version> {
    let p = component.version_file();
    let s =
        fs::read_to_string(&p).with_context(|| format!("reading current version file: {}", p))?;

    Ok(Version::parse(s.trim().trim_start_matches('v'))?)
}

// Write the current version from the component within INSTALL_ROOT
fn write_current_version(component: Component, v: Version) -> Result<()> {
    let p = component.version_file();

    if let Some(parent) = Path::new(&p).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating version dir: {}", parent.display()))?;
    }

    fs::write(&p, format!("v{}\n", v))
        .with_context(|| format!("writing current version file: {}", p))?;

    Ok(())
}

fn run(cmd: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '@'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r#"'\''"#))
    }
}

fn fetch_github_user_keyring(
    client: &Client,
    user: &str,
) -> Result<(Vec<Cert>, HashSet<Fingerprint>)> {
    let url = format!("https://github.com/{user}.gpg");
    println!("Fetching GPG keys for {user} from {url}");
    let body = client.get(&url).send()?.error_for_status()?.bytes()?;

    let mut certs = Vec::new();
    let mut fps = HashSet::new();

    let mut parser = openpgp::cert::CertParser::from_bytes(&body)?;
    while let Some(cert) = parser.next().transpose()? {
        for ka in cert.keys() {
            fps.insert(ka.key().fingerprint());
        }
        certs.push(cert);
    }

    if certs.is_empty() {
        bail!("No OpenPGP certs found at {}", url);
    }

    Ok((certs, fps))
}

struct Helper {
    certs: Vec<Cert>,
    signer_fprs: Vec<Fingerprint>,
}

impl VerificationHelper for Helper {
    fn get_certs(&mut self, _ids: &[KeyHandle]) -> openpgp::Result<Vec<Cert>> {
        Ok(self.certs.clone())
    }

    fn check(&mut self, structure: MessageStructure) -> openpgp::Result<()> {
        for layer in structure.iter() {
            if let MessageLayer::SignatureGroup { results } = layer {
                for r in results {
                    if let Ok(GoodChecksum { ka, .. }) = r {
                        self.signer_fprs.push(ka.key().fingerprint());
                    }
                }
            }
        }
        Ok(())
    }
}

/// Verify a detached signature over manifest, requiring that the signer’s fingerprint
/// matches some key published on https://github.com/<github_user>.gpg
fn verify_manifest_sig_requires_user(
    manifest: &[u8],
    sig: &[u8],
    certs: &[Cert],
    allowed_fprs: &HashSet<Fingerprint>,
    github_user: &str,
    label: &str,
) -> Result<()> {
    let policy = &StandardPolicy::new();

    let helper = Helper {
        certs: certs.to_vec(),
        signer_fprs: Vec::new(),
    };

    let mut v = DetachedVerifierBuilder::from_bytes(sig)
        .context("Parsing detached signature failed")?
        .with_policy(policy, None, helper)
        .context("Building verifier failed")?;

    v.verify_bytes(manifest)
        .context("Feeding manifest into verifier failed")?;

    let helper = v.into_helper();

    if helper.signer_fprs.is_empty() {
        bail!(
            "Signature verified but no signer fingerprint reported (github_user={}, label={})",
            github_user,
            label
        );
    }

    if helper.signer_fprs.iter().any(|f| allowed_fprs.contains(f)) {
        println!(
            "Verified manifest signature (label={}, github_user={})",
            label, github_user
        );
        Ok(())
    } else {
        bail!(
            "Signature verified, but signer fingerprint did not match {}'s GitHub keys (label={})",
            github_user,
            label
        );
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();

    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        use std::fmt::Write;
        write!(&mut s, "{:02x}", b).unwrap();
    }
    s
}

fn normalize_hex(s: &str) -> String {
    s.trim().trim_start_matches("sha256:").to_ascii_lowercase()
}
