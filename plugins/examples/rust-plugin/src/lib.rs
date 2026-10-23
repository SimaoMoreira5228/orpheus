wit_bindgen::generate!({
	path: "../../wit/plugin.wit",
	world: "orpheus-plugin",
});

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

static CONFIGURED_NAME: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
	pub id: String,
	pub title: String,
	pub artist: String,
	pub album: Option<String>,
	#[serde(rename = "durationMs")]
	pub duration_ms: u64,
	pub source: String,
	pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
	pub name: String,
	pub version: String,
	pub description: String,
	pub plugin_type: String,
	pub api_version: String,
	pub provides: Vec<String>,
	pub routes: Vec<RouteDecl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecl {
	pub method: String,
	pub path: String,
}

pub struct Plugin;

fn display_name() -> String {
	CONFIGURED_NAME
		.lock()
		.ok()
		.and_then(|guard| guard.clone())
		.unwrap_or_else(|| "example-plugin".to_string())
}

impl Guest for Plugin {
	fn init() {}

	fn shutdown() {}

	fn resolve(query: String) -> String {
		let track = resolve_track(&query);
		serde_json::to_string(&track).unwrap_or_default()
	}

	fn filter(_frame_ptr: u32, _frame_len: u32) {}

	fn track_info() -> String {
		let info = PluginInfo {
			name: "example-plugin".to_string(),
			version: "0.1.0".to_string(),
			description: "Example Orpheus plugin".to_string(),
			plugin_type: "track-resolver".to_string(),
			// NOTE: must match the package version in plugins/wit/plugin.wit; the
			// host refuses to load guests that report anything else.
			api_version: "0.2.0".to_string(),
			provides: vec!["example:".to_string()],
			routes: vec![RouteDecl {
				method: "GET".to_string(),
				path: "/hello".to_string(),
			}],
		};
		serde_json::to_string(&info).unwrap_or_default()
	}

	fn configure(json: String) {
		let name = serde_json::from_str::<serde_json::Value>(&json)
			.ok()
			.and_then(|value| value.get("name").and_then(|name| name.as_str()).map(|name| name.to_string()));

		if let Ok(mut guard) = CONFIGURED_NAME.lock() {
			*guard = name;
		}
	}

	fn handle_route(
		method: String,
		path: String,
		_query: String,
		_headers_json: String,
		_body_b64: String,
	) -> Result<String, String> {
		if method == "GET" && path == "/hello" {
			let payload = serde_json::json!({ "hello": display_name() });
			let body = serde_json::to_string(&payload).unwrap_or_default();
			let response = serde_json::json!({
				"status": 200,
				"headers": [["content-type", "application/json"]],
				"body_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, body.as_bytes()),
			});
			return serde_json::to_string(&response).map_err(|e| e.to_string());
		}

		Err(format!("no route {method} {path}"))
	}
}

export!(Plugin);

fn resolve_track(query: &str) -> TrackInfo {
	TrackInfo {
		id: "example-001".to_string(),
		title: format!("Search result for: {}", query),
		artist: "Example Artist".to_string(),
		album: Some("Example Album".to_string()),
		duration_ms: 180000,
		source: "example".to_string(),
		url: "https://example.com/audio.mp3".to_string(),
	}
}
