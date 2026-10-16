//! Using one of a Codex account's rate-limit resets, through `codex
//! app-server`'s own `account/rateLimitResetCredit/consume`: the call Codex
//! makes when you use one there.
//!
//! The panel never holds credit ids (they stay out of the usage it shows), so
//! the credit is found again by its expiry right before it is used. Each
//! credit keeps one idempotency key for the life of the app, so a retry after
//! a lost answer can't spend a second reset.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::{json, Value};

use crate::codex_rpc::{CodexRpc, CodexTransport};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResetOutcome {
    /// The limits were reset (or this credit had already been, by this key).
    Reset,
    /// Nothing was used up, so there was nothing to reset.
    NothingToReset,
    /// The credit wasn't there to use: spent or lapsed meanwhile.
    NoCredit,
    Failed,
}

/// Uses the available credit lapsing at `expires_at` (Unix seconds) for the
/// Codex home `codex_home`.
pub async fn use_reset(codex_home: &Path, expires_at: i64) -> ResetOutcome {
    match CodexRpc::launch(codex_home).await {
        Ok(mut rpc) => use_reset_with(&mut rpc, expires_at).await,
        Err(_) => ResetOutcome::Failed,
    }
}

pub async fn use_reset_with(rpc: &mut impl CodexTransport, expires_at: i64) -> ResetOutcome {
    let Ok(limits) = rpc.request("account/rateLimits/read", Value::Null).await else {
        return ResetOutcome::Failed;
    };
    let Some(credit_id) = available_credit(&limits, expires_at) else {
        return ResetOutcome::NoCredit;
    };
    let params = json!({ "creditId": credit_id, "idempotencyKey": idempotency_key(&credit_id) });
    match rpc
        .request("account/rateLimitResetCredit/consume", params)
        .await
    {
        Ok(answer) => outcome_of(&answer),
        Err(_) => ResetOutcome::Failed,
    }
}

/// The id of the available credit lapsing at `expires_at`.
fn available_credit(limits: &Value, expires_at: i64) -> Option<String> {
    limits
        .pointer("/rateLimitResetCredits/credits")?
        .as_array()?
        .iter()
        .find(|credit| {
            credit.get("status").and_then(Value::as_str) == Some("available")
                && credit.get("expiresAt").and_then(Value::as_i64) == Some(expires_at)
        })?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

/// One key per credit, kept for the life of the app.
fn idempotency_key(credit_id: &str) -> String {
    static KEYS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    KEYS.get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(credit_id.to_owned())
        .or_insert_with(|| uuid::Uuid::new_v4().to_string())
        .clone()
}

/// What the consume answer says happened. The answer names its outcome with
/// a code (`reset`, `already_redeemed`, `nothing_to_reset`, `no_credit`).
fn outcome_of(answer: &Value) -> ResetOutcome {
    let mut codes = Vec::new();
    collect_strings(answer, &mut codes);
    if codes
        .iter()
        .any(|code| *code == "reset" || *code == "already_redeemed")
    {
        ResetOutcome::Reset
    } else if codes.contains(&"nothing_to_reset") {
        ResetOutcome::NothingToReset
    } else if codes.contains(&"no_credit") {
        ResetOutcome::NoCredit
    } else {
        ResetOutcome::Failed
    }
}

fn collect_strings<'a>(value: &'a Value, into: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => into.push(text),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, into)),
        Value::Object(fields) => fields
            .values()
            .for_each(|field| collect_strings(field, into)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_rpc::CodexRpcError;
    use async_trait::async_trait;

    /// An app-server answering each method from a script, and noting calls.
    struct Scripted {
        limits: Value,
        consume: Result<Value, ()>,
        consumed: Vec<Value>,
    }

    #[async_trait]
    impl CodexTransport for Scripted {
        async fn request(&mut self, method: &str, params: Value) -> Result<Value, CodexRpcError> {
            match method {
                "account/rateLimits/read" => Ok(self.limits.clone()),
                "account/rateLimitResetCredit/consume" => {
                    self.consumed.push(params);
                    self.consume
                        .clone()
                        .map_err(|()| CodexRpcError::Io(std::io::Error::other("gone")))
                }
                other => panic!("unexpected {other}"),
            }
        }
    }

    fn limits() -> Value {
        json!({ "rateLimitResetCredits": { "availableCount": 2, "credits": [
            { "id": "c-used", "status": "used", "expiresAt": 1_792_700_817 },
            { "id": "c-1", "status": "available", "expiresAt": 1_792_700_817 },
            { "id": "c-2", "status": "available", "expiresAt": 1_793_299_680 },
        ] } })
    }

    fn server(consume: Result<Value, ()>) -> Scripted {
        Scripted {
            limits: limits(),
            consume,
            consumed: Vec::new(),
        }
    }

    #[tokio::test]
    async fn uses_the_available_credit_lapsing_then() {
        let mut rpc = server(Ok(json!({ "code": "reset" })));
        assert_eq!(
            use_reset_with(&mut rpc, 1_793_299_680).await,
            ResetOutcome::Reset
        );
        assert_eq!(rpc.consumed[0]["creditId"], "c-2");
        assert!(!rpc.consumed[0]["idempotencyKey"]
            .as_str()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_retry_reuses_the_credits_key() {
        let mut rpc = server(Err(()));
        assert_eq!(
            use_reset_with(&mut rpc, 1_792_700_817).await,
            ResetOutcome::Failed
        );
        let mut retry = server(Ok(json!({ "status": "already_redeemed" })));
        assert_eq!(
            use_reset_with(&mut retry, 1_792_700_817).await,
            ResetOutcome::Reset
        );
        assert_eq!(
            rpc.consumed[0]["idempotencyKey"],
            retry.consumed[0]["idempotencyKey"]
        );
    }

    #[tokio::test]
    async fn a_credit_gone_meanwhile_is_not_used() {
        let mut rpc = server(Ok(json!({ "code": "reset" })));
        assert_eq!(use_reset_with(&mut rpc, 42).await, ResetOutcome::NoCredit);
        assert!(rpc.consumed.is_empty());
    }

    #[test]
    fn the_answer_says_what_happened() {
        assert_eq!(
            outcome_of(&json!({ "result": { "code": "nothing_to_reset" } })),
            ResetOutcome::NothingToReset
        );
        assert_eq!(
            outcome_of(&json!({ "code": "no_credit" })),
            ResetOutcome::NoCredit
        );
        assert_eq!(outcome_of(&json!({})), ResetOutcome::Failed);
    }
}
