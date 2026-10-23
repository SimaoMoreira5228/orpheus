use orpheus_plugins::PluginHost;

const GUEST_BYTES: &[u8] = include_bytes!("fixtures/example-plugin.wasm");

#[tokio::test]
async fn old_guest_without_kv_import_still_loads_and_resolves() {
	let host = PluginHost::new().expect("host builds without wasm");

	let info = host
		.load_plugin(GUEST_BYTES, "example")
		.await
		.expect("pre-kv fixture guest loads after kv import added");

	assert_eq!(info.name, "example-plugin");

	let raw = host
		.call_resolve("example", "kv compat probe")
		.await
		.expect("resolve works without kv import");

	let track: serde_json::Value = serde_json::from_str(&raw).expect("resolve returns JSON");

	assert_eq!(track["title"], "Search result for: kv compat probe");
}
