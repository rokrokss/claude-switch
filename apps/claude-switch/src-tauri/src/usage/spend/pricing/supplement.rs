//! claude-switch's own price list: prices the public lists don't carry yet,
//! fast-mode multipliers they omit, and alias rules mapping the model names
//! in session logs to price keys. Bundled with the app and published at
//! <https://claude-switch.shizue.net/pricing-supplement.json> from
//! `apps/landing/public/pricing-supplement.json`, so a correction reaches
//! installed apps without a release. Ported from OpenUsage's
//! `PricingSupplement` (MIT, <https://github.com/robinebers/openusage>).

use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use serde::Deserialize;

use super::catalog::normalized_key;
use super::ModelRates;

#[derive(Debug, Clone, Default)]
pub struct Supplement {
    /// Models priced here, over every other list.
    pub pricing: HashMap<String, ModelRates>,
    /// Base model → its fast mode's multiplier, for `-fast` names whose
    /// lists carry none. An entry priced here also carries its own.
    pub fast_multipliers: BTreeMap<String, f64>,
    /// Model name pattern → price key; the first match wins.
    pub alias_rules: Vec<(Regex, String)>,
    /// When the file was last changed, ISO 8601, so lexical order is
    /// chronological. Of a cached and a bundled copy the newer is used.
    pub updated_at: Option<String>,
    /// Models offered as fallback prices, by usage provider (`codex`).
    pub fallback_models: HashMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct File {
    updated_at: Option<String>,
    pricing: HashMap<String, Entry>,
    fast_multipliers: Option<BTreeMap<String, f64>>,
    alias_rules: Vec<Rule>,
    fallback_models: Option<HashMap<String, Vec<String>>>,
}

#[derive(Deserialize)]
struct Entry {
    input_per_million: f64,
    output_per_million: f64,
    cache_write_per_million: Option<f64>,
    cache_read_per_million: Option<f64>,
}

#[derive(Deserialize)]
struct Rule {
    pattern: String,
    canonical: String,
}

impl Supplement {
    /// Reads the file. A pattern that isn't a valid regex is skipped, with a
    /// line in the log, rather than failing the whole file.
    pub fn decode(body: &[u8]) -> Result<Self, serde_json::Error> {
        let file: File = serde_json::from_slice(body)?;
        let fast_multipliers = file.fast_multipliers.unwrap_or_default();
        let pricing = file
            .pricing
            .into_iter()
            .map(|(model, entry)| {
                let mut rates = ModelRates::new(
                    entry.input_per_million,
                    entry.output_per_million,
                    entry
                        .cache_write_per_million
                        .unwrap_or(entry.input_per_million),
                    entry
                        .cache_read_per_million
                        .unwrap_or(entry.input_per_million * 0.1),
                );
                rates.cache_read_explicit = entry.cache_read_per_million.is_some();
                // Requests flagged fast in the log (Claude's `speed`) are
                // priced under the base name, so the entry carries its own.
                rates.fast_multiplier = fast_multipliers.get(&model).copied().unwrap_or(1.0);
                (model, rates)
            })
            .collect();
        let alias_rules = file
            .alias_rules
            .into_iter()
            .filter_map(|rule| match Regex::new(&rule.pattern) {
                Ok(pattern) => Some((pattern, rule.canonical)),
                Err(err) => {
                    eprintln!(
                        "claude-switch: skipped the price alias {:?}: {err}",
                        rule.pattern
                    );
                    None
                }
            })
            .collect();
        Ok(Self {
            pricing,
            fast_multipliers,
            alias_rules,
            updated_at: file.updated_at,
            fallback_models: file.fallback_models.unwrap_or_default(),
        })
    }

    /// The price key the alias rules give `model`, if any matches.
    pub fn canonical_name(&self, model: &str) -> Option<&str> {
        self.alias_rules
            .iter()
            .find(|(pattern, _)| pattern.is_match(model))
            .map(|(_, canonical)| canonical.as_str())
    }

    /// The fast multiplier of base model `model`: by its exact key, else by a
    /// key it ends with, so a dated key (`gpt-5.5-2026-04-23`) finds its
    /// base.
    pub fn fast_multiplier(&self, model: &str) -> Option<f64> {
        if let Some(multiplier) = self.fast_multipliers.get(model) {
            return Some(*multiplier);
        }
        let normalized = normalized_key(model);
        normalized
            .split(['/', ':'])
            .filter(|part| !part.is_empty())
            .find_map(|part| {
                self.fast_multipliers
                    .iter()
                    .find(|(base, _)| ends_with_model(part, &normalized_key(base)))
                    .map(|(_, multiplier)| *multiplier)
            })
    }

    /// This copy, with the fallback models of `other` for any provider it
    /// lists none for: older feeds carry none, and an empty list still
    /// counts as one.
    pub fn filling_missing_fallback_models(mut self, other: &Supplement) -> Self {
        for (provider, models) in &other.fallback_models {
            self.fallback_models
                .entry(provider.clone())
                .or_insert_with(|| models.clone());
        }
        self
    }
}

/// `base` last occurs in `part` with nothing after it, or a `-`.
fn ends_with_model(part: &str, base: &str) -> bool {
    part.rfind(base).is_some_and(|at| {
        part[at + base.len()..].is_empty() || part[at + base.len()..].starts_with('-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_supplement_reads_and_every_alias_compiles() {
        let raw: serde_json::Value =
            serde_json::from_slice(super::super::store::BUNDLED_SUPPLEMENT).unwrap();
        let supplement = Supplement::decode(super::super::store::BUNDLED_SUPPLEMENT).unwrap();
        assert_eq!(
            supplement.alias_rules.len(),
            raw["alias_rules"].as_array().unwrap().len()
        );
        assert!(supplement.updated_at.is_some());
        assert!(!supplement.fallback_models["codex"].is_empty());
        assert_eq!(supplement.pricing["claude-opus-5-5"].fast_multiplier, 2.0);
    }

    #[test]
    fn the_first_matching_alias_wins_and_bad_patterns_are_skipped() {
        let supplement = Supplement::decode(
            br#"{"pricing":{},"alias_rules":[
                {"pattern":"(unclosed","canonical":"x"},
                {"pattern":"^gpt-5\\.5(?:-(?:low|high))?$","canonical":"gpt-5.5"},
                {"pattern":"^gpt-5","canonical":"gpt-5"}]}"#,
        )
        .unwrap();
        assert_eq!(supplement.alias_rules.len(), 2);
        assert_eq!(supplement.canonical_name("gpt-5.5-high"), Some("gpt-5.5"));
        assert_eq!(supplement.canonical_name("gpt-5-mini"), Some("gpt-5"));
        assert_eq!(supplement.canonical_name("claude-opus-4-8"), None);
    }

    #[test]
    fn a_dated_key_finds_its_base_fast_multiplier() {
        let supplement = Supplement::decode(
            br#"{"pricing":{},"alias_rules":[],"fast_multipliers":{"gpt-5.5":2.5}}"#,
        )
        .unwrap();
        assert_eq!(supplement.fast_multiplier("gpt-5.5"), Some(2.5));
        assert_eq!(
            supplement.fast_multiplier("openai/gpt-5.5-2026-04-23"),
            Some(2.5)
        );
        assert_eq!(supplement.fast_multiplier("gpt-5.5x"), None);
        assert_eq!(supplement.fast_multiplier("gpt-5"), None);
    }

    #[test]
    fn missing_fallback_lists_come_from_the_other_copy() {
        let feed = Supplement::decode(br#"{"pricing":{},"alias_rules":[]}"#).unwrap();
        let bundled = Supplement::decode(
            br#"{"pricing":{},"alias_rules":[],"fallback_models":{"codex":["gpt-5.5"]}}"#,
        )
        .unwrap();
        let merged = feed.filling_missing_fallback_models(&bundled);
        assert_eq!(merged.fallback_models["codex"], ["gpt-5.5"]);
        let emptied = Supplement::decode(
            br#"{"pricing":{},"alias_rules":[],"fallback_models":{"codex":[]}}"#,
        )
        .unwrap()
        .filling_missing_fallback_models(&bundled);
        assert!(emptied.fallback_models["codex"].is_empty());
    }
}
