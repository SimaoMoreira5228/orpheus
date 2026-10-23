use std::sync::Arc;

use crate::{LoadResult, LoadType, ResolverError, Result, SourceManager, TrackInfo, new_track};

pub struct PluginSourceManager {
	manager: Arc<tokio::sync::RwLock<orpheus_plugins::PluginManager>>,
}

impl PluginSourceManager {
	pub fn new(manager: Arc<tokio::sync::RwLock<orpheus_plugins::PluginManager>>) -> Self {
		Self { manager }
	}

	async fn matching_plugin(&self, identifier: &str) -> Option<String> {
		let manager = self.manager.read().await;

		find_route(&manager.track_resolver_plugins(), identifier)
	}

	async fn resolve_raw(&self, plugin: &str, identifier: &str) -> Result<String> {
		self.manager
			.write()
			.await
			.resolve_track(plugin, identifier)
			.await
			.map_err(|e| ResolverError::FetchError(e.to_string()))
	}
}

fn find_route(resolvers: &[(String, orpheus_plugins::LoadedPlugin)], identifier: &str) -> Option<String> {
	let query = identifier.to_ascii_lowercase();

	for (name, plugin) in resolvers {
		if plugin.provides.iter().any(|prefix| {
			let prefix = prefix.to_ascii_lowercase();

			!prefix.is_empty() && query.starts_with(prefix.as_str())
		}) {
			return Some(name.clone());
		}
	}

	None
}

fn guest_track_to_info(plugin: &str, identifier: &str, raw: &str) -> Result<TrackInfo> {
	let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| ResolverError::ParseError(e.to_string()))?;

	let title = value
		.get("title")
		.and_then(|v| v.as_str())
		.filter(|s| !s.is_empty())
		.unwrap_or(identifier)
		.to_string();

	let author = value.get("artist").and_then(|v| v.as_str()).unwrap_or_default().to_string();

	let length = value
		.get("durationMs")
		.or_else(|| value.get("duration_ms"))
		.and_then(|v| v.as_u64())
		.unwrap_or(0);

	let uri = value
		.get("url")
		.and_then(|v| v.as_str())
		.filter(|s| !s.is_empty())
		.map(|s| s.to_string());

	Ok(TrackInfo {
		identifier: identifier.to_string(),
		is_seekable: true,
		author,
		length,
		is_stream: false,
		position: 0,
		title,
		uri,
		artwork_url: None,
		isrc: None,
		source_name: plugin.to_string(),
	})
}

impl SourceManager for PluginSourceManager {
	fn name(&self) -> &'static str {
		"plugin"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		let Ok(manager) = self.manager.try_read() else {
			return false;
		};

		find_route(&manager.track_resolver_plugins(), identifier).is_some()
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		let plugin = self
			.matching_plugin(identifier)
			.await
			.ok_or_else(|| ResolverError::UnsupportedSource(format!("no plugin provides {identifier}")))?;

		let raw = self.resolve_raw(&plugin, identifier).await?;

		let info = guest_track_to_info(&plugin, identifier, &raw)?;

		Ok(LoadResult {
			load_type: LoadType::Track,
			tracks: vec![new_track(info)],
			playlist_info: None,
		})
	}
}

#[cfg(test)]
mod tests {
	use orpheus_plugins::{LoadedPlugin, PluginType};

	use super::*;

	fn resolver(name: &str, provides: &[&str]) -> (String, LoadedPlugin) {
		(
			name.to_string(),
			LoadedPlugin {
				name: name.to_string(),
				version: "0.1.0".to_string(),
				path: None,
				plugin_type: PluginType::TrackResolver,
				provides: provides.iter().map(|p| p.to_string()).collect(),
				routes: Vec::new(),
			},
		)
	}

	fn routes() -> Vec<(String, LoadedPlugin)> {
		vec![resolver("example", &["example:"])]
	}

	#[test]
	fn declared_prefix_matches() {
		assert_eq!(
			find_route(&routes(), "example:never gonna give you up"),
			Some("example".to_string())
		);
	}

	#[test]
	fn prefix_match_is_case_insensitive() {
		assert_eq!(find_route(&routes(), "EXAMPLE:query"), Some("example".to_string()));
	}

	#[test]
	fn first_declared_prefix_wins() {
		let resolvers = vec![resolver("one", &["ex:"]), resolver("two", &["ex:"])];

		assert_eq!(find_route(&resolvers, "ex:query"), Some("one".to_string()));
	}

	#[test]
	fn bare_query_matches_nothing() {
		assert_eq!(find_route(&routes(), "never gonna give you up"), None);
	}

	#[test]
	fn plain_url_matches_nothing() {
		assert_eq!(find_route(&routes(), "https://example.com/audio.mp3"), None);
		assert_eq!(find_route(&routes(), "https://www.youtube.com/watch?v=dQw4w9WgXcQ"), None);
	}

	#[test]
	fn unknown_prefix_matches_nothing() {
		assert_eq!(find_route(&routes(), "spotify:track:abc"), None);
	}

	#[test]
	fn empty_provides_never_match() {
		let resolvers = vec![resolver("vacuous", &[]), resolver("blank", &[""])];

		assert_eq!(find_route(&resolvers, "example:query"), None);
		assert_eq!(find_route(&resolvers, "anything at all"), None);
		assert_eq!(find_route(&resolvers, ""), None);
	}

	#[test]
	fn guest_json_maps_all_fields() {
		let raw =
			r#"{"id":"example-001","title":"Song","artist":"Artist","durationMs":180000,"url":"https://example.com/a.mp3"}"#;

		let info = guest_track_to_info("example", "example:Song", raw).expect("valid guest JSON");

		assert_eq!(info.identifier, "example:Song");
		assert_eq!(info.title, "Song");
		assert_eq!(info.author, "Artist");
		assert_eq!(info.length, 180000);
		assert_eq!(info.uri.as_deref(), Some("https://example.com/a.mp3"));
		assert_eq!(info.source_name, "example");
		assert!(info.is_seekable);
		assert!(!info.is_stream);
	}

	#[test]
	fn guest_json_accepts_snake_case_duration() {
		let raw = r#"{"title":"Song","artist":"Artist","duration_ms":210000}"#;

		let info = guest_track_to_info("example", "example:Song", raw).expect("valid guest JSON");

		assert_eq!(info.length, 210000);
	}

	#[test]
	fn guest_json_missing_fields_fall_back() {
		let info = guest_track_to_info("example", "example:query", "{}").expect("empty object parses");

		assert_eq!(info.identifier, "example:query");
		assert_eq!(info.title, "example:query");
		assert_eq!(info.author, "");
		assert_eq!(info.length, 0);
		assert_eq!(info.uri, None);
	}

	#[test]
	fn guest_json_rejects_invalid_json() {
		assert!(guest_track_to_info("example", "example:query", "not json").is_err());
	}
}
