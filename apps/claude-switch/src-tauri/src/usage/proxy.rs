//! An optional proxy for the requests the usage code makes itself, set in
//! `<app data>/config.json`:
//!
//! ```json
//! { "proxy": { "enabled": true, "url": "socks5://127.0.0.1:1080" } }
//! ```
//!
//! `socks5`, `http` and `https` proxies, with credentials in the URL when they
//! need them (`http://user:pass@host:8080`). Without a port, the scheme's
//! default applies (1080, 80, 443). A SOCKS proxy resolves the hostnames too.
//! The file is read once per process, so a change takes effect at the next
//! launch. A missing, disabled or invalid file means no proxy, and loopback
//! addresses never go through one.
//!
//! The file's format follows OpenUsage's (MIT, <https://github.com/robinebers/openusage>).

use std::fs;
use std::sync::LazyLock;

use reqwest::{ClientBuilder, NoProxy, Proxy, Url};
use serde::Deserialize;

use crate::paths::app_data_dir;

static PROXY: LazyLock<Option<Proxy>> = LazyLock::new(load);

/// A client builder that sends its requests through the configured proxy,
/// if there is one.
pub fn client_builder() -> ClientBuilder {
    let builder = reqwest::Client::builder();
    match PROXY.as_ref() {
        Some(proxy) => builder.proxy(proxy.clone()),
        None => builder,
    }
}

fn load() -> Option<Proxy> {
    let path = app_data_dir().ok()?.join("config.json");
    let text = fs::read_to_string(&path).ok()?;
    match configured(&text) {
        Ok(proxy) => proxy,
        Err(reason) => {
            eprintln!(
                "claude-switch: ignoring the proxy in {}: {reason}",
                path.display()
            );
            None
        }
    }
}

#[derive(Deserialize)]
struct Config {
    proxy: Option<ProxyConfig>,
}

#[derive(Deserialize)]
struct ProxyConfig {
    #[serde(default)]
    enabled: bool,
    url: Option<String>,
}

/// The proxy `config` (the file's text) turns on: none when it turns none
/// on, an error when the one it turns on can't be used.
fn configured(config: &str) -> Result<Option<Proxy>, String> {
    let Ok(config) = serde_json::from_str::<Config>(config) else {
        return Err("not valid JSON of the expected shape".to_owned());
    };
    let Some(proxy) = config.proxy.filter(|proxy| proxy.enabled) else {
        return Ok(None);
    };
    let url = proxy.url.ok_or("no url")?;
    let url = proxy_url(&url).ok_or("the url isn't a socks5, http or https URL with a host")?;
    let proxy = Proxy::all(url.as_str()).map_err(|err| err.to_string())?;
    Ok(Some(
        proxy.no_proxy(NoProxy::from_string("localhost,127.0.0.1,::1")),
    ))
}

/// `url` as reqwest takes it: its scheme's default port filled in, and a
/// SOCKS proxy asked to resolve hostnames itself (`socks5h`).
fn proxy_url(url: &str) -> Option<Url> {
    let parsed = Url::parse(url.trim()).ok()?;
    let (scheme, default_port) = match parsed.scheme() {
        "socks5" => ("socks5h", 1080),
        "http" => ("http", 80),
        "https" => ("https", 443),
        _ => return None,
    };
    let host = parsed.host_str().filter(|host| !host.is_empty())?;
    let port = parsed.port().unwrap_or(default_port);
    let mut normalized = Url::parse(&format!("{scheme}://{host}:{port}")).ok()?;
    normalized.set_username(parsed.username()).ok()?;
    normalized.set_password(parsed.password()).ok()?;
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn config(url: &str) -> String {
        serde_json::json!({ "proxy": { "enabled": true, "url": url } }).to_string()
    }

    #[test]
    fn urls_get_their_scheme_s_default_port() {
        let cases = [
            ("socks5://127.0.0.1", "socks5h://127.0.0.1:1080"),
            ("socks5://127.0.0.1:10808", "socks5h://127.0.0.1:10808"),
            ("http://proxy.example.com", "http://proxy.example.com/"),
            (
                "http://proxy.example.com:8080",
                "http://proxy.example.com:8080/",
            ),
            ("https://proxy.example.com", "https://proxy.example.com/"),
            (
                " http://user:p%40ss@proxy.example.com:8080 ",
                "http://user:p%40ss@proxy.example.com:8080/",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                proxy_url(input).map(String::from).as_deref(),
                Some(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn other_schemes_and_hostless_urls_are_refused() {
        for input in [
            "ftp://proxy:21",
            "socks4://proxy:1080",
            "proxy:8080",
            "http://",
            "",
        ] {
            assert!(proxy_url(input).is_none(), "{input}");
        }
    }

    #[test]
    fn only_an_enabled_proxy_with_a_usable_url_is_used() {
        assert!(configured(&config("socks5://127.0.0.1:1080"))
            .unwrap()
            .is_some());
        for off in [
            r#"{}"#,
            r#"{"proxy":null}"#,
            r#"{"proxy":{"url":"http://proxy:8080"}}"#,
            r#"{"proxy":{"enabled":false,"url":"http://proxy:8080"}}"#,
        ] {
            assert!(configured(off).unwrap().is_none(), "{off}");
        }
        for invalid in [
            "not json",
            r#"{"proxy":{"enabled":"yes","url":"http://proxy:8080"}}"#,
            r#"{"proxy":{"enabled":true}}"#,
            &config("ftp://proxy:21"),
        ] {
            assert!(configured(invalid).is_err(), "{invalid}");
        }
    }

    /// Sends one GET to `url` through `proxy`, on a runtime of its own.
    fn get_through(proxy: Proxy, url: &str) {
        let client = reqwest::Client::builder().proxy(proxy).build().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = runtime.block_on(client.get(url).send());
    }

    /// The head of the first request `listener` receives, answered with a 200.
    fn first_request(listener: TcpListener) -> thread::JoinHandle<String> {
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
            head
        })
    }

    #[test]
    fn an_http_proxy_gets_the_request_with_its_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let head = first_request(listener);
        let proxy = configured(&config(&format!("http://user:pass@127.0.0.1:{port}")))
            .unwrap()
            .unwrap();
        get_through(proxy, "http://usage.example.com/api");
        let head = head.join().unwrap();
        assert!(
            head.starts_with("GET http://usage.example.com/api HTTP/1.1"),
            "{head}"
        );
        // base64("user:pass")
        assert!(
            head.to_lowercase()
                .contains("proxy-authorization: basic dxnlcjpwyxnz"),
            "{head}"
        );
    }

    #[test]
    fn a_socks_proxy_is_offered_its_credentials_and_resolves_the_host() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut greeting = [0u8; 2];
            stream.read_exact(&mut greeting).unwrap();
            let mut methods = vec![0u8; usize::from(greeting[1])];
            stream.read_exact(&mut methods).unwrap();
            // Username and password.
            stream.write_all(&[5, 2]).unwrap();
            let mut version_and_length = [0u8; 2];
            stream.read_exact(&mut version_and_length).unwrap();
            let mut user = vec![0u8; usize::from(version_and_length[1])];
            stream.read_exact(&mut user).unwrap();
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).unwrap();
            let mut pass = vec![0u8; usize::from(length[0])];
            stream.read_exact(&mut pass).unwrap();
            stream.write_all(&[1, 0]).unwrap();
            // The connect request: version, command, reserved, address type.
            let mut request = [0u8; 4];
            stream.read_exact(&mut request).unwrap();
            (methods, user, pass, request[3])
        });
        let proxy = configured(&config(&format!("socks5://user:pass@127.0.0.1:{port}")))
            .unwrap()
            .unwrap();
        get_through(proxy, "http://usage.example.com/api");
        let (methods, user, pass, address_type) = server.join().unwrap();
        assert!(methods.contains(&2), "{methods:?}");
        assert_eq!(user, b"user");
        assert_eq!(pass, b"pass");
        assert_eq!(address_type, 3, "a domain name, for the proxy to resolve");
    }

    #[test]
    fn loopback_addresses_bypass_the_proxy() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_port = target.local_addr().unwrap().port();
        let head = first_request(target);
        // Nothing listens here: the request only succeeds by going direct.
        let proxy = configured(&config("http://127.0.0.1:9")).unwrap().unwrap();
        get_through(proxy, &format!("http://127.0.0.1:{target_port}/v1/limits"));
        assert!(head.join().unwrap().starts_with("GET /v1/limits HTTP/1.1"));
    }
}
