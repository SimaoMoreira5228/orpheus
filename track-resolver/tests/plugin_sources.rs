use std::sync::Arc;

use orpheus_plugins::PluginManager;
use orpheus_track_resolver::{PluginSourceManager, SourceManager, TrackResolver, decode_track};

const GUEST_BYTES: &[u8] = include_bytes!("../../plugins/tests/fixtures/example-plugin.wasm");

async fn manager_with_example() -> Arc<tokio::sync::RwLock<PluginManager>> {
	let manager = Arc::new(tokio::sync::RwLock::new(
		PluginManager::new().await.expect("manager builds without wasm"),
	));

	manager
		.write()
		.await
		.load("example", GUEST_BYTES)
		.await
		.expect("fixture guest loads");

	manager
}

#[tokio::test]
async fn plugin_routes_declared_prefix() {
	let source = PluginSourceManager::new(manager_with_example().await);

	assert!(source.can_handle("example:never gonna give you up"));
}

#[tokio::test]
async fn plugin_ignores_bare_queries_and_urls() {
	let source = PluginSourceManager::new(manager_with_example().await);

	assert!(!source.can_handle("never gonna give you up"));
	assert!(!source.can_handle("https://example.com/audio.mp3"));
	assert!(!source.can_handle("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
}

#[tokio::test]
async fn end_to_end_resolve_and_encode() {
	let client = Arc::new(libytdlp::Client::new("{}").expect("test client"));
	let mut resolver = TrackResolver::from_client(client);

	resolver.register_plugin_source(manager_with_example().await);

	let (result, source) = resolver.load("example:hello").await.expect("plugin resolves");

	assert_eq!(source, "plugin");

	assert_eq!(result.tracks.len(), 1);

	let track = &result.tracks[0];

	assert_eq!(track.info.identifier, "example:hello");
	assert_eq!(track.info.source_name, "example");
	assert_eq!(track.info.uri.as_deref(), Some("https://example.com/audio.mp3"));
	assert!(track.info.title.contains("example:hello"));

	let encoded = track.encoded.as_deref().expect("resolver encodes plugin track");
	let decoded = decode_track(encoded).expect("plugin track decodes");

	assert_eq!(decoded.track.info.identifier, "example:hello");
	assert_eq!(decoded.track.info.source_name, "example");
}
