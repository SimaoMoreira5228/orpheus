use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

const MAX_FETCH_BODY_BYTES: usize = 1024 * 1024;

pub(crate) struct FetchCtx {
	plugin_name: String,
	configs: Arc<RwLock<HashMap<String, serde_json::Value>>>,
	client: reqwest::Client,
	timeout_ms: u64,
}

impl FetchCtx {
	pub(crate) fn new(
		plugin_name: String,
		configs: Arc<RwLock<HashMap<String, serde_json::Value>>>,
		client: reqwest::Client,
		timeout_ms: u64,
	) -> Self {
		Self {
			plugin_name,
			configs,
			client,
			timeout_ms,
		}
	}

	async fn allow_hosts(&self) -> Vec<String> {
		self.configs
			.read()
			.await
			.get(&self.plugin_name)
			.and_then(|config| config.get("allow_hosts"))
			.and_then(|hosts| serde_json::from_value::<Vec<String>>(hosts.clone()).ok())
			.unwrap_or_default()
	}
}

#[derive(Debug, serde::Deserialize)]
struct FetchOpts {
	#[serde(default)]
	method: Option<String>,
	#[serde(default)]
	headers: Option<Vec<(String, String)>>,
	#[serde(default)]
	body_b64: Option<String>,
	#[serde(default)]
	timeout_ms: Option<u64>,
}

fn host_allowed(host: &str, allow_hosts: &[String]) -> bool {
	let host = host.to_ascii_lowercase();

	allow_hosts.iter().any(|entry| {
		let entry = entry.trim().trim_start_matches('.').to_ascii_lowercase();

		!entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
	})
}

pub(crate) async fn fetch_inner(ctx: FetchCtx, url: &str, opts: &str) -> Result<String, String> {
	let opts: FetchOpts = serde_json::from_str(opts).map_err(|e| format!("invalid fetch opts JSON: {e}"))?;

	let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid fetch url: {e}"))?;

	if !matches!(parsed.scheme(), "http" | "https") {
		return Err(format!("fetch scheme not allowed: {}", parsed.scheme()));
	}

	let host = parsed.host_str().unwrap_or_default().to_string();
	let allowed = ctx.allow_hosts().await;

	if !host_allowed(&host, &allowed) {
		return Err(format!("fetch denied: host {host} is not in allow_hosts"));
	}

	let method_name = opts.method.as_deref().unwrap_or("GET").to_ascii_uppercase();
	let method: reqwest::Method = method_name
		.parse()
		.map_err(|_| format!("invalid fetch method: {method_name}"))?;

	let budget_ms = match opts.timeout_ms {
		Some(t) if t > 0 => ctx.timeout_ms.min(t),
		_ => ctx.timeout_ms,
	};

	let mut request = ctx.client.request(method, parsed).timeout(Duration::from_millis(budget_ms));

	for (name, value) in opts.headers.unwrap_or_default() {
		let parsed_value =
			reqwest::header::HeaderValue::from_str(&value).map_err(|_| format!("invalid fetch header value for {name}"))?;

		request = request.header(name, parsed_value);
	}

	if let Some(body_b64) = opts.body_b64 {
		let body = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &body_b64)
			.map_err(|e| format!("invalid fetch body_b64: {e}"))?;

		request = request.body(body);
	}

	let response = request.send().await.map_err(|e| format!("fetch failed: {e}"))?;
	let status = response.status().as_u16();

	let headers: Vec<(String, String)> = response
		.headers()
		.iter()
		.map(|(name, value)| (name.to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned()))
		.collect();

	let mut response = response;
	let mut body = Vec::new();

	while let Some(chunk) = response.chunk().await.map_err(|e| format!("fetch body failed: {e}"))? {
		body.extend_from_slice(&chunk);

		if body.len() > MAX_FETCH_BODY_BYTES {
			return Err(format!("fetch response exceeds {} byte cap", MAX_FETCH_BODY_BYTES));
		}
	}

	let result = serde_json::json!({
		"status": status,
		"headers": headers,
		"body_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &body),
	});

	serde_json::to_string(&result).map_err(|e| format!("fetch result encode failed: {e}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fetch_ctx(config: serde_json::Value) -> FetchCtx {
		let mut configs = HashMap::new();

		configs.insert("example".to_string(), config);

		FetchCtx::new(
			"example".to_string(),
			Arc::new(RwLock::new(configs)),
			reqwest::Client::new(),
			5000,
		)
	}

	fn spawn_http_server(body: &str) -> (std::net::SocketAddr, Arc<std::sync::atomic::AtomicUsize>) {
		use std::io::{Read, Write};
		use std::sync::atomic::{AtomicUsize, Ordering};

		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
		let addr = listener.local_addr().expect("local addr");
		let hits = Arc::new(AtomicUsize::new(0));
		let served = hits.clone();
		let payload = body.to_string();

		std::thread::spawn(move || {
			for _ in 0..4 {
				let Ok((mut stream, _)) = listener.accept() else {
					break;
				};

				served.fetch_add(1, Ordering::SeqCst);

				let mut request = [0u8; 4096];
				let _ = stream.read(&mut request);

				let response = format!(
					"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
					payload.len(),
					payload
				);

				let _ = stream.write_all(response.as_bytes());
			}
		});

		(addr, hits)
	}

	#[tokio::test]
	async fn fetch_allowed_host_returns_body() {
		let (addr, hits) = spawn_http_server("hello-bytes");
		let ctx = fetch_ctx(serde_json::json!({ "allow_hosts": ["127.0.0.1"] }));

		let raw = fetch_inner(ctx, &format!("http://{addr}/data"), "{}")
			.await
			.expect("allowed fetch works");

		let value: serde_json::Value = serde_json::from_str(&raw).expect("fetch result is JSON");

		assert_eq!(value["status"], 200);

		let body = String::from_utf8(
			base64::Engine::decode(
				&base64::engine::general_purpose::STANDARD,
				value["body_b64"].as_str().unwrap_or_default(),
			)
			.expect("body decodes"),
		)
		.expect("body is utf-8");

		assert_eq!(body, "hello-bytes");
		assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
	}

	#[tokio::test]
	async fn fetch_denied_host_errors_without_sending() {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
		listener.set_nonblocking(true).expect("nonblocking listener");
		let addr = listener.local_addr().expect("local addr");

		let ctx = fetch_ctx(serde_json::json!({}));

		let err = fetch_inner(ctx, &format!("http://{addr}/secret"), "{}")
			.await
			.expect_err("empty allow_hosts denies everything");

		assert!(err.contains("denied"), "unexpected error: {err}");
		assert!(listener.accept().is_err(), "denied fetch must not connect");
	}

	#[test]
	fn host_suffix_match_covers_subdomains_only() {
		assert!(host_allowed("example.com", &["example.com".to_string()]));
		assert!(host_allowed("a.example.com", &["example.com".to_string()]));
		assert!(host_allowed("A.EXAMPLE.COM", &["example.com".to_string()]));
		assert!(!host_allowed("example.com.evil.com", &["example.com".to_string()]));
		assert!(!host_allowed("notexample.com", &["example.com".to_string()]));
		assert!(!host_allowed("example.com", &[]));
		assert!(!host_allowed("example.com", &["".to_string()]));
	}
}
