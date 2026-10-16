//! Renews a Claude profile's sign-in the way Claude Code does: its refresh
//! token exchanged at Anthropic's OAuth endpoint, the rotated tokens saved
//! back where Claude Code keeps them. Usage then keeps coming while nobody
//! runs `claude`. The endpoint, client id and scopes are Claude Code's own
//! and undocumented; OpenUsage renews the same way (MIT).
//!
//! The profile's `claude` and Claude Desktop rotate the same refresh token,
//! so nothing here trusts an earlier read of it:
//! - renewals of one profile run one at a time, and each re-reads the stored
//!   sign-in first;
//! - a rotation is saved only over the refresh token it replaced, merged into
//!   the document as it is then (Claude Code keeps MCP logins beside it);
//! - a refresh token the endpoint rejects counts as dead only if nobody has
//!   rotated it meanwhile.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex as TokioMutex;

use super::credentials::{self, CredentialSource, StoredCredentials};
use super::QuotaError;

const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const SCOPES: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
/// A sign-in this close to lapsing is renewed before it is used.
const RENEW_AHEAD_MS: i64 = 5 * 60 * 1000;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(15);

/// The OAuth login inside the stored document (`claudeAiOauth`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthLogin {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    /// Milliseconds since the epoch.
    pub expires_at: Option<f64>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    claude_ai_oauth: Option<OAuthLogin>,
}

/// The login in a stored document, if it has one.
pub fn login(document: &str) -> Option<OAuthLogin> {
    serde_json::from_str::<Document>(document)
        .ok()?
        .claude_ai_oauth
}

/// What the token endpoint hands back.
#[derive(Debug, Clone, PartialEq)]
pub struct Rotated {
    pub access_token: String,
    /// Absent when the endpoint keeps the old one.
    pub refresh_token: Option<String>,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExchangeError {
    /// The refresh token was refused: expired, revoked or already used.
    Rejected,
    /// Anything that may pass: the network, a server error, a garbled answer.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renewal {
    /// The stored sign-in is now fresh: renewed here or by someone else.
    Renewed,
    /// It wasn't due, or there is nothing to renew it with.
    Unchanged,
    /// The refresh token is dead: the profile has to sign in again.
    Dead,
    /// It couldn't be renewed this time.
    Failed,
}

#[async_trait]
pub trait TokenEndpoint: Send + Sync {
    async fn exchange(&self, refresh_token: &str) -> Result<Rotated, ExchangeError>;
}

pub trait CredentialStore: Send + Sync {
    fn read(&self, cli_config_dir: &Path) -> Result<StoredCredentials, QuotaError>;
    fn write(&self, source: &CredentialSource, document: &str) -> Result<(), String>;
}

/// The profile's real store: its credentials file or Keychain item.
pub struct SystemStore;

impl CredentialStore for SystemStore {
    fn read(&self, cli_config_dir: &Path) -> Result<StoredCredentials, QuotaError> {
        credentials::read_stored(cli_config_dir)
    }

    fn write(&self, source: &CredentialSource, document: &str) -> Result<(), String> {
        credentials::write_stored(source, document)
    }
}

/// Anthropic's token endpoint, as Claude Code calls it.
pub struct AnthropicEndpoint {
    client: reqwest::Client,
}

impl AnthropicEndpoint {
    pub fn new(user_agent: &str) -> Option<Self> {
        let client = crate::usage::proxy::client_builder()
            .user_agent(user_agent)
            .timeout(EXCHANGE_TIMEOUT)
            .build()
            .ok()?;
        Some(Self { client })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<f64>,
}

#[derive(Deserialize)]
struct TokenErrorResponse {
    error: Option<String>,
}

#[async_trait]
impl TokenEndpoint for AnthropicEndpoint {
    async fn exchange(&self, refresh_token: &str) -> Result<Rotated, ExchangeError> {
        let body = serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
            "scope": SCOPES,
        });
        let response = self
            .client
            .post(TOKEN_URL)
            .json(&body)
            .send()
            .await
            .map_err(|_| ExchangeError::Failed)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|_| ExchangeError::Failed)?;
        if status.is_success() {
            return rotated_from(&bytes, Utc::now().timestamp_millis());
        }
        Err(rejection(status.as_u16(), &bytes))
    }
}

/// Pure: the rotation in a successful answer received at `now_ms`.
fn rotated_from(body: &[u8], now_ms: i64) -> Result<Rotated, ExchangeError> {
    let parsed: TokenResponse = serde_json::from_slice(body).map_err(|_| ExchangeError::Failed)?;
    if parsed.access_token.trim().is_empty() {
        return Err(ExchangeError::Failed);
    }
    Ok(Rotated {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token.filter(|token| !token.is_empty()),
        expires_at_ms: parsed
            .expires_in
            .filter(|secs| secs.is_finite() && *secs > 0.0)
            .map(|secs| now_ms + (secs * 1000.0) as i64),
    })
}

/// Pure: whether a failed answer refused the refresh token (OAuth's
/// `invalid_grant`) or failed for a reason that may pass.
fn rejection(status: u16, body: &[u8]) -> ExchangeError {
    let error = serde_json::from_slice::<TokenErrorResponse>(body)
        .ok()
        .and_then(|parsed| parsed.error);
    match (status, error.as_deref()) {
        (400 | 401, Some("invalid_grant")) => ExchangeError::Rejected,
        _ => ExchangeError::Failed,
    }
}

/// Pure: whether `login` lapses within five minutes of `now_ms`. A login
/// without an expiry is never renewed ahead of time.
pub fn due(login: &OAuthLogin, now_ms: i64) -> bool {
    login
        .expires_at
        .is_some_and(|expires_at| expires_at - (now_ms as f64) <= RENEW_AHEAD_MS as f64)
}

/// Pure: `document` with its login's tokens and expiry replaced by
/// `rotated`, every other field left as it was.
fn merged(document: &str, rotated: &Rotated) -> Option<String> {
    let mut value: Value = serde_json::from_str(document).ok()?;
    let login = value.get_mut("claudeAiOauth")?.as_object_mut()?;
    login.insert(
        "accessToken".into(),
        Value::String(rotated.access_token.clone()),
    );
    if let Some(refresh_token) = &rotated.refresh_token {
        login.insert("refreshToken".into(), Value::String(refresh_token.clone()));
    }
    if let Some(expires_at) = rotated.expires_at_ms {
        login.insert("expiresAt".into(), Value::from(expires_at));
    }
    serde_json::to_string(&value).ok()
}

/// One lock per profile, so one renewal at a time can spend its refresh token.
fn lock_for(cli_config_dir: &Path) -> Arc<TokioMutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<TokioMutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(cli_config_dir.to_path_buf())
        .or_default()
        .clone()
}

/// Renews the sign-in at `cli_config_dir` with Anthropic's endpoint. See
/// [`renew_with`].
pub async fn renew(cli_config_dir: &Path, rejected_token: Option<&str>) -> Renewal {
    let user_agent = format!("claude-switch/{}", env!("CARGO_PKG_VERSION"));
    let Some(endpoint) = AnthropicEndpoint::new(&user_agent) else {
        return Renewal::Failed;
    };
    renew_with(&endpoint, &SystemStore, cli_config_dir, rejected_token).await
}

/// Renews the sign-in at `cli_config_dir` when it is due, or, given the
/// access token the usage endpoint just refused (`rejected_token`), unless
/// it has been replaced since.
pub async fn renew_with(
    endpoint: &dyn TokenEndpoint,
    store: &dyn CredentialStore,
    cli_config_dir: &Path,
    rejected_token: Option<&str>,
) -> Renewal {
    let lock = lock_for(cli_config_dir);
    let _renewing = lock.lock().await;
    let Ok(stored) = store.read(cli_config_dir) else {
        return Renewal::Unchanged;
    };
    let Some(current) = login(&stored.document) else {
        return Renewal::Unchanged;
    };
    match rejected_token {
        // Someone renewed it since the usage endpoint refused the old one.
        Some(rejected) if current.access_token.as_deref() != Some(rejected) => {
            return Renewal::Renewed;
        }
        Some(_) => {}
        None if !due(&current, Utc::now().timestamp_millis()) => return Renewal::Unchanged,
        None => {}
    }
    let Some(refresh_token) = current.refresh_token.filter(|token| !token.is_empty()) else {
        return Renewal::Unchanged;
    };
    match endpoint.exchange(&refresh_token).await {
        Ok(rotated) => save(store, cli_config_dir, &refresh_token, &rotated),
        Err(ExchangeError::Failed) => Renewal::Failed,
        Err(ExchangeError::Rejected) => {
            // Refused because someone spent it first, or because it is dead.
            let rotated_meanwhile = store
                .read(cli_config_dir)
                .ok()
                .and_then(|stored| login(&stored.document))
                .is_some_and(|login| login.refresh_token.as_deref() != Some(&refresh_token));
            if rotated_meanwhile {
                Renewal::Renewed
            } else {
                Renewal::Dead
            }
        }
    }
}

/// Saves `rotated` over the login that held `spent`, if it still does.
fn save(
    store: &dyn CredentialStore,
    cli_config_dir: &Path,
    spent: &str,
    rotated: &Rotated,
) -> Renewal {
    let Ok(latest) = store.read(cli_config_dir) else {
        return Renewal::Failed;
    };
    let still_ours =
        login(&latest.document).is_some_and(|login| login.refresh_token.as_deref() == Some(spent));
    if !still_ours {
        // Rotated by someone else while the endpoint answered; theirs stands.
        return Renewal::Renewed;
    }
    let Some(document) = merged(&latest.document, rotated) else {
        return Renewal::Failed;
    };
    match store.write(&latest.source, &document) {
        Ok(()) => Renewal::Renewed,
        Err(_) => Renewal::Failed,
    }
}

/// The plan the profile at `cli_config_dir` signed in under, e.g. `Max 20x`.
pub fn stored_plan(cli_config_dir: &Path) -> Option<String> {
    let stored = credentials::read_stored(cli_config_dir).ok()?;
    plan(&login(&stored.document)?)
}

/// The plan a login was signed in under, as Claude names it: the
/// subscription with the tier's multiplier, e.g. `Max 20x`.
fn plan(login: &OAuthLogin) -> Option<String> {
    let subscription = login.subscription_type.as_deref()?.trim();
    let mut chars = subscription.chars();
    let first = chars.next()?;
    let mut plan: String = first.to_uppercase().chain(chars).collect();
    let multiplier = login.rate_limit_tier.as_deref().and_then(|tier| {
        tier.split('_')
            .find(|part| {
                part.len() > 1
                    && part.ends_with('x')
                    && part[..part.len() - 1].chars().all(|c| c.is_ascii_digit())
            })
            .map(str::to_owned)
    });
    if let Some(multiplier) = multiplier {
        plan.push(' ');
        plan.push_str(&multiplier);
    }
    Some(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;

    fn document(access: &str, refresh: &str, expires_at: i64) -> String {
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{access}","refreshToken":"{refresh}","expiresAt":{expires_at},"subscriptionType":"max","rateLimitTier":"default_claude_max_20x","scopes":["user:profile"]}},"mcpOAuth":{{"server":{{"token":"keep"}}}}}}"#
        )
    }

    /// A store holding one document, which another party may rotate when
    /// it is read for the `rotate_on_read`th time.
    struct FakeStore {
        document: Mutex<String>,
        reads: Mutex<u32>,
        rotate_on_read: Option<(u32, String)>,
        writes: Mutex<Vec<String>>,
    }

    impl FakeStore {
        fn new(document: String) -> Self {
            Self {
                document: Mutex::new(document),
                reads: Mutex::new(0),
                rotate_on_read: None,
                writes: Mutex::new(Vec::new()),
            }
        }

        fn rotated_by_someone_else_on_read(mut self, read: u32, document: String) -> Self {
            self.rotate_on_read = Some((read, document));
            self
        }
    }

    impl CredentialStore for FakeStore {
        fn read(&self, _dir: &Path) -> Result<StoredCredentials, QuotaError> {
            let mut reads = self.reads.lock().unwrap();
            *reads += 1;
            if let Some((read, document)) = &self.rotate_on_read {
                if *reads == *read {
                    *self.document.lock().unwrap() = document.clone();
                }
            }
            Ok(StoredCredentials {
                document: self.document.lock().unwrap().clone(),
                source: CredentialSource::File(PathBuf::from("/x")),
            })
        }

        fn write(&self, _source: &CredentialSource, document: &str) -> Result<(), String> {
            self.writes.lock().unwrap().push(document.to_owned());
            *self.document.lock().unwrap() = document.to_owned();
            Ok(())
        }
    }

    struct FakeEndpoint(Result<Rotated, ExchangeError>, Mutex<Vec<String>>);

    impl FakeEndpoint {
        fn answering(answer: Result<Rotated, ExchangeError>) -> Self {
            Self(answer, Mutex::new(Vec::new()))
        }

        fn spent(&self) -> Vec<String> {
            self.1.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TokenEndpoint for FakeEndpoint {
        async fn exchange(&self, refresh_token: &str) -> Result<Rotated, ExchangeError> {
            self.1.lock().unwrap().push(refresh_token.to_owned());
            self.0.clone()
        }
    }

    fn rotated() -> Rotated {
        Rotated {
            access_token: "access-2".into(),
            refresh_token: Some("refresh-2".into()),
            expires_at_ms: Some(NOW + 8 * 3_600_000),
        }
    }

    fn dir() -> &'static Path {
        Path::new("/profiles/p1/cli-config")
    }

    #[test]
    fn a_login_is_due_five_minutes_before_it_lapses() {
        let login = |expires_at: f64| OAuthLogin {
            expires_at: Some(expires_at),
            ..OAuthLogin::default()
        };
        assert!(due(&login((NOW + 4 * 60_000) as f64), NOW));
        assert!(due(&login((NOW - 1) as f64), NOW));
        assert!(!due(&login((NOW + 6 * 60_000) as f64), NOW));
        assert!(!due(&OAuthLogin::default(), NOW));
    }

    #[test]
    fn a_rotation_replaces_only_the_tokens_and_expiry() {
        let merged = merged(&document("access-1", "refresh-1", NOW), &rotated()).unwrap();
        let value: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["claudeAiOauth"]["accessToken"], "access-2");
        assert_eq!(value["claudeAiOauth"]["refreshToken"], "refresh-2");
        assert_eq!(value["claudeAiOauth"]["expiresAt"], NOW + 8 * 3_600_000);
        assert_eq!(value["claudeAiOauth"]["subscriptionType"], "max");
        assert_eq!(value["mcpOAuth"]["server"]["token"], "keep");
    }

    #[test]
    fn a_rotation_without_a_new_refresh_token_keeps_the_old_one() {
        let rotation = Rotated {
            refresh_token: None,
            ..rotated()
        };
        let merged = merged(&document("access-1", "refresh-1", NOW), &rotation).unwrap();
        assert!(merged.contains(r#""refreshToken":"refresh-1""#));
    }

    #[test]
    fn the_endpoint_answer_is_read_as_a_rotation() {
        let body = br#"{"access_token":"a2","refresh_token":"r2","expires_in":28800,"token_type":"Bearer"}"#;
        assert_eq!(
            rotated_from(body, NOW).unwrap(),
            Rotated {
                access_token: "a2".into(),
                refresh_token: Some("r2".into()),
                expires_at_ms: Some(NOW + 28_800_000),
            }
        );
        assert_eq!(rotated_from(b"{}", NOW), Err(ExchangeError::Failed));
    }

    #[test]
    fn only_invalid_grant_counts_as_a_refused_token() {
        assert_eq!(
            rejection(400, br#"{"error":"invalid_grant"}"#),
            ExchangeError::Rejected
        );
        assert_eq!(
            rejection(400, br#"{"error":"invalid_request"}"#),
            ExchangeError::Failed
        );
        assert_eq!(rejection(503, b""), ExchangeError::Failed);
    }

    #[tokio::test]
    async fn a_due_login_is_renewed_and_saved() {
        let store = FakeStore::new(document("access-1", "refresh-1", NOW));
        let endpoint = FakeEndpoint::answering(Ok(rotated()));
        assert_eq!(
            renew_with(&endpoint, &store, dir(), None).await,
            Renewal::Renewed
        );
        assert_eq!(endpoint.spent(), ["refresh-1"]);
        assert!(store.writes.lock().unwrap()[0].contains(r#""accessToken":"access-2""#));
    }

    #[tokio::test]
    async fn a_login_not_yet_due_is_left_alone() {
        let far = Utc::now().timestamp_millis() + 3_600_000;
        let store = FakeStore::new(document("access-1", "refresh-1", far));
        let endpoint = FakeEndpoint::answering(Ok(rotated()));
        assert_eq!(
            renew_with(&endpoint, &store, dir(), None).await,
            Renewal::Unchanged
        );
        assert!(endpoint.spent().is_empty());
    }

    #[tokio::test]
    async fn a_refused_access_token_already_replaced_is_not_renewed_again() {
        let far = Utc::now().timestamp_millis() + 3_600_000;
        let store = FakeStore::new(document("access-2", "refresh-2", far));
        let endpoint = FakeEndpoint::answering(Ok(rotated()));
        assert_eq!(
            renew_with(&endpoint, &store, dir(), Some("access-1")).await,
            Renewal::Renewed
        );
        assert!(endpoint.spent().is_empty());
    }

    #[tokio::test]
    async fn a_refused_access_token_is_renewed_even_if_not_due() {
        let far = Utc::now().timestamp_millis() + 3_600_000;
        let store = FakeStore::new(document("access-1", "refresh-1", far));
        let endpoint = FakeEndpoint::answering(Ok(rotated()));
        assert_eq!(
            renew_with(&endpoint, &store, dir(), Some("access-1")).await,
            Renewal::Renewed
        );
        assert_eq!(endpoint.spent(), ["refresh-1"]);
    }

    #[tokio::test]
    async fn a_rotation_by_someone_else_meanwhile_is_not_overwritten() {
        // Claude Code rotates the login while the endpoint answers us: the
        // store's second read (the one before saving) sees its tokens.
        let store = FakeStore::new(document("access-1", "refresh-1", NOW))
            .rotated_by_someone_else_on_read(2, document("access-cc", "refresh-cc", NOW));
        let endpoint = FakeEndpoint::answering(Ok(rotated()));
        assert_eq!(
            renew_with(&endpoint, &store, dir(), None).await,
            Renewal::Renewed
        );
        assert!(store.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refused_refresh_token_nobody_rotated_is_dead() {
        let store = FakeStore::new(document("access-1", "refresh-1", NOW));
        let endpoint = FakeEndpoint::answering(Err(ExchangeError::Rejected));
        assert_eq!(
            renew_with(&endpoint, &store, dir(), None).await,
            Renewal::Dead
        );
    }

    #[tokio::test]
    async fn a_refresh_token_refused_because_someone_spent_it_first_is_not_dead() {
        let store = FakeStore::new(document("access-1", "refresh-1", NOW))
            .rotated_by_someone_else_on_read(2, document("access-cc", "refresh-cc", NOW));
        let endpoint = FakeEndpoint::answering(Err(ExchangeError::Rejected));
        assert_eq!(
            renew_with(&endpoint, &store, dir(), None).await,
            Renewal::Renewed
        );
    }

    #[tokio::test]
    async fn a_passing_failure_changes_nothing() {
        let store = FakeStore::new(document("access-1", "refresh-1", NOW));
        let endpoint = FakeEndpoint::answering(Err(ExchangeError::Failed));
        assert_eq!(
            renew_with(&endpoint, &store, dir(), None).await,
            Renewal::Failed
        );
        assert!(store.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn the_plan_reads_as_claude_names_it() {
        let login = login(&document("a", "r", NOW)).unwrap();
        assert_eq!(plan(&login).as_deref(), Some("Max 20x"));
        let pro = OAuthLogin {
            subscription_type: Some("pro".into()),
            rate_limit_tier: Some("default_claude_ai".into()),
            ..OAuthLogin::default()
        };
        assert_eq!(plan(&pro).as_deref(), Some("Pro"));
        assert_eq!(plan(&OAuthLogin::default()), None);
    }
}
