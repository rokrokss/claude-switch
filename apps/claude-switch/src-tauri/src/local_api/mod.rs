//! The local API: the menu bar's usage, served read-only on 127.0.0.1 for
//! other apps and scripts. Off unless Settings turns it on
//! (`settings.integrations.local_api`); it then listens for as long as
//! claude-switch runs, and stops when turned off.
//!
//! It serves what the menu bar last fetched ([`routes`]) and fetches
//! nothing itself. Loopback only, but with CORS open to any origin, so any
//! web page open on this Mac can read it: usage figures, never credentials.
//! At most [`MAX_CONNECTIONS`] are served at once; the rest get a 503.
//! A port already in use leaves the API off until it is turned on again.

pub mod limits;
pub mod routes;

use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderValue, CONTENT_TYPE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tauri::async_runtime::JoinHandle;
use tauri::AppHandle;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::tray::{self, TrayEntry};

/// The API's port on 127.0.0.1. The Settings note and the README name it too.
pub const PORT: u16 = 6741;
const MAX_CONNECTIONS: usize = 16;
/// A connection that hasn't sent its request by now is dropped, so idle ones
/// can't hold every slot.
const HEADER_TIMEOUT: Duration = Duration::from_secs(5);

/// The listener, while there is one.
static SERVER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// What the server serves: the entries as of each request.
type Entries = Arc<dyn Fn() -> Vec<TrayEntry> + Send + Sync>;

/// Starts the API, or stops it, as `enabled` says; nothing when it already is
/// as asked.
pub fn apply(app: &AppHandle, enabled: bool) {
    let app = app.clone();
    switch(enabled, PORT, Arc::new(move || tray::entries(&app)));
}

fn switch(enabled: bool, port: u16, entries: Entries) {
    let mut server = SERVER.lock().unwrap();
    if !enabled {
        if let Some(running) = server.take() {
            running.abort();
        }
        return;
    }
    if server
        .as_ref()
        .is_some_and(|running| !running.inner().is_finished())
    {
        return;
    }
    *server = Some(tauri::async_runtime::spawn(async move {
        match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
            Ok(listener) => serve(listener, entries).await,
            Err(err) => eprintln!("claude-switch: the local API could not listen on {port}: {err}"),
        }
    }));
}

async fn serve(listener: TcpListener, entries: Entries) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            // Out of file descriptors, say: let it pass rather than spin.
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let slot = Arc::clone(&slots).try_acquire_owned().ok();
        let entries = Arc::clone(&entries);
        tokio::spawn(async move {
            let busy = slot.is_none();
            let service = service_fn(move |request: Request<Incoming>| {
                let response = if busy {
                    routes::busy()
                } else {
                    let path = request
                        .uri()
                        .path_and_query()
                        .map_or("/", |path| path.as_str());
                    routes::respond(
                        request.method().as_str(),
                        path,
                        &entries(),
                        chrono::Utc::now(),
                    )
                };
                async move { Ok::<_, Infallible>(http_response(response)) }
            });
            let _ = http1::Builder::new()
                .keep_alive(false)
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service)
                .await;
            drop(slot);
        });
    }
}

fn http_response(response: routes::Response) -> hyper::Response<Full<Bytes>> {
    let has_body = response.body.is_some();
    let mut http = hyper::Response::new(Full::new(Bytes::from(response.body.unwrap_or_default())));
    *http.status_mut() = StatusCode::from_u16(response.status).unwrap_or(StatusCode::OK);
    let headers = http.headers_mut();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, OPTIONS"),
    );
    headers.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("Content-Type"),
    );
    if has_body {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    http
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_kind::AppKind;
    use limits::tests::entry;
    use tokio::net::TcpStream;

    fn entries() -> Entries {
        Arc::new(|| vec![entry("work", AppKind::Claude, Some("2026-10-17T12:00:00Z"))])
    }

    /// A server on a free port of its own, and its address.
    async fn server() -> String {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(serve(listener, entries()));
        address
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn serves_json_with_open_cors() {
        let address = server().await;
        let response = client()
            .get(format!("{address}/v1/limits/claude"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let headers = response.headers();
        assert_eq!(headers["access-control-allow-origin"], "*");
        assert_eq!(headers["access-control-allow-methods"], "GET, OPTIONS");
        assert_eq!(headers["content-type"], "application/json");
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["schema"], limits::SCHEMA);
        assert!(body["profiles"]["work"].is_object());
    }

    #[tokio::test]
    async fn answers_preflights_errors_and_other_methods() {
        let address = server().await;
        let preflight = client()
            .request(reqwest::Method::OPTIONS, format!("{address}/v1/limits"))
            .send()
            .await
            .unwrap();
        assert_eq!(preflight.status(), 204);
        assert_eq!(preflight.headers()["access-control-allow-origin"], "*");

        let post = client()
            .post(format!("{address}/v1/limits"))
            .send()
            .await
            .unwrap();
        assert_eq!(post.status(), 405);

        let missing = client()
            .get(format!("{address}/v1/limits/nobody"))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);
        assert_eq!(missing.headers()["access-control-allow-origin"], "*");
        assert_eq!(
            missing.text().await.unwrap(),
            r#"{"error":"profile_not_found"}"#
        );
    }

    #[tokio::test]
    async fn a_seventeenth_connection_is_turned_away() {
        let address = server().await;
        let host = address.trim_start_matches("http://");
        // Sixteen connections that send nothing hold every slot.
        let mut held = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            held.push(TcpStream::connect(host).await.unwrap());
        }
        let busy = client()
            .get(format!("{address}/v1/limits"))
            .send()
            .await
            .unwrap();
        assert_eq!(busy.status(), 503);
        assert_eq!(busy.text().await.unwrap(), r#"{"error":"server_busy"}"#);
        drop(held);
    }

    /// Whether something accepts connections on `port`, waiting a little for
    /// the answer to become `expected`.
    fn listening(port: u16, expected: bool) -> bool {
        for _ in 0..50 {
            let open = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok();
            if open == expected {
                return open;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        !expected
    }

    #[test]
    fn switching_on_listens_and_switching_off_stops() {
        let port = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        switch(true, port, entries());
        assert!(listening(port, true));
        // Already on: nothing changes.
        switch(true, port, entries());
        assert!(listening(port, true));
        switch(false, port, entries());
        assert!(!listening(port, false));
        // On again after being off.
        switch(true, port, entries());
        assert!(listening(port, true));
        switch(false, port, entries());
        assert!(!listening(port, false));
    }
}
