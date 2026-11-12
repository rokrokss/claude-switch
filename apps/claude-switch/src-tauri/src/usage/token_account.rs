//! Whether a stored sign-in belongs to the profile reading it.
//!
//! A Keychain item holds whichever account last signed in through it: a CLI
//! run without `CLAUDE_CONFIG_DIR` overwrites the stock install's, whoever
//! that install is meant to be. The usage endpoint answers for any valid token
//! and doesn't say whose it is, so quota is only shown once the profile
//! endpoint has named the token's account and it's one of the profile's own.
//!
//! This fails open: when the endpoint can't answer or the profile's files
//! can't say who it should be, usage shows as it would have. A flaky
//! undocumented endpoint shouldn't hide correct numbers.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::Deserialize;

use crate::usage::credentials::read_access_token;
use crate::usage::quota::{token_cache_key, HttpResponse, ReqwestUsageClient};
use crate::usage::{ProfileUsage, QuotaError};

const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";

/// The account a token belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenAccount {
    uuid: String,
    email: Option<String>,
}

#[derive(Deserialize)]
struct ProfileResponse {
    account: Option<ProfileResponseAccount>,
}

#[derive(Deserialize)]
struct ProfileResponseAccount {
    uuid: Option<String>,
    email: Option<String>,
}

/// Pure: the account a profile response names, if it was a success that does.
fn parse_token_account(response: &HttpResponse) -> Option<TokenAccount> {
    if response.status != 200 {
        return None;
    }
    let account = serde_json::from_slice::<ProfileResponse>(&response.body)
        .ok()?
        .account?;
    let uuid = account.uuid.filter(|uuid| !uuid.trim().is_empty())?;
    Some(TokenAccount {
        uuid,
        email: account.email.filter(|email| !email.trim().is_empty()),
    })
}

#[async_trait]
pub trait TokenAccountClient: Send + Sync {
    /// The account `access_token` belongs to; `None` when that can't be found out.
    async fn fetch_account(&self, access_token: &str) -> Option<TokenAccount>;
}

#[async_trait]
impl TokenAccountClient for ReqwestUsageClient {
    async fn fetch_account(&self, access_token: &str) -> Option<TokenAccount> {
        let response = self.get(PROFILE_URL, access_token).await.ok()?;
        parse_token_account(&response)
    }
}

/// Accounts already resolved, by token hash. A token's account never changes
/// and a renewed token is a new key, so entries don't expire. Failures aren't
/// kept: the next poll asks again.
#[derive(Default)]
pub struct TokenAccountCache {
    accounts: Mutex<HashMap<String, TokenAccount>>,
}

impl TokenAccountCache {
    pub fn new() -> Self {
        Self::default()
    }

    async fn account(&self, token: &str, client: &dyn TokenAccountClient) -> Option<TokenAccount> {
        let key = token_cache_key(token);
        if let Some(account) = self.accounts.lock().unwrap().get(&key) {
            return Some(account.clone());
        }
        let account = client.fetch_account(token).await?;
        self.accounts.lock().unwrap().insert(key, account.clone());
        Some(account)
    }
}

/// Withholds `usage` unless the sign-in stored for `cli_config_dir` belongs to
/// one of `account_ids`, the accounts the profile is signed in under
/// ([`crate::accounts::claude_account_ids`]).
///
/// Runs after the fetch and any renewal it needed, so the token checked is the
/// one the numbers came from.
pub async fn guard(
    usage: ProfileUsage,
    cli_config_dir: &Path,
    account_ids: &[String],
    client: &dyn TokenAccountClient,
    cache: &TokenAccountCache,
) -> ProfileUsage {
    if usage.quota.is_none() || account_ids.is_empty() {
        return usage;
    }
    let Ok(token) = read_access_token(cli_config_dir) else {
        return usage;
    };
    let Some(account) = cache.account(&token, client).await else {
        return usage;
    };
    if belongs_to(&account, account_ids) {
        return usage;
    }
    ProfileUsage {
        quota: None,
        quota_error: Some(QuotaError::AccountMismatch),
        stored_account_email: account.email,
        fetched_at: usage.fetched_at,
    }
}

/// Pure: whether `account` is one of `account_ids`.
fn belongs_to(account: &TokenAccount, account_ids: &[String]) -> bool {
    account_ids
        .iter()
        .any(|id| id.trim().eq_ignore_ascii_case(account.uuid.trim()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use tempfile::TempDir;

    use super::*;
    use crate::usage::QuotaUsage;

    struct StubClient {
        account: Option<TokenAccount>,
        calls: AtomicU32,
    }

    impl StubClient {
        fn naming(uuid: &str) -> Self {
            Self {
                account: Some(TokenAccount {
                    uuid: uuid.to_string(),
                    email: Some("work@example.com".to_string()),
                }),
                calls: AtomicU32::new(0),
            }
        }

        fn unreachable() -> Self {
            Self {
                account: None,
                calls: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl TokenAccountClient for StubClient {
        async fn fetch_account(&self, _: &str) -> Option<TokenAccount> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.account.clone()
        }
    }

    fn usage_with_quota() -> ProfileUsage {
        ProfileUsage {
            quota: Some(QuotaUsage::default()),
            quota_error: None,
            stored_account_email: None,
            fetched_at: "now".to_string(),
        }
    }

    fn signed_in_as(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn config_dir_with_token(token: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(".credentials.json"),
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}"}}}}"#),
        )
        .unwrap();
        dir
    }

    #[tokio::test]
    async fn another_accounts_token_withholds_the_quota_and_names_that_account() {
        let dir = config_dir_with_token("t");

        let guarded = guard(
            usage_with_quota(),
            dir.path(),
            &signed_in_as(&["default-account"]),
            &StubClient::naming("work-account"),
            &TokenAccountCache::new(),
        )
        .await;

        assert!(guarded.quota.is_none());
        assert!(matches!(
            guarded.quota_error,
            Some(QuotaError::AccountMismatch)
        ));
        assert_eq!(
            guarded.stored_account_email.as_deref(),
            Some("work@example.com")
        );
        assert_eq!(guarded.fetched_at, "now");
    }

    #[tokio::test]
    async fn a_token_of_any_of_the_profiles_accounts_keeps_the_quota() {
        let dir = config_dir_with_token("t");

        let guarded = guard(
            usage_with_quota(),
            dir.path(),
            &signed_in_as(&["desktop-account", "CLI-ACCOUNT"]),
            &StubClient::naming("cli-account"),
            &TokenAccountCache::new(),
        )
        .await;

        assert!(guarded.quota.is_some());
        assert!(guarded.quota_error.is_none());
    }

    #[tokio::test]
    async fn an_endpoint_that_cant_answer_keeps_the_quota() {
        let dir = config_dir_with_token("t");

        let guarded = guard(
            usage_with_quota(),
            dir.path(),
            &signed_in_as(&["default-account"]),
            &StubClient::unreachable(),
            &TokenAccountCache::new(),
        )
        .await;

        assert!(guarded.quota.is_some());
    }

    #[tokio::test]
    async fn a_profile_that_cant_say_who_it_is_keeps_the_quota_without_asking() {
        let dir = config_dir_with_token("t");
        let client = StubClient::naming("work-account");

        let guarded = guard(
            usage_with_quota(),
            dir.path(),
            &signed_in_as(&[]),
            &client,
            &TokenAccountCache::new(),
        )
        .await;

        assert!(guarded.quota.is_some());
        assert_eq!(client.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn usage_without_a_quota_is_left_as_it_is_without_asking() {
        let dir = config_dir_with_token("t");
        let client = StubClient::naming("work-account");
        let usage = ProfileUsage {
            quota: None,
            quota_error: Some(QuotaError::RateLimited),
            stored_account_email: None,
            fetched_at: "now".to_string(),
        };

        let guarded = guard(
            usage,
            dir.path(),
            &signed_in_as(&["default-account"]),
            &client,
            &TokenAccountCache::new(),
        )
        .await;

        assert!(matches!(guarded.quota_error, Some(QuotaError::RateLimited)));
        assert_eq!(client.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_token_is_only_resolved_once_until_it_is_renewed() {
        let dir = config_dir_with_token("first");
        let client = StubClient::naming("default-account");
        let cache = TokenAccountCache::new();
        let sign_in = signed_in_as(&["default-account"]);

        for _ in 0..2 {
            guard(usage_with_quota(), dir.path(), &sign_in, &client, &cache).await;
        }
        assert_eq!(client.calls.load(Ordering::SeqCst), 1);

        let renewed = config_dir_with_token("second");
        guard(
            usage_with_quota(),
            renewed.path(),
            &sign_in,
            &client,
            &cache,
        )
        .await;
        assert_eq!(client.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_lookup_is_asked_again_next_time() {
        let dir = config_dir_with_token("t");
        let client = StubClient::unreachable();
        let cache = TokenAccountCache::new();
        let sign_in = signed_in_as(&["default-account"]);

        for _ in 0..2 {
            guard(usage_with_quota(), dir.path(), &sign_in, &client, &cache).await;
        }

        assert_eq!(client.calls.load(Ordering::SeqCst), 2);
    }

    // --- manual end-to-end smoke (real endpoint + keychain) ---
    // Gated behind an env var pointing at a profile's cli-config dir, since it
    // reads the real token and calls the real endpoint. Prints no token, id or
    // email. `CLAUDE_SWITCH_ACCOUNT_SMOKE_JSON` names the `.claude.json` to
    // compare with, for the stock install, whose file isn't in its config dir.
    // Run with:
    //   CLAUDE_SWITCH_ACCOUNT_SMOKE="<cli-config dir>" cargo test \
    //     --manifest-path apps/claude-switch/src-tauri/Cargo.toml \
    //     usage::token_account::tests::profile_endpoint_smoke -- --nocapture --ignored
    #[tokio::test]
    #[ignore = "calls the real endpoint; opt in via CLAUDE_SWITCH_ACCOUNT_SMOKE"]
    async fn profile_endpoint_smoke() {
        let Ok(dir) = std::env::var("CLAUDE_SWITCH_ACCOUNT_SMOKE") else {
            eprintln!("set CLAUDE_SWITCH_ACCOUNT_SMOKE=<cli-config dir> to run");
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let json = std::env::var("CLAUDE_SWITCH_ACCOUNT_SMOKE_JSON")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| dir.join(".claude.json"));
        let token = read_access_token(&dir).expect("a stored token");
        let client = ReqwestUsageClient::new(crate::usage::CLAUDE_USER_AGENT.to_owned()).unwrap();
        let response = client.get(PROFILE_URL, &token).await.unwrap();
        eprintln!("profile_endpoint_smoke: status={}", response.status);
        let account = parse_token_account(&response).expect("a named account");
        let named = std::fs::read_to_string(json).unwrap_or_default();
        eprintln!(
            "profile_endpoint_smoke: has_email={} matches_claude_json={}",
            account.email.is_some(),
            named.contains(&account.uuid),
        );
    }

    #[test]
    fn parses_the_account_a_profile_response_names() {
        let response = HttpResponse {
            status: 200,
            body: br#"{"account":{"uuid":"abc","email":"a@example.com","full_name":"A"},"organization":{"uuid":"org"}}"#.to_vec(),
            retry_after: None,
        };
        assert_eq!(
            parse_token_account(&response),
            Some(TokenAccount {
                uuid: "abc".to_string(),
                email: Some("a@example.com".to_string()),
            })
        );
    }

    #[test]
    fn a_failed_or_nameless_profile_response_names_no_one() {
        let response = |status, body: &[u8]| HttpResponse {
            status,
            body: body.to_vec(),
            retry_after: None,
        };
        assert_eq!(
            parse_token_account(&response(401, br#"{"account":{"uuid":"abc"}}"#)),
            None
        );
        assert_eq!(parse_token_account(&response(200, b"not json")), None);
        assert_eq!(
            parse_token_account(&response(200, br#"{"account":{"uuid":" "}}"#)),
            None
        );
        assert_eq!(parse_token_account(&response(200, b"{}")), None);
    }
}
