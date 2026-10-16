//! Where the price lists come from: snapshots bundled with the app, so
//! pricing works offline and on first launch, and copies fetched about once
//! an hour (with ETag revalidation) and kept in `<app data>/pricing/`.
//! Reading prices never waits on the network: a scan prices against what is
//! on hand, and a refresh shows up in the next one. Ported from OpenUsage's
//! `ModelPricingStore` (MIT, <https://github.com/robinebers/openusage>).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::catalog::{self, Catalog};
use super::supplement::Supplement;
use super::{lock, ModelPricing};
use crate::app_state::atomic_write;

pub const BUNDLED_LITELLM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/pricing/litellm.json"
));
pub const BUNDLED_MODELS_DEV: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/pricing/models-dev.json"
));
/// The same file the landing site publishes, so what ships and what is
/// served never drift apart.
pub const BUNDLED_SUPPLEMENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../landing/public/pricing-supplement.json"
));

/// A list is fetched again this long after it last was.
const REFRESH_INTERVAL: chrono::Duration = chrono::Duration::hours(1);
/// After a failed fetch, the next waits this long.
const RETRY_INTERVAL: chrono::Duration = chrono::Duration::minutes(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Litellm,
    ModelsDev,
    Supplement,
}

impl Source {
    const ALL: [Source; 3] = [Source::Litellm, Source::ModelsDev, Source::Supplement];

    pub fn url(self) -> &'static str {
        match self {
            Source::Litellm => "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json",
            Source::ModelsDev => "https://models.dev/api.json",
            Source::Supplement => "https://claude-switch.shizue.net/pricing-supplement.json",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Source::Litellm => "litellm.json",
            Source::ModelsDev => "models_dev.json",
            Source::Supplement => "supplement.json",
        }
    }
}

/// When a list was last fetched, and with what ETag.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SourceState {
    etag: Option<String>,
    fetched_at: Option<DateTime<Utc>>,
    failed_at: Option<DateTime<Utc>>,
}

impl SourceState {
    fn due(&self, now: DateTime<Utc>) -> bool {
        if self.failed_at.is_some_and(|at| now - at < RETRY_INTERVAL) {
            return false;
        }
        self.fetched_at
            .is_none_or(|at| now - at >= REFRESH_INTERVAL)
    }
}

/// What a fetch brought back.
pub struct Fetched {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait Fetch: Send + Sync {
    /// GETs `url`, revalidating against `etag` when there is one.
    async fn get(&self, url: &str, etag: Option<&str>) -> Result<Fetched, String>;
}

/// Fetches over HTTPS, as `claude-switch/<version>`. The requests carry no
/// usage data.
pub struct HttpFetch {
    client: reqwest::Client,
}

impl HttpFetch {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(concat!("claude-switch/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| err.to_string())?;
        Ok(Self { client })
    }
}

#[async_trait]
impl Fetch for HttpFetch {
    async fn get(&self, url: &str, etag: Option<&str>) -> Result<Fetched, String> {
        let mut request = self.client.get(url);
        if let Some(etag) = etag {
            request = request.header("if-none-match", etag);
        }
        let response = request.send().await.map_err(|err| err.to_string())?;
        let status = response.status().as_u16();
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.bytes().await.map_err(|err| err.to_string())?;
        Ok(Fetched {
            status,
            etag,
            body: body.to_vec(),
        })
    }
}

pub struct PricingStore {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    pricing: Option<Arc<ModelPricing>>,
    states: Option<BTreeMap<Source, SourceState>>,
    refreshing: bool,
}

impl PricingStore {
    /// A store keeping its fetched lists in `dir`.
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            inner: Mutex::default(),
        }
    }

    /// The prices on hand: the bundled lists with what was fetched over them.
    pub fn current(&self) -> Arc<ModelPricing> {
        lock(&self.inner)
            .pricing
            .get_or_insert_with(|| Arc::new(build(&self.dir)))
            .clone()
    }

    /// Whether any list is due a fetch.
    pub fn due(&self, now: DateTime<Utc>) -> bool {
        let mut inner = lock(&self.inner);
        if inner.refreshing {
            return false;
        }
        let states = inner.states.get_or_insert_with(|| read_states(&self.dir));
        Source::ALL
            .into_iter()
            .any(|source| states.get(&source).cloned().unwrap_or_default().due(now))
    }

    /// Fetches each list that is due, and rebuilds the prices if any came
    /// back changed. Does nothing while another refresh runs. A list that
    /// fails to fetch, or fetches as something unreadable, keeps what was on
    /// hand.
    pub async fn refresh_due(&self, fetch: &dyn Fetch, now: DateTime<Utc>) {
        let (due, mut states) = {
            let mut inner = lock(&self.inner);
            if inner.refreshing {
                return;
            }
            let states = inner
                .states
                .get_or_insert_with(|| read_states(&self.dir))
                .clone();
            let due: Vec<Source> = Source::ALL
                .into_iter()
                .filter(|source| states.get(source).cloned().unwrap_or_default().due(now))
                .collect();
            if due.is_empty() {
                return;
            }
            inner.refreshing = true;
            (due, states)
        };
        let mut changed = false;
        for source in due {
            let state = states.entry(source).or_default();
            match self.fetch_one(fetch, source, state.etag.as_deref()).await {
                Ok(Some(etag)) => {
                    *state = SourceState {
                        etag,
                        fetched_at: Some(now),
                        failed_at: None,
                    };
                    changed = true;
                }
                Ok(None) => {
                    state.fetched_at = Some(now);
                    state.failed_at = None;
                }
                Err(err) => {
                    state.failed_at = Some(now);
                    eprintln!(
                        "claude-switch: could not refresh the {source:?} price list, keeping the one on hand: {err}"
                    );
                }
            }
        }
        if let Err(err) = serde_json::to_vec(&states)
            .map_err(|err| err.to_string())
            .and_then(|body| {
                atomic_write(&self.dir.join("state.json"), &body).map_err(|err| err.message())
            })
        {
            eprintln!("claude-switch: could not save when the price lists were fetched: {err}");
        }
        let rebuilt = changed.then(|| Arc::new(build(&self.dir)));
        let mut inner = lock(&self.inner);
        inner.states = Some(states);
        if rebuilt.is_some() {
            inner.pricing = rebuilt;
        }
        inner.refreshing = false;
    }

    /// Fetches `source` and keeps it. `Some(etag)` when a new copy was kept,
    /// `None` when the one on hand is still current.
    async fn fetch_one(
        &self,
        fetch: &dyn Fetch,
        source: Source,
        etag: Option<&str>,
    ) -> Result<Option<Option<String>>, String> {
        let fetched = fetch.get(source.url(), etag).await?;
        match fetched.status {
            200 => {
                let kept = validated(source, &fetched.body)?;
                atomic_write(&self.dir.join(source.file_name()), &kept)
                    .map_err(|err| err.message())?;
                Ok(Some(fetched.etag))
            }
            304 => Ok(None),
            status => Err(format!("HTTP {status}")),
        }
    }
}

/// What to keep of a fetched `body`: the lists cut down to the compact form,
/// the supplement as it came. An error for a body that doesn't read, so it
/// never replaces a good copy.
fn validated(source: Source, body: &[u8]) -> Result<Vec<u8>, String> {
    let catalog = match source {
        Source::Litellm => catalog::from_litellm(body),
        Source::ModelsDev => catalog::from_models_dev(body),
        Source::Supplement => {
            Supplement::decode(body).map_err(|err| err.to_string())?;
            return Ok(body.to_vec());
        }
    };
    catalog
        .and_then(|catalog| catalog::to_compact(&catalog))
        .map_err(|err| err.to_string())
}

fn read_states(dir: &Path) -> BTreeMap<Source, SourceState> {
    fs::read(dir.join("state.json"))
        .ok()
        .and_then(|body| serde_json::from_slice(&body).ok())
        .unwrap_or_default()
}

fn build(dir: &Path) -> ModelPricing {
    ModelPricing::new(
        supplement(dir),
        catalog_for(dir, Source::Litellm, BUNDLED_LITELLM),
        catalog_for(dir, Source::ModelsDev, BUNDLED_MODELS_DEV),
    )
}

/// The newer of the fetched and the bundled supplement by `updated_at`: an
/// app update can ship rates newer than an hour-old copy, or than one that
/// can no longer be fetched. The bundled copy wins only when strictly newer.
fn supplement(dir: &Path) -> Supplement {
    let bundled = Supplement::decode(BUNDLED_SUPPLEMENT)
        .inspect_err(|err| {
            eprintln!("claude-switch: the bundled price supplement doesn't read: {err}")
        })
        .ok();
    let fetched = fs::read(dir.join(Source::Supplement.file_name()))
        .ok()
        .and_then(|body| Supplement::decode(&body).ok());
    match (fetched, bundled) {
        (Some(fetched), Some(bundled)) => {
            let preferred = if newer(bundled.updated_at.as_deref(), fetched.updated_at.as_deref()) {
                bundled.clone()
            } else {
                fetched
            };
            preferred.filling_missing_fallback_models(&bundled)
        }
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => Supplement::default(),
    }
}

/// Zero-padded ISO 8601 sorts chronologically as text; a missing date is
/// the oldest.
fn newer(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(left), Some(right)) => left > right,
    }
}

/// The bundled snapshot of `source` with the fetched copy over it, so a
/// model the feed drops keeps its snapshot price.
fn catalog_for(dir: &Path, source: Source, bundled: &[u8]) -> Catalog {
    let snapshot = catalog::from_compact(bundled)
        .inspect_err(|err| {
            eprintln!("claude-switch: the bundled {source:?} prices don't read: {err}")
        })
        .unwrap_or_default();
    match fs::read(dir.join(source.file_name())) {
        Ok(body) => match catalog::from_compact(&body) {
            Ok(fetched) => snapshot.merging(fetched),
            Err(_) => snapshot,
        },
        Err(_) => snapshot,
    }
}

static STORE: OnceLock<Option<PricingStore>> = OnceLock::new();

/// The app's store, keeping its lists in `<app data>/pricing/`.
fn global() -> Option<&'static PricingStore> {
    STORE
        .get_or_init(|| {
            crate::paths::app_data_dir()
                .ok()
                .map(|dir| PricingStore::new(dir.join("pricing")))
        })
        .as_ref()
}

/// The prices on hand, for a scan.
pub fn current() -> Arc<ModelPricing> {
    match global() {
        Some(store) => store.current(),
        None => Arc::new(build(Path::new(""))),
    }
}

/// Fetches the lists that are due. Called with every usage refresh; returns
/// at once while a refresh runs or none is due.
pub async fn refresh() {
    let Some(store) = global().filter(|store| store.due(Utc::now())) else {
        return;
    };
    match HttpFetch::new() {
        Ok(fetch) => store.refresh_due(&fetch, Utc::now()).await,
        Err(err) => eprintln!("claude-switch: could not set up price list fetches: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    /// Answers each GET with the next queued reply, recording what was asked.
    #[derive(Default)]
    struct Script {
        replies: Mutex<VecDeque<Result<Fetched, String>>>,
        asked: Mutex<Vec<(String, Option<String>)>>,
    }

    impl Script {
        fn reply(&self, status: u16, etag: Option<&str>, body: &str) {
            lock(&self.replies).push_back(Ok(Fetched {
                status,
                etag: etag.map(str::to_owned),
                body: body.as_bytes().to_vec(),
            }));
        }
    }

    #[async_trait]
    impl Fetch for Script {
        async fn get(&self, url: &str, etag: Option<&str>) -> Result<Fetched, String> {
            lock(&self.asked).push((url.to_owned(), etag.map(str::to_owned)));
            lock(&self.replies)
                .pop_front()
                .unwrap_or_else(|| Err("offline".into()))
        }
    }

    const LITELLM: &str =
        r#"{"claude-test-1":{"input_cost_per_token":0.000001,"output_cost_per_token":0.000002}}"#;

    fn at(hours: i64) -> DateTime<Utc> {
        "2026-10-17T00:00:00Z".parse::<DateTime<Utc>>().unwrap() + chrono::Duration::hours(hours)
    }

    #[test]
    fn with_nothing_fetched_the_bundled_lists_price() {
        let dir = tempfile::tempdir().unwrap();
        let pricing = PricingStore::new(dir.path().to_path_buf()).current();
        assert!(pricing.resolve("claude-opus-5-5").is_some());
        assert!(pricing.resolve("gpt-5").is_some());
    }

    #[tokio::test]
    async fn a_fetched_list_is_kept_compact_and_priced_over_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let store = PricingStore::new(dir.path().to_path_buf());
        assert!(store.current().resolve("claude-test-1").is_none());
        let script = Script::default();
        script.reply(200, Some("\"v1\""), LITELLM);
        store.refresh_due(&script, at(0)).await;
        assert_eq!(store.current().resolve("claude-test-1").unwrap().input, 1.0);
        // Snapshot-only models survive.
        assert!(store.current().resolve("gpt-5").is_some());
        let kept = fs::read_to_string(dir.path().join("litellm.json")).unwrap();
        assert!(kept.contains(r#""i":1.0"#), "{kept}");
        // The two lists that failed wait out the retry; LiteLLM waits an hour.
        let asked = lock(&script.asked).len();
        store
            .refresh_due(&script, at(0) + chrono::Duration::minutes(20))
            .await;
        assert_eq!(lock(&script.asked).len(), asked);
        // An hour on, LiteLLM revalidates with its ETag.
        script.reply(304, None, "");
        store.refresh_due(&script, at(1)).await;
        let asked = lock(&script.asked).clone();
        let revalidated = asked
            .iter()
            .rev()
            .find(|(url, _)| url == Source::Litellm.url());
        assert_eq!(revalidated.unwrap().1.as_deref(), Some("\"v1\""));
        // A new store reads what was kept.
        let reopened = PricingStore::new(dir.path().to_path_buf());
        assert!(reopened.current().resolve("claude-test-1").is_some());
        assert!(!read_states(dir.path())[&Source::Litellm].due(at(1)));
    }

    #[tokio::test]
    async fn an_unreadable_fetch_never_replaces_a_good_copy() {
        let dir = tempfile::tempdir().unwrap();
        let store = PricingStore::new(dir.path().to_path_buf());
        let script = Script::default();
        script.reply(200, None, LITELLM);
        store.refresh_due(&script, at(0)).await;
        script.reply(200, None, "<html>rate limited</html>");
        store.refresh_due(&script, at(2)).await;
        assert!(store.current().resolve("claude-test-1").is_some());
        assert!(read_states(dir.path())[&Source::Litellm]
            .failed_at
            .is_some());
    }

    #[test]
    fn the_newer_supplement_wins() {
        let dir = tempfile::tempdir().unwrap();
        let write = |updated_at: &str| {
            let body = format!(
                r#"{{"updated_at":"{updated_at}","pricing":{{"gpt-test":{{"input_per_million":7,"output_per_million":8}}}},"alias_rules":[]}}"#
            );
            fs::write(dir.path().join("supplement.json"), body).unwrap();
        };
        write("2020-01-01T00:00:00Z");
        let old = supplement(dir.path());
        assert!(!old.pricing.contains_key("gpt-test"));
        write("2999-01-01T00:00:00Z");
        let new = supplement(dir.path());
        assert!(new.pricing.contains_key("gpt-test"));
        // The fetched copy lists no fallback models, so the bundled ones stay.
        assert!(!new.fallback_models["codex"].is_empty());
        assert!(newer(Some("2026-10-17T00:00:00Z"), Some("2026-10-17")));
        assert!(!newer(None, Some("2026-10-17")));
    }
}
