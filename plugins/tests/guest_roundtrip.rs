use orpheus_plugins::{HostError, PluginHost, PluginManager, PluginType};

const GUEST_BYTES: &[u8] = include_bytes!("fixtures/example-plugin.wasm");

async fn loaded_host() -> PluginHost {
	let host = PluginHost::new().expect("host builds without wasm");
	host.load_plugin(GUEST_BYTES, "example").await.expect("fixture guest loads");
	host
}

#[tokio::test]
async fn load_detects_track_resolver_with_guest_metadata() {
	let host = PluginHost::new().expect("host builds without wasm");

	let info = host.load_plugin(GUEST_BYTES, "example").await.expect("fixture guest loads");

	assert_eq!(info.plugin_type, PluginType::TrackResolver);
	assert_eq!(info.name, "example-plugin");
	assert_eq!(info.version, "0.1.0");
}

#[tokio::test]
async fn resolve_echoes_query_as_json() {
	let host = loaded_host().await;

	let raw = host
		.call_resolve("example", "never gonna give you up")
		.await
		.expect("resolve works");

	let track: serde_json::Value = serde_json::from_str(&raw).expect("resolve returns JSON");

	assert_eq!(track["title"], "Search result for: never gonna give you up");
}

#[tokio::test]
async fn track_info_returns_parseable_json() {
	let host = loaded_host().await;

	let raw = host.call_track_info("example").await.expect("track-info works");

	let info: serde_json::Value = serde_json::from_str(&raw).expect("track-info returns JSON");

	assert_eq!(info["name"], "example-plugin");
	assert_eq!(info["version"], "0.1.0");
}

#[tokio::test]
async fn unknown_plugin_returns_not_found() {
	let host = PluginHost::new().expect("host builds without wasm");

	let resolve_err = host.call_resolve("missing", "query").await.unwrap_err();
	assert!(matches!(resolve_err, HostError::NotFound(_)));

	let mut frame = [0i16; 16];
	let filter_err = host.call_filter("missing", &mut frame).await.unwrap_err();
	assert!(matches!(filter_err, HostError::NotFound(_)));

	let info_err = host.call_track_info("missing").await.unwrap_err();
	assert!(matches!(info_err, HostError::NotFound(_)));
}

#[tokio::test]
async fn init_shutdown_cycle_through_manager() {
	let mut manager = PluginManager::new().await.expect("manager builds without wasm");

	manager.load("example", GUEST_BYTES).await.expect("fixture guest loads");
	assert!(manager.is_loaded("example"));

	manager.unload("example").await;
	assert!(!manager.is_loaded("example"));

	let err = manager.resolve_track("example", "query").await.unwrap_err();
	assert!(matches!(err, HostError::NotFound(_)));
}

#[tokio::test]
async fn manager_reports_detected_type_and_version() {
	let mut manager = PluginManager::new().await.expect("manager builds without wasm");
	manager.load("example", GUEST_BYTES).await.expect("fixture guest loads");

	let infos = manager.get_all_plugin_info();

	assert_eq!(infos.len(), 1);
	assert_eq!(infos[0].plugin_type, "track_resolver");
	assert_eq!(infos[0].version, "0.1.0");

	let resolvers = manager
		.resolve_track("example", "hello")
		.await
		.expect("resolver dispatch works");
	assert!(resolvers.contains("hello"));
}
