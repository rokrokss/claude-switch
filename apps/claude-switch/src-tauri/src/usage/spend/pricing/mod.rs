//! What a model's tokens cost: rates from three price lists, and how a model
//! name from a session log finds its rates.
//!
//! The lists, highest first: claude-switch's own supplement (prices the
//! public lists lack, fast-mode multipliers, alias rules), LiteLLM's model
//! prices, and models.dev's as a gap-filler. Resolution and the rate maths
//! are ported from OpenUsage's `ModelPricing` and `ModelRates` (MIT,
//! <https://github.com/robinebers/openusage>), which port ccusage's.

pub mod catalog;
pub mod store;
pub mod supplement;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use serde::Serialize;

use catalog::Catalog;
use supplement::Supplement;

/// The provider whose fallback models the supplement lists for Codex.
pub const CODEX_PROVIDER: &str = "codex";

/// A model's rates, in USD per million tokens, with optional long-context
/// rates and a fast-mode multiplier.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRates {
    pub input: f64,
    pub output: f64,
    /// A write to the 5-minute cache. For lists without a separate price,
    /// the input rate.
    pub cache_write: f64,
    pub cache_read: f64,
    /// Rates for a request whose prompt passes
    /// [`long_context_threshold`](Self::long_context_threshold): the whole
    /// request is billed at them. Named after the lists' `above_200k` fields.
    pub input_long: Option<f64>,
    pub output_long: Option<f64>,
    pub cache_write_long: Option<f64>,
    pub cache_read_long: Option<f64>,
    /// Whether the list published the cache-read rate rather than it being
    /// filled in as a tenth of input. Codex charges the full input rate for
    /// cached input when no discount is published.
    pub cache_read_explicit: bool,
    /// Prompt tokens above which the long-context rates apply.
    pub long_context_threshold: u64,
    /// What the model's fast mode multiplies every rate by; 1 without one.
    pub fast_multiplier: f64,
}

impl ModelRates {
    /// Rates from the four base prices, with no long-context tier or fast
    /// mode.
    pub fn new(input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        Self {
            input,
            output,
            cache_write,
            cache_read,
            input_long: None,
            output_long: None,
            cache_write_long: None,
            cache_read_long: None,
            cache_read_explicit: true,
            long_context_threshold: 200_000,
            fast_multiplier: 1.0,
        }
    }

    /// The same rates with every price multiplied by `factor`: how a
    /// `-fast` model name is priced off its base model.
    pub fn scaled(&self, factor: f64) -> Self {
        Self {
            input: self.input * factor,
            output: self.output * factor,
            cache_write: self.cache_write * factor,
            cache_read: self.cache_read * factor,
            input_long: self.input_long.map(|rate| rate * factor),
            output_long: self.output_long.map(|rate| rate * factor),
            cache_write_long: self.cache_write_long.map(|rate| rate * factor),
            cache_read_long: self.cache_read_long.map(|rate| rate * factor),
            cache_read_explicit: self.cache_read_explicit,
            long_context_threshold: self.long_context_threshold,
            fast_multiplier: 1.0,
        }
    }

    /// What one request of `tokens` costs, in USD: at the long-context rates
    /// when its prompt passes the threshold (unless `long_context` is off,
    /// for totals that don't keep requests apart), times the fast multiplier
    /// when it ran in fast mode. A write to the 1-hour cache costs twice the
    /// input rate.
    pub fn cost(&self, tokens: &TokenBreakdown, long_context: bool) -> f64 {
        const CACHE_WRITE_1H_INPUT_MULTIPLIER: f64 = 2.0;
        let multiplier = if tokens.is_fast {
            self.fast_multiplier
        } else {
            1.0
        };
        let long = long_context && tokens.prompt_tokens() > self.long_context_threshold;
        let pick = |base: f64, long_rate: Option<f64>| match (long, long_rate) {
            (true, Some(rate)) => rate,
            _ => base,
        };
        let input = pick(self.input, self.input_long);
        let cost = per_million(tokens.input, input)
            + per_million(tokens.output, pick(self.output, self.output_long))
            + per_million(
                tokens.cache_write_5m,
                pick(self.cache_write, self.cache_write_long),
            )
            + per_million(
                tokens.cache_write_1h,
                input * CACHE_WRITE_1H_INPUT_MULTIPLIER,
            )
            + per_million(
                tokens.cache_read,
                pick(self.cache_read, self.cache_read_long),
            );
        cost * multiplier
    }
}

fn per_million(tokens: u64, rate: f64) -> f64 {
    tokens as f64 * rate / 1_000_000.0
}

/// A request's tokens, split by how they are priced. Every log parser
/// normalises into this.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenBreakdown {
    /// Input billed at the plain input rate: neither written to nor read
    /// from the cache.
    pub input: u64,
    /// Input written to the 5-minute cache.
    pub cache_write_5m: u64,
    /// Input written to the 1-hour cache.
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub output: u64,
    /// The request ran the model's fast mode.
    pub is_fast: bool,
}

impl TokenBreakdown {
    /// The prompt, which decides whether a request is long-context.
    pub fn prompt_tokens(&self) -> u64 {
        self.input + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens() + self.output
    }
}

/// A model the user may pick to price usage no list has a price for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FallbackOption {
    pub id: String,
    pub title: String,
}

impl FallbackOption {
    /// `gpt-5.1-codex-mini` → `GPT 5.1 Codex Mini`.
    pub fn title(model: &str) -> String {
        model
            .split('-')
            .map(|part| match part {
                "gpt" => "GPT".to_owned(),
                "o1" | "o3" | "o4" => part.to_owned(),
                _ => {
                    let mut chars = part.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().chain(chars).collect())
                        .unwrap_or_default()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The three lists as of one moment, and how a model name resolves against
/// them. Immutable: a refresh builds a new one.
///
/// A name resolves through, in order:
/// 1. the supplement's alias rules, which rewrite it to a price key (the
///    name as written is tried after);
/// 2. the supplement's own prices;
/// 3. LiteLLM, exact;
/// 4. for a `-fast` name, its base model scaled by the base's fast
///    multiplier; without one it stays unpriced rather than taking the
///    standard rate;
/// 5. LiteLLM, fuzzy (see [`Catalog::find_fuzzy`]), for names without
///    `-fast`;
/// 6. models.dev, exact only: it lists resellers under near-identical ids
///    at different rates, so a fuzzy match there risks the wrong price.
pub struct ModelPricing {
    pub supplement: Supplement,
    /// LiteLLM's list: the bundled snapshot with what was fetched on top.
    pub primary: Catalog,
    /// models.dev's list, the same way.
    pub secondary: Catalog,
    /// A fuzzy miss walks every entry, so each name is resolved once.
    resolved: Mutex<HashMap<String, Option<ModelRates>>>,
    canonical: Mutex<HashMap<String, String>>,
}

impl ModelPricing {
    pub fn new(supplement: Supplement, primary: Catalog, secondary: Catalog) -> Self {
        Self {
            supplement,
            primary,
            secondary,
            resolved: Mutex::default(),
            canonical: Mutex::default(),
        }
    }

    /// The rates of `model`, or `None` when no list can price it.
    pub fn resolve(&self, model: &str) -> Option<ModelRates> {
        if let Some(cached) = lock(&self.resolved).get(model) {
            return cached.clone();
        }
        let rates = match self.supplement.canonical_name(model) {
            Some(canonical) if canonical != model => {
                self.lookup(canonical).or_else(|| self.lookup(model))
            }
            _ => self.lookup(model),
        };
        lock(&self.resolved).insert(model.to_owned(), rates.clone());
        rates
    }

    /// The price key `model` resolves through: its alias rule's, else itself.
    pub fn canonical_name(&self, model: &str) -> String {
        if let Some(cached) = lock(&self.canonical).get(model) {
            return cached.clone();
        }
        let canonical = self
            .supplement
            .canonical_name(model)
            .unwrap_or(model)
            .to_owned();
        lock(&self.canonical).insert(model.to_owned(), canonical.clone());
        canonical
    }

    /// What `tokens` of `model` cost, or `None` when it can't be priced.
    pub fn estimated_cost(&self, model: &str, tokens: &TokenBreakdown) -> Option<f64> {
        self.resolve(model).map(|rates| rates.cost(tokens, true))
    }

    /// The models the supplement offers as a fallback for `provider` that
    /// have a usable exact price, each once.
    pub fn fallback_options(&self, provider: &str) -> Vec<FallbackOption> {
        let mut seen = HashSet::new();
        self.supplement
            .fallback_models
            .get(provider)
            .into_iter()
            .flatten()
            .filter(|model| seen.insert(model.as_str()))
            .filter(|model| self.fallback_rates(model, provider).is_some())
            .map(|model| FallbackOption {
                id: model.clone(),
                title: FallbackOption::title(model),
            })
            .collect()
    }

    /// The rates of fallback `model` for `provider`: only a model the
    /// supplement lists for it, and only an exact price, so an estimate
    /// never rests on a fuzzy guess.
    pub fn fallback_rates(&self, model: &str, provider: &str) -> Option<ModelRates> {
        let listed = self
            .supplement
            .fallback_models
            .get(provider)
            .is_some_and(|models| models.iter().any(|listed| listed == model));
        if !listed {
            return None;
        }
        let rates = self
            .supplement
            .pricing
            .get(model)
            .or_else(|| self.primary.find_exact(model).map(|(_, rates)| rates))
            .or_else(|| self.secondary.find_exact(model).map(|(_, rates)| rates))?;
        let usable = rates.input.is_finite()
            && rates.input > 0.0
            && rates.output.is_finite()
            && rates.output > 0.0
            && rates.cache_read.is_finite()
            && rates.cache_read >= 0.0;
        usable.then(|| rates.clone())
    }

    /// models.dev is asked only after the whole LiteLLM lookup misses: it
    /// aggregates resellers, so LiteLLM wins whenever it knows the model.
    fn lookup(&self, name: &str) -> Option<ModelRates> {
        if let Some(rates) = self.supplement.pricing.get(name) {
            return Some(rates.clone());
        }
        if let Some((_, rates)) = self.primary.find_exact(name) {
            return Some(rates.clone());
        }
        if let Some(rates) = self.fast_variant(name) {
            return Some(rates);
        }
        if name.ends_with("-fast") {
            return self
                .secondary
                .find_exact(name)
                .map(|(_, rates)| rates.clone());
        }
        if let Some((_, rates)) = self.primary.find_fuzzy(name) {
            return Some(rates.clone());
        }
        self.secondary
            .find_exact(name)
            .map(|(_, rates)| rates.clone())
    }

    /// A `<base>-fast` name priced off its base model, when the base has a
    /// known fast multiplier.
    fn fast_variant(&self, name: &str) -> Option<ModelRates> {
        let base = name.strip_suffix("-fast").filter(|base| !base.is_empty())?;
        let (key, rates) = self.base_entry(base)?;
        let multiplier = if rates.fast_multiplier != 1.0 {
            rates.fast_multiplier
        } else {
            self.supplement
                .fast_multiplier(&key)
                .or_else(|| self.supplement.fast_multiplier(base))?
        };
        Some(rates.scaled(multiplier))
    }

    fn base_entry(&self, base: &str) -> Option<(String, ModelRates)> {
        if let Some(rates) = self.supplement.pricing.get(base) {
            return Some((base.to_owned(), rates.clone()));
        }
        self.primary
            .find_exact(base)
            .or_else(|| self.primary.find_fuzzy(base))
            .or_else(|| self.secondary.find_exact(base))
            .map(|(key, rates)| (key.to_owned(), rates.clone()))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A pricing of the given lists, from compact catalog JSON and
    /// supplement JSON.
    pub(crate) fn pricing(supplement: &str, litellm: &str, models_dev: &str) -> ModelPricing {
        ModelPricing::new(
            Supplement::decode(supplement.as_bytes()).unwrap(),
            catalog::from_compact(litellm.as_bytes()).unwrap(),
            catalog::from_compact(models_dev.as_bytes()).unwrap(),
        )
    }

    const EMPTY_SUPPLEMENT: &str = r#"{"pricing":{},"alias_rules":[]}"#;
    const EMPTY_CATALOG: &str = r#"{"models":{}}"#;

    fn approx(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    #[test]
    fn a_request_costs_each_bucket_at_its_rate() {
        let rates = ModelRates::new(3.0, 15.0, 3.75, 0.3);
        let tokens = TokenBreakdown {
            input: 1_000_000,
            cache_write_5m: 1_000_000,
            cache_write_1h: 1_000_000,
            cache_read: 1_000_000,
            output: 1_000_000,
            is_fast: false,
        };
        // A 1-hour write is twice the input rate.
        assert!(approx(
            rates.cost(&tokens, true),
            3.0 + 15.0 + 3.75 + 6.0 + 0.3
        ));
    }

    #[test]
    fn a_long_prompt_bills_the_whole_request_at_the_long_rates() {
        let mut rates = ModelRates::new(3.0, 15.0, 3.75, 0.3);
        rates.input_long = Some(6.0);
        rates.output_long = Some(22.5);
        let short = TokenBreakdown {
            input: 200_000,
            output: 1_000_000,
            ..TokenBreakdown::default()
        };
        assert!(approx(rates.cost(&short, true), 0.6 + 15.0));
        let long = TokenBreakdown {
            input: 200_001,
            ..short
        };
        assert!(approx(rates.cost(&long, true), 0.200_001 * 6.0 + 22.5));
        // Totals that don't keep requests apart stay at the base rates.
        assert!(approx(rates.cost(&long, false), 0.200_001 * 3.0 + 15.0));
    }

    #[test]
    fn fast_mode_multiplies_the_whole_request() {
        let mut rates = ModelRates::new(5.0, 25.0, 6.25, 0.5);
        rates.fast_multiplier = 2.0;
        let tokens = TokenBreakdown {
            output: 1_000_000,
            is_fast: true,
            ..TokenBreakdown::default()
        };
        assert!(approx(rates.cost(&tokens, true), 50.0));
    }

    #[test]
    fn the_supplement_wins_over_the_lists_and_aliases_resolve_first() {
        let pricing = pricing(
            r#"{"pricing":{"claude-opus-5-5":{"input_per_million":4,"output_per_million":20}},
                "alias_rules":[{"pattern":"^claude-opus-5[.-]5(?:\\[1m\\])?$","canonical":"claude-opus-5-5"}]}"#,
            r#"{"models":{"claude-opus-5-5":{"i":99,"o":99,"cw":99,"cr":99}}}"#,
            EMPTY_CATALOG,
        );
        let rates = pricing.resolve("claude-opus-5.5[1m]").unwrap();
        assert_eq!(rates.input, 4.0);
        // Missing cache prices default to the input rate and a tenth of it.
        assert_eq!(rates.cache_write, 4.0);
        assert!(approx(rates.cache_read, 0.4));
        assert!(!rates.cache_read_explicit);
        assert_eq!(
            pricing.canonical_name("claude-opus-5.5[1m]"),
            "claude-opus-5-5"
        );
        assert_eq!(pricing.canonical_name("gpt-5"), "gpt-5");
    }

    #[test]
    fn an_alias_target_without_a_price_falls_back_to_the_name_as_written() {
        let pricing = pricing(
            r#"{"pricing":{},"alias_rules":[{"pattern":"^odd$","canonical":"missing"}]}"#,
            r#"{"models":{"odd":{"i":1,"o":2,"cw":1,"cr":0.1}}}"#,
            EMPTY_CATALOG,
        );
        assert_eq!(pricing.resolve("odd").unwrap().output, 2.0);
    }

    #[test]
    fn a_fast_name_scales_its_base_by_the_fast_multiplier() {
        let pricing = pricing(
            r#"{"pricing":{},"alias_rules":[],"fast_multipliers":{"gpt-5.5":2.5}}"#,
            r#"{"models":{"gpt-5.5":{"i":5,"o":30,"cw":5,"cr":0.5},
                          "claude-opus-4-8":{"i":5,"o":25,"cw":6.25,"cr":0.5,"fast":2},
                          "claude-sonnet-4-6":{"i":3,"o":15,"cw":3.75,"cr":0.3}}}"#,
            r#"{"models":{"claude-sonnet-4-6-fast":{"i":9,"o":45,"cw":9,"cr":0.9}}}"#,
        );
        assert_eq!(pricing.resolve("gpt-5.5-fast").unwrap().output, 75.0);
        // The list's own multiplier comes first.
        assert_eq!(pricing.resolve("claude-opus-4-8-fast").unwrap().input, 10.0);
        // Without a multiplier only an exact models.dev entry prices it,
        // never the standard-speed base.
        assert_eq!(
            pricing.resolve("claude-sonnet-4-6-fast").unwrap().input,
            9.0
        );
        assert!(pricing.resolve("claude-haiku-4-5-fast").is_none());
    }

    #[test]
    fn litellm_matches_loosely_and_models_dev_only_exactly() {
        let pricing = pricing(
            EMPTY_SUPPLEMENT,
            r#"{"models":{"claude-sonnet-4-20250514":{"i":3,"o":15,"cw":3.75,"cr":0.3}}}"#,
            r#"{"models":{"glm-5.2":{"i":1,"o":2,"cw":1,"cr":0.1}}}"#,
        );
        assert_eq!(pricing.resolve("claude-sonnet-4").unwrap().input, 3.0);
        assert_eq!(pricing.resolve("glm-5.2").unwrap().input, 1.0);
        assert!(pricing.resolve("glm-5-2").is_none());
        assert!(pricing.resolve("claude-sonnet-4-5").is_none());
    }

    #[test]
    fn fallback_models_need_a_listing_and_an_exact_price() {
        let pricing = pricing(
            r#"{"pricing":{},"alias_rules":[],"fallback_models":{"codex":["gpt-5.5","gpt-5.5","gpt-9","o3"]}}"#,
            r#"{"models":{"gpt-5.5":{"i":5,"o":30,"cw":5,"cr":0.5},"gpt-4o":{"i":2.5,"o":10,"cw":2.5,"cr":1.25},
                          "o3":{"i":2,"o":8,"cw":2,"cr":0.5}}}"#,
            EMPTY_CATALOG,
        );
        let options = pricing.fallback_options(CODEX_PROVIDER);
        assert_eq!(
            options,
            [
                FallbackOption {
                    id: "gpt-5.5".into(),
                    title: "GPT 5.5".into()
                },
                FallbackOption {
                    id: "o3".into(),
                    title: "o3".into()
                }
            ]
        );
        assert!(pricing.fallback_rates("gpt-4o", CODEX_PROVIDER).is_none());
        assert_eq!(
            FallbackOption::title("codex-mini-latest"),
            "Codex Mini Latest"
        );
    }
}
