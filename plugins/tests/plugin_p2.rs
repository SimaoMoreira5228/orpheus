use std::path::PathBuf;

use orpheus_plugins::{HostError, PluginHost, PluginManager, PluginType};

const GUEST_BYTES: &[u8] = include_bytes!("fixtures/example-plugin.wasm");

fn temp_fixture(test: &str) -> PathBuf {
	let path = std::env::temp_dir().join(format!("orpheus-p2-{test}-{}.wasm", std::process::id()));

	std::fs::write(&path, GUEST_BYTES).expect("write temp fixture");

	path
}

#[tokio::test]
async fn new_guest_carries_type_and_routes() {
	let host = PluginHost::new().expect("host builds without wasm");
	let info = host.load_plugin(GUEST_BYTES, "example").await.expect("fixture guest loads");

	assert_eq!(info.plugin_type, PluginType::TrackResolver);
	assert_eq!(info.routes.len(), 1);
	assert_eq!(info.routes[0].method, "GET");
	assert_eq!(info.routes[0].path, "/hello");
}

#[tokio::test]
async fn config_stored_survives_reload_and_configure_invoked() {
	let path = temp_fixture("reload");
	let mut manager = PluginManager::new().await.expect("manager builds without wasm");

	manager.load_from_file("example", &path).await.expect("fixture guest loads");
	manager
		.set_plugin_config(
			"example",
			serde_json::json!({ "name": "configured", "allow_hosts": ["127.0.0.1"] }),
		)
		.await
		.expect("config stores");

	let stored = manager.plugin_config("example").await.expect("config stored");

	assert_eq!(stored["name"], "configured");

	let response = manager
		.handle_route("example", "GET", "/hello", "", "[]", "")
		.await
		.expect("route works");

	assert_eq!(response.status, 200);
	assert!(String::from_utf8_lossy(&response.body).contains("configured"));

	manager.hot_reload_from_file("example").await.expect("reload works");

	let stored = manager.plugin_config("example").await.expect("config survives reload");

	assert_eq!(stored["name"], "configured");

	let response = manager
		.handle_route("example", "GET", "/hello", "", "[]", "")
		.await
		.expect("route works after reload");

	assert!(String::from_utf8_lossy(&response.body).contains("configured"));

	let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn route_dispatch_rejects_unknown_path() {
	let mut manager = PluginManager::new().await.expect("manager builds without wasm");

	manager.load("example", GUEST_BYTES).await.expect("fixture guest loads");

	let err = manager
		.handle_route("example", "GET", "/missing", "", "[]", "")
		.await
		.unwrap_err();

	assert!(matches!(err, HostError::NotFound(_)));

	let err = manager
		.handle_route("ghost", "GET", "/hello", "", "[]", "")
		.await
		.unwrap_err();

	assert!(matches!(err, HostError::NotFound(_)));
}

#[tokio::test]
async fn capabilities_reflect_type_and_config() {
	let host = PluginHost::new().expect("host builds without wasm");

	host.load_plugin(GUEST_BYTES, "example").await.expect("fixture guest loads");

	let before = host.capabilities("example").await.expect("capabilities read");

	assert!(before.contains(&"resolve".to_string()));
	assert!(before.contains(&"handle-route".to_string()));
	assert!(before.contains(&"configure".to_string()));
	assert!(!before.contains(&"fetch".to_string()));

	host.set_plugin_config("example", serde_json::json!({ "allow_hosts": ["127.0.0.1"] }))
		.await
		.expect("config stores");

	let after = host.capabilities("example").await.expect("capabilities read");

	assert!(after.contains(&"fetch".to_string()));
}
