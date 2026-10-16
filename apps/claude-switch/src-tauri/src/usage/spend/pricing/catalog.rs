//! One public price list (LiteLLM's or models.dev's) as a table of model
//! keys to rates, how a model name finds its entry, and the lists' formats:
//! the feeds as published, and the compact form claude-switch bundles and
//! caches (prices only; the feeds run to megabytes).
//!
//! Lookup ports ccusage's `pricing.rs` by way of OpenUsage's
//! `PricingCatalog` (MIT, <https://github.com/robinebers/openusage>): an exact
//! key first, then a match at word boundaries that tolerates provider
//! prefixes (`anthropic/claude-sonnet-4`), separator variants (`grok-4.3` for
//! `grok-4-3`) and date suffixes (`claude-sonnet-4` for
//! `claude-sonnet-4-20250514`), without taking one version for another
//! (`claude-sonnet-4` never matches `claude-sonnet-4-5`).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::ModelRates;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub entries: HashMap<String, ModelRates>,
    /// When the list was taken, as its snapshot says; for logs only.
    pub retrieved_at: Option<String>,
}

impl Catalog {
    pub fn find_exact(&self, model: &str) -> Option<(&str, &ModelRates)> {
        self.entries
            .get_key_value(model)
            .map(|(key, rates)| (key.as_str(), rates))
    }

    /// The entry whose key `model` loosely matches (see the module docs):
    /// the longest such key, then the first in byte order.
    pub fn find_fuzzy(&self, model: &str) -> Option<(&str, &ModelRates)> {
        let normalized = normalized_key(model);
        let mut best: Option<(&str, &ModelRates)> = None;
        for (key, rates) in &self.entries {
            if !key_matches(key, model, &normalized) {
                continue;
            }
            let better = best.is_none_or(|(current, _)| {
                let (length, current_length) = (key.chars().count(), current.chars().count());
                length > current_length || (length == current_length && key.as_str() < current)
            });
            if better {
                best = Some((key, rates));
            }
        }
        best
    }

    /// This list with `other`'s entries over it.
    pub fn merging(mut self, other: Catalog) -> Catalog {
        self.entries.extend(other.entries);
        Catalog {
            entries: self.entries,
            retrieved_at: other.retrieved_at.or(self.retrieved_at),
        }
    }
}

/// `.` and `@` read as `-`: `grok-4.3` → `grok-4-3`.
pub fn normalized_key(value: &str) -> String {
    value.replace(['.', '@'], "-")
}

/// Either name contains the other at word boundaries, as written or with
/// separators normalised.
fn key_matches(candidate: &str, model: &str, normalized_model: &str) -> bool {
    if contains_key(model, candidate) || contains_key(candidate, model) {
        return true;
    }
    let normalized_candidate = normalized_key(candidate);
    contains_key(normalized_model, &normalized_candidate)
        || contains_key(&normalized_candidate, normalized_model)
}

/// Whether `key` occurs in `value` with no letter or digit right before it,
/// and nothing after it that carries on the name or its version number.
fn contains_key(value: &str, key: &str) -> bool {
    let (value, key) = (value.as_bytes(), key.as_bytes());
    if key.is_empty() || key.len() > value.len() {
        return false;
    }
    (0..=value.len() - key.len()).any(|start| {
        value[start..start + key.len()] == *key
            && (start == 0 || !value[start - 1].is_ascii_alphanumeric())
            && suffix_allows_match(key, &value[start + key.len()..])
    })
}

fn suffix_allows_match(key: &[u8], suffix: &[u8]) -> bool {
    match suffix.first() {
        None => true,
        Some(separator) if separator.is_ascii_alphanumeric() => false,
        Some(_) => !continues_version(key, suffix),
    }
}

/// A suffix that carries on a key ending in a digit as a version
/// (`claude-sonnet-4` + `-5…`), but not an 8-digit date (`-20250514`).
fn continues_version(key: &[u8], suffix: &[u8]) -> bool {
    const DATE_DIGITS: usize = 8;
    if !key.last().is_some_and(u8::is_ascii_digit) {
        return false;
    }
    if !matches!(suffix.first(), Some(b'-' | b'.')) {
        return false;
    }
    let rest = &suffix[1..];
    let digits = rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits == 0 {
        return false;
    }
    let is_date = digits == DATE_DIGITS
        && rest
            .get(digits)
            .is_none_or(|after| !after.is_ascii_alphanumeric());
    !is_date
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("the price list isn't JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the price list isn't a JSON object")]
    NotAnObject,
    #[error("the price list has no usable prices")]
    NoUsableEntries,
}

/// LiteLLM's `model_prices_and_context_window.json`. Prices are per token
/// there; entries without both an input and an output price (stubs, modes
/// other than chat) are skipped, as is any entry that doesn't read, so one
/// bad entry can't sink the list.
pub fn from_litellm(body: &[u8]) -> Result<Catalog, CodecError> {
    let Value::Object(root) = serde_json::from_slice(body)? else {
        return Err(CodecError::NotAnObject);
    };
    let mut entries = HashMap::new();
    for (key, entry) in root {
        let Value::Object(entry) = entry else {
            continue;
        };
        let (Some(input), Some(output)) = (
            number(&entry, "input_cost_per_token"),
            number(&entry, "output_cost_per_token"),
        ) else {
            continue;
        };
        let cache_read = number(&entry, "cache_read_input_token_cost");
        let mut rates = ModelRates::new(
            input * 1e6,
            output * 1e6,
            number(&entry, "cache_creation_input_token_cost").unwrap_or(input) * 1e6,
            cache_read.unwrap_or(input * 0.1) * 1e6,
        );
        rates.cache_read_explicit = cache_read.is_some();
        rates.input_long =
            number(&entry, "input_cost_per_token_above_200k_tokens").map(|v| v * 1e6);
        rates.output_long =
            number(&entry, "output_cost_per_token_above_200k_tokens").map(|v| v * 1e6);
        rates.cache_write_long =
            number(&entry, "cache_creation_input_token_cost_above_200k_tokens").map(|v| v * 1e6);
        rates.cache_read_long =
            number(&entry, "cache_read_input_token_cost_above_200k_tokens").map(|v| v * 1e6);
        if let Some(Value::Object(specific)) = entry.get("provider_specific_entry") {
            if let Some(fast) = number(specific, "fast") {
                rates.fast_multiplier = fast;
            }
        }
        entries.insert(key, rates);
    }
    non_empty(entries)
}

/// models.dev's `api.json`: `{provider: {models: {id: {cost: …}}}}`, prices
/// per million. Ids are kept bare; of the same id under several providers,
/// the first provider by name wins.
pub fn from_models_dev(body: &[u8]) -> Result<Catalog, CodecError> {
    let Value::Object(root) = serde_json::from_slice(body)? else {
        return Err(CodecError::NotAnObject);
    };
    let providers: BTreeMap<String, Value> = root.into_iter().collect();
    let mut entries: HashMap<String, ModelRates> = HashMap::new();
    for provider in providers.into_values() {
        let Some(Value::Object(models)) = provider.get("models") else {
            continue;
        };
        for (id, model) in models {
            if entries.contains_key(id) {
                continue;
            }
            let Some(Value::Object(cost)) = model.get("cost") else {
                continue;
            };
            let (Some(input), Some(output)) = (number(cost, "input"), number(cost, "output"))
            else {
                continue;
            };
            let mut rates = ModelRates::new(
                input,
                output,
                number(cost, "cache_write").unwrap_or(input),
                number(cost, "cache_read").unwrap_or(input * 0.1),
            );
            rates.cache_read_explicit = cost.contains_key("cache_read");
            entries.insert(id.clone(), rates);
        }
    }
    non_empty(entries)
}

fn non_empty(entries: HashMap<String, ModelRates>) -> Result<Catalog, CodecError> {
    if entries.is_empty() {
        return Err(CodecError::NoUsableEntries);
    }
    Ok(Catalog {
        entries,
        retrieved_at: None,
    })
}

fn number(object: &Map<String, Value>, key: &str) -> Option<f64> {
    object.get(key).and_then(Value::as_f64)
}

/// The compact form: rates per million under short names, `i`nput,
/// `o`utput, `c`ache `w`rite and `c`ache `r`ead, their `a`bove-threshold
/// variants, and the `fast` multiplier. `scripts/update-pricing-snapshots.sh`
/// writes the same.
#[derive(Serialize, Deserialize)]
struct CompactCatalog {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retrieved_at: Option<String>,
    models: BTreeMap<String, CompactModel>,
}

#[derive(Serialize, Deserialize)]
struct CompactModel {
    i: f64,
    o: f64,
    cw: f64,
    cr: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ia: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    oa: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwa: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cra: Option<f64>,
    /// `false` when the cache-read rate was filled in, not published;
    /// absent means published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cre: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fast: Option<f64>,
}

pub fn from_compact(body: &[u8]) -> Result<Catalog, CodecError> {
    let file: CompactCatalog = serde_json::from_slice(body)?;
    let entries = file
        .models
        .into_iter()
        .map(|(key, model)| {
            let mut rates = ModelRates::new(model.i, model.o, model.cw, model.cr);
            rates.input_long = model.ia;
            rates.output_long = model.oa;
            rates.cache_write_long = model.cwa;
            rates.cache_read_long = model.cra;
            rates.cache_read_explicit = model.cre.unwrap_or(true);
            rates.fast_multiplier = model.fast.unwrap_or(1.0);
            (key, rates)
        })
        .collect();
    Ok(Catalog {
        entries,
        retrieved_at: file.retrieved_at,
    })
}

pub fn to_compact(catalog: &Catalog) -> Result<Vec<u8>, CodecError> {
    let models = catalog
        .entries
        .iter()
        .map(|(key, rates)| {
            let model = CompactModel {
                i: rates.input,
                o: rates.output,
                cw: rates.cache_write,
                cr: rates.cache_read,
                ia: rates.input_long,
                oa: rates.output_long,
                cwa: rates.cache_write_long,
                cra: rates.cache_read_long,
                cre: (!rates.cache_read_explicit).then_some(false),
                fast: (rates.fast_multiplier != 1.0).then_some(rates.fast_multiplier),
            };
            (key.clone(), model)
        })
        .collect();
    Ok(serde_json::to_vec(&CompactCatalog {
        retrieved_at: catalog.retrieved_at.clone(),
        models,
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(keys: &[&str]) -> Catalog {
        Catalog {
            entries: keys
                .iter()
                .enumerate()
                .map(|(index, key)| {
                    (
                        (*key).to_owned(),
                        ModelRates::new(index as f64, 1.0, 1.0, 0.1),
                    )
                })
                .collect(),
            retrieved_at: None,
        }
    }

    fn fuzzy<'a>(catalog: &'a Catalog, model: &str) -> Option<&'a str> {
        catalog.find_fuzzy(model).map(|(key, _)| key)
    }

    #[test]
    fn a_date_suffix_matches_but_a_later_version_does_not() {
        let list = catalog(&["claude-sonnet-4-20250514"]);
        assert_eq!(
            fuzzy(&list, "claude-sonnet-4"),
            Some("claude-sonnet-4-20250514")
        );
        assert_eq!(fuzzy(&list, "claude-sonnet-4-5"), None);
        let list = catalog(&["claude-sonnet-4"]);
        assert_eq!(fuzzy(&list, "claude-sonnet-4-5-20250929"), None);
        assert_eq!(
            fuzzy(&list, "claude-sonnet-4-20250514"),
            Some("claude-sonnet-4")
        );
    }

    #[test]
    fn provider_prefixes_and_separators_are_tolerated() {
        let list = catalog(&["xai/grok-4.3"]);
        assert_eq!(fuzzy(&list, "grok-4-3"), Some("xai/grok-4.3"));
        assert_eq!(fuzzy(&list, "grok-4.3"), Some("xai/grok-4.3"));
        // A letter or digit next to the match breaks it.
        assert_eq!(fuzzy(&catalog(&["gpt-5"]), "xgpt-5"), None);
        assert_eq!(fuzzy(&catalog(&["gpt-5"]), "gpt-5x"), None);
    }

    #[test]
    fn the_longest_matching_key_wins_then_byte_order() {
        let list = catalog(&["gpt-5", "openai/gpt-5", "azure/gpt-5"]);
        assert_eq!(fuzzy(&list, "gpt-5"), Some("openai/gpt-5"));
        let list = catalog(&["b/gpt-5", "a/gpt-5"]);
        assert_eq!(fuzzy(&list, "gpt-5"), Some("a/gpt-5"));
    }

    #[test]
    fn litellm_prices_per_token_read_per_million() {
        let catalog = from_litellm(
            br#"{"sample_spec":{"input_cost_per_token":"x"},
                 "claude-opus-4-8":{"input_cost_per_token":5e-6,"output_cost_per_token":2.5e-5,
                   "cache_creation_input_token_cost":6.25e-6,"cache_read_input_token_cost":5e-7,
                   "input_cost_per_token_above_200k_tokens":1e-5,"provider_specific_entry":{"fast":2}},
                 "embedder":{"input_cost_per_token":1e-7},
                 "plain":{"input_cost_per_token":1e-6,"output_cost_per_token":2e-6}}"#,
        )
        .unwrap();
        assert_eq!(catalog.entries.len(), 2);
        let opus = &catalog.entries["claude-opus-4-8"];
        assert!((opus.input - 5.0).abs() < 1e-9 && (opus.cache_read - 0.5).abs() < 1e-9);
        assert_eq!(opus.input_long.map(|v| v.round()), Some(10.0));
        assert_eq!(opus.fast_multiplier, 2.0);
        let plain = &catalog.entries["plain"];
        assert!((plain.cache_write - 1.0).abs() < 1e-9);
        assert!((plain.cache_read - 0.1).abs() < 1e-9 && !plain.cache_read_explicit);
        assert!(matches!(
            from_litellm(b"{}"),
            Err(CodecError::NoUsableEntries)
        ));
        assert!(matches!(from_litellm(b"[]"), Err(CodecError::NotAnObject)));
    }

    #[test]
    fn models_dev_keeps_the_first_provider_by_name() {
        let catalog = from_models_dev(
            br#"{"zeta":{"models":{"glm-5.2":{"cost":{"input":9,"output":9}}}},
                 "alpha":{"models":{"glm-5.2":{"cost":{"input":1,"output":2,"cache_read":0.2}},
                                    "free":{"cost":{}}}}}"#,
        )
        .unwrap();
        let glm = &catalog.entries["glm-5.2"];
        assert_eq!(
            (glm.input, glm.cache_read, glm.cache_read_explicit),
            (1.0, 0.2, true)
        );
        assert!(!catalog.entries.contains_key("free"));
    }

    #[test]
    fn the_compact_form_round_trips() {
        let mut rates = ModelRates::new(5.0, 25.0, 6.25, 0.5);
        rates.cache_read_explicit = false;
        rates.fast_multiplier = 2.0;
        rates.output_long = Some(37.5);
        let catalog = Catalog {
            entries: HashMap::from([("claude-opus-4-8".to_owned(), rates)]),
            retrieved_at: Some("2026-10-17T00:00:00Z".into()),
        };
        let back = from_compact(&to_compact(&catalog).unwrap()).unwrap();
        assert_eq!(back, catalog);
    }

    #[test]
    fn the_bundled_snapshots_read() {
        for snapshot in [
            super::super::store::BUNDLED_LITELLM,
            super::super::store::BUNDLED_MODELS_DEV,
        ] {
            assert!(from_compact(snapshot).unwrap().entries.len() > 1000);
        }
    }
}
