//! UnifiedPush / Web Push delivery helpers.
//!
//! SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use reqwest::Url;
use secluso_server_backbone::types::NotificationTarget;
use std::env;
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, WebPushClient, WebPushMessageBuilder,
};

pub const UNIFIEDPUSH_ALLOWED_HOSTS_ENV: &str = "SECLUSO_UNIFIEDPUSH_ALLOWED_HOSTS";

// Hosted distributors we intentionally support out of the box. Self-hosted
// backends must be added explicitly via SECLUSO_UNIFIEDPUSH_ALLOWED_HOSTS.
const DEFAULT_ALLOWED_HOSTS: &[&str] = &[
    "ntfy.sh",
    "gotify1.unifiedpush.org",
    "push.services.mozilla.com",
    "updates.push.services.mozilla.com",
    "up.conversations.im",
    "fcm.googleapis.com",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct AllowedEndpoint {
    host: String,
    port: Option<u16>,
}

#[derive(Debug, Clone, Default)]
pub struct UnifiedPushPolicy {
    allowed_endpoints: Vec<AllowedEndpoint>,
}

impl UnifiedPushPolicy {
    pub fn from_env() -> Result<Self> {
        let mut policy = Self::with_default_allowlist()?;
        let raw = env::var(UNIFIEDPUSH_ALLOWED_HOSTS_ENV).unwrap_or_default();
        policy.extend_from_allowlist_csv(&raw)?;
        Ok(policy)
    }

    fn from_allowlist_csv(raw: &str) -> Result<Self> {
        let mut policy = Self::default();
        policy.extend_from_allowlist_csv(raw)?;
        Ok(policy)
    }

    fn with_default_allowlist() -> Result<Self> {
        let mut policy = Self::default();
        policy.extend_from_allowlist_csv(&DEFAULT_ALLOWED_HOSTS.join(","))?;
        Ok(policy)
    }

    fn extend_from_allowlist_csv(&mut self, raw: &str) -> Result<()> {
        for item in raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let endpoint = parse_allowed_endpoint(item)?;
            if !self.allowed_endpoints.contains(&endpoint) {
                self.allowed_endpoints.push(endpoint);
            }
        }
        Ok(())
    }

    fn validate_endpoint_url(&self, raw_url: &str) -> Result<Url> {
        let parsed = Url::parse(raw_url)
            .with_context(|| format!("Invalid UnifiedPush endpoint URL: {raw_url}"))?;
        if parsed.scheme() != "https" {
            anyhow::bail!("Refusing non-HTTPS UnifiedPush endpoint URL: {raw_url}");
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            anyhow::bail!("UnifiedPush endpoint URL must not include credentials: {raw_url}");
        }
        if parsed.fragment().is_some() {
            anyhow::bail!("UnifiedPush endpoint URL must not include a fragment: {raw_url}");
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| {
                anyhow::anyhow!("UnifiedPush endpoint URL is missing a host: {raw_url}")
            })?
            .to_ascii_lowercase();
        let port = parsed.port_or_known_default().ok_or_else(|| {
            anyhow::anyhow!("UnifiedPush endpoint URL is missing an HTTPS port: {raw_url}")
        })?;

        let allowed = self.allowed_endpoints.iter().any(|candidate| {
            candidate.host == host
                && match candidate.port {
                    Some(allowed_port) => allowed_port == port,
                    None => port == 443,
                }
        });
        if !allowed {
            anyhow::bail!(
                "Refusing UnifiedPush endpoint host '{host}:{port}'. It is not in the built-in distributor allowlist and was not added via {UNIFIEDPUSH_ALLOWED_HOSTS_ENV}."
            );
        }

        Ok(parsed)
    }
}

fn parse_allowed_endpoint(raw: &str) -> Result<AllowedEndpoint> {
    if raw.contains("://") {
        anyhow::bail!(
            "Invalid {UNIFIEDPUSH_ALLOWED_HOSTS_ENV} entry '{raw}'. Use host or host:port, not a full URL."
        );
    }

    let parsed = Url::parse(&format!("https://{raw}"))
        .with_context(|| format!("Invalid {UNIFIEDPUSH_ALLOWED_HOSTS_ENV} entry '{raw}'"))?;
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        anyhow::bail!(
            "Invalid {UNIFIEDPUSH_ALLOWED_HOSTS_ENV} entry '{raw}'. Only host or host:port is allowed."
        );
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!(
            "Invalid {UNIFIEDPUSH_ALLOWED_HOSTS_ENV} entry '{raw}'. Credentials are not allowed."
        );
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid {UNIFIEDPUSH_ALLOWED_HOSTS_ENV} entry '{raw}'"))?
        .to_ascii_lowercase();

    Ok(AllowedEndpoint {
        host,
        port: parsed.port(),
    })
}

pub fn validate_notification_target(
    policy: &UnifiedPushPolicy,
    target: &NotificationTarget,
) -> Result<()> {
    if !target.platform.eq_ignore_ascii_case("android_unified") {
        return Ok(());
    }

    let endpoint_url = target
        .unifiedpush_endpoint_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("UnifiedPush endpoint URL is required."))?;
    let pub_key = target
        .unifiedpush_pub_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("UnifiedPush public key is required."))?;
    let auth = target
        .unifiedpush_auth
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("UnifiedPush auth secret is required."))?;

    policy.validate_endpoint_url(endpoint_url)?;

    if pub_key.is_empty() || auth.is_empty() {
        anyhow::bail!("UnifiedPush key material is incomplete.");
    }

    Ok(())
}

pub async fn send_notification(
    policy: &UnifiedPushPolicy,
    endpoint_url: &str,
    pub_key: &str,
    auth: &str,
    payload: &[u8],
) -> Result<()> {
    let endpoint_url = policy.validate_endpoint_url(endpoint_url)?.to_string();
    let subscription_info =
        SubscriptionInfo::new(endpoint_url, pub_key.to_string(), auth.to_string());

    let mut builder = WebPushMessageBuilder::new(&subscription_info);
    builder.set_payload(ContentEncoding::Aes128Gcm, payload);
    builder.set_ttl(60);

    let client = IsahcWebPushClient::new()?;
    client.send(builder.build()?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_notification_target, UnifiedPushPolicy};
    use secluso_server_backbone::types::NotificationTarget;

    fn android_target(url: &str) -> NotificationTarget {
        NotificationTarget {
            platform: "android_unified".to_string(),
            ios_relay_binding: None,
            unifiedpush_endpoint_url: Some(url.to_string()),
            unifiedpush_pub_key: Some("pub".to_string()),
            unifiedpush_auth: Some("auth".to_string()),
        }
    }

    #[test]
    fn rejects_endpoints_when_allowlist_is_missing() {
        let policy = UnifiedPushPolicy::default();
        let err =
            validate_notification_target(&policy, &android_target("https://up.example.com/push"))
                .unwrap_err()
                .to_string();
        assert!(err.contains("built-in distributor allowlist"));
    }

    #[test]
    fn accepts_allowlisted_default_https_host() {
        let policy = UnifiedPushPolicy::from_allowlist_csv("up.example.com").unwrap();
        validate_notification_target(
            &policy,
            &android_target("https://up.example.com/push/token"),
        )
        .unwrap();
    }

    #[test]
    fn rejects_unlisted_host() {
        let policy = UnifiedPushPolicy::from_allowlist_csv("up.example.com").unwrap();
        let err =
            validate_notification_target(&policy, &android_target("https://evil.example.net/push"))
                .unwrap_err()
                .to_string();
        assert!(err.contains("allowlist"));
    }

    #[test]
    fn rejects_non_default_port_unless_explicitly_allowlisted() {
        let policy = UnifiedPushPolicy::from_allowlist_csv("up.example.com").unwrap();
        let err = validate_notification_target(
            &policy,
            &android_target("https://up.example.com:8443/push"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("allowlist"));
    }

    #[test]
    fn accepts_explicit_allowlisted_port() {
        let policy = UnifiedPushPolicy::from_allowlist_csv("up.example.com:8443").unwrap();
        validate_notification_target(&policy, &android_target("https://up.example.com:8443/push"))
            .unwrap();
    }

    #[test]
    fn rejects_allowlist_entries_that_are_urls() {
        let err = UnifiedPushPolicy::from_allowlist_csv("https://up.example.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("host or host:port"));
    }

    #[test]
    fn built_in_ntfy_host_is_allowed() {
        let policy = UnifiedPushPolicy::with_default_allowlist().unwrap();
        validate_notification_target(&policy, &android_target("https://ntfy.sh/example-topic"))
            .unwrap();
    }

    #[test]
    fn built_in_gotify_host_is_allowed() {
        let policy = UnifiedPushPolicy::with_default_allowlist().unwrap();
        validate_notification_target(
            &policy,
            &android_target("https://gotify1.unifiedpush.org/message/example"),
        )
        .unwrap();
    }

    #[test]
    fn built_in_autopush_host_is_allowed() {
        let policy = UnifiedPushPolicy::with_default_allowlist().unwrap();
        validate_notification_target(
            &policy,
            &android_target("https://updates.push.services.mozilla.com/wpush/v1/example"),
        )
        .unwrap();
    }

    #[test]
    fn built_in_push_services_host_is_allowed() {
        let policy = UnifiedPushPolicy::with_default_allowlist().unwrap();
        validate_notification_target(
            &policy,
            &android_target("https://push.services.mozilla.com/wpush/v1/example"),
        )
        .unwrap();
    }

    #[test]
    fn built_in_conversations_proxy_host_is_allowed() {
        let policy = UnifiedPushPolicy::with_default_allowlist().unwrap();
        validate_notification_target(
            &policy,
            &android_target("https://up.conversations.im/push/example"),
        )
        .unwrap();
    }

    #[test]
    fn built_in_google_fcm_host_is_allowed() {
        let policy = UnifiedPushPolicy::with_default_allowlist().unwrap();
        validate_notification_target(
            &policy,
            &android_target("https://fcm.googleapis.com/fcm/send/example"),
        )
        .unwrap();
    }
}
