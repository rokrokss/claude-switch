//! What the local API answers to a request: pure, so every route is tested
//! without a socket; the server ([`super`]) only carries bytes.
//!
//! - `GET /v1/limits`, `GET /v1/limits/:id`: [`limits::limits`].
//! - `GET /v1/usage`, `GET /v1/usage/:id`: [`limits::usage`].
//!
//! `:id` is a profile's exact id (`default:claude` for a stock install), or
//! an app (`claude`, `codex`) for every profile of it. Errors are
//! `{"error": "<code>"}`. The routes follow OpenUsage's local API
//! (MIT, <https://github.com/robinebers/openusage>).

use chrono::{DateTime, Utc};

use super::limits;
use crate::tray::TrayEntry;

/// One of the two bodies the routes serve.
type Render = fn(&[TrayEntry], DateTime<Utc>) -> String;

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// JSON; none for a preflight.
    pub body: Option<String>,
}

/// The answer to `method` `path` (query and all), from `entries` as of `now`.
pub fn respond(method: &str, path: &str, entries: &[TrayEntry], now: DateTime<Utc>) -> Response {
    // A browser's preflight, wherever it goes: the server sends the CORS
    // headers with every response.
    if method == "OPTIONS" {
        return Response {
            status: 204,
            body: None,
        };
    }
    let path = path.split('?').next().unwrap_or_default();
    let segments: Vec<String> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_decoded)
        .collect();
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    let (route, token): (Render, Option<&str>) = match segments.as_slice() {
        ["v1", "limits"] => (limits::limits, None),
        ["v1", "limits", token] => (limits::limits, Some(token)),
        ["v1", "usage"] => (limits::usage, None),
        ["v1", "usage", token] => (limits::usage, Some(token)),
        _ => return error(404, "not_found"),
    };
    if method != "GET" {
        return error(405, "method_not_allowed");
    }
    let body = match token {
        None => route(entries, now),
        Some(token) => {
            let named: Vec<TrayEntry> = entries
                .iter()
                .filter(|entry| entry.id == token || entry.app.as_str() == token)
                .cloned()
                .collect();
            if named.is_empty() {
                return error(404, "profile_not_found");
            }
            route(&named, now)
        }
    };
    Response {
        status: 200,
        body: Some(body),
    }
}

/// The answer while the server is already serving all it will at once.
pub fn busy() -> Response {
    error(503, "server_busy")
}

fn error(status: u16, code: &str) -> Response {
    Response {
        status,
        body: Some(serde_json::json!({ "error": code }).to_string()),
    }
}

/// `segment` with its `%XX` escapes decoded, so `default%3Aclaude` names the
/// stock install as `default:claude` does.
fn percent_decoded(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| segment.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escape {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_kind::AppKind;
    use crate::local_api::limits::tests::entry;
    use serde_json::Value;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-17T12:01:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn entries() -> Vec<TrayEntry> {
        vec![
            entry(
                "default:claude",
                AppKind::Claude,
                Some("2026-10-17T12:00:00Z"),
            ),
            entry("work", AppKind::Claude, Some("2026-10-17T12:00:00Z")),
            entry("chat", AppKind::Codex, Some("2026-10-17T12:00:00Z")),
        ]
    }

    fn get(path: &str) -> Response {
        respond("GET", path, &entries(), now())
    }

    fn body(response: &Response) -> Value {
        serde_json::from_str(response.body.as_deref().unwrap()).unwrap()
    }

    fn profile_ids(response: &Response) -> Vec<String> {
        body(response)["profiles"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    fn usage_ids(response: &Response) -> Vec<String> {
        body(response)
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["id"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn limits_lists_every_profile() {
        let response = get("/v1/limits");
        assert_eq!(response.status, 200);
        assert_eq!(body(&response)["schema"], "claude-switch.limits.v1");
        assert_eq!(profile_ids(&response), ["default:claude", "work", "chat"]);
    }

    #[test]
    fn an_id_names_one_profile_and_an_app_every_profile_of_it() {
        assert_eq!(profile_ids(&get("/v1/limits/work")), ["work"]);
        assert_eq!(
            profile_ids(&get("/v1/limits/claude")),
            ["default:claude", "work"]
        );
        assert_eq!(profile_ids(&get("/v1/limits/codex")), ["chat"]);
        assert_eq!(
            profile_ids(&get("/v1/limits/default:claude")),
            ["default:claude"]
        );
        assert_eq!(
            profile_ids(&get("/v1/limits/default%3Aclaude")),
            ["default:claude"]
        );
    }

    #[test]
    fn usage_routes_answer_with_an_array() {
        assert_eq!(
            usage_ids(&get("/v1/usage")),
            ["default:claude", "work", "chat"]
        );
        assert_eq!(usage_ids(&get("/v1/usage/codex")), ["chat"]);
        assert_eq!(usage_ids(&get("/v1/usage/work")), ["work"]);
    }

    #[test]
    fn a_query_and_a_trailing_slash_change_nothing() {
        assert_eq!(profile_ids(&get("/v1/limits/?pretty=1")).len(), 3);
        assert_eq!(profile_ids(&get("/v1/limits/work?x=y")), ["work"]);
    }

    #[test]
    fn an_unknown_profile_or_route_is_404() {
        let unknown = get("/v1/limits/nobody");
        assert_eq!(unknown.status, 404);
        assert_eq!(
            body(&unknown),
            serde_json::json!({ "error": "profile_not_found" })
        );
        assert_eq!(get("/v1/usage/Claude").status, 404, "matching is exact");
        for path in [
            "/",
            "/v1",
            "/v2/limits",
            "/v1/limits/work/extra",
            "/v1/other",
        ] {
            let response = get(path);
            assert_eq!(response.status, 404, "{path}");
            assert_eq!(body(&response)["error"], "not_found");
        }
    }

    #[test]
    fn options_is_a_bare_204_anywhere() {
        for path in ["/v1/limits", "/anything"] {
            assert_eq!(
                respond("OPTIONS", path, &entries(), now()),
                Response {
                    status: 204,
                    body: None
                }
            );
        }
    }

    #[test]
    fn other_methods_are_405() {
        for method in ["POST", "PUT", "DELETE", "HEAD", "PATCH"] {
            let response = respond(method, "/v1/limits", &entries(), now());
            assert_eq!(response.status, 405, "{method}");
            assert_eq!(body(&response)["error"], "method_not_allowed");
        }
    }

    #[test]
    fn busy_is_503() {
        let response = busy();
        assert_eq!(response.status, 503);
        assert_eq!(body(&response)["error"], "server_busy");
    }

    #[test]
    fn nothing_fetched_yet_is_an_empty_answer_not_an_error() {
        let response = respond("GET", "/v1/limits", &[], now());
        assert_eq!(response.status, 200);
        assert!(profile_ids(&response).is_empty());
        assert_eq!(
            respond("GET", "/v1/usage", &[], now()).body.as_deref(),
            Some("[]")
        );
    }

    #[test]
    fn a_malformed_escape_is_kept_as_written() {
        assert_eq!(percent_decoded("a%zzb%4"), "a%zzb%4");
        assert_eq!(percent_decoded("%41%42"), "AB");
    }
}
