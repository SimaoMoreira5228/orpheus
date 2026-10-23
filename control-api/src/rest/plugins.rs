use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::http::header::{HeaderName, HeaderValue};
use actix_web::web::{Bytes, Data, Json, Path};
use actix_web::{HttpRequest, HttpResponse};
use orpheus_plugins::{HostError, PluginManager, RouteDecl};
use serde::Serialize;

use crate::errors::{ErrorCode, RestError};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct PluginListEntry {
	pub name: String,
	pub version: String,
	#[serde(rename = "type")]
	pub plugin_type: String,
	pub routes: Vec<RouteDecl>,
	pub capabilities: Vec<String>,
}

fn plugin_type_name(plugin_type: orpheus_plugins::PluginType) -> String {
	match plugin_type {
		orpheus_plugins::PluginType::TrackResolver => "track_resolver".to_string(),
		orpheus_plugins::PluginType::AudioFilter => "audio_filter".to_string(),
		orpheus_plugins::PluginType::Route => "route".to_string(),
		orpheus_plugins::PluginType::Unknown => "unknown".to_string(),
	}
}

fn host_error_to_rest(err: HostError, req: &HttpRequest) -> RestError {
	match err {
		HostError::NotFound(message) => RestError::not_found(ErrorCode::PluginNotFound, message, req),
		HostError::Module(message) | HostError::Component(message) => {
			RestError::bad_request(ErrorCode::InvalidPlugin, message, req)
		}
		other => RestError::internal(ErrorCode::PluginGuestFailed, other.to_string(), req),
	}
}

async fn entry_for(manager: &PluginManager, name: &str) -> Option<PluginListEntry> {
	let info = manager.get_plugin(name)?;

	Some(PluginListEntry {
		name: name.to_string(),
		version: info.version.clone(),
		plugin_type: plugin_type_name(info.plugin_type),
		routes: info.routes.clone(),
		capabilities: manager.plugin_capabilities(name).await.unwrap_or_default(),
	})
}

pub async fn list_plugins(state: Data<Arc<AppState>>) -> Json<Vec<PluginListEntry>> {
	let manager = state.plugin_manager.read().await;

	let mut entries = Vec::new();

	for name in manager.list() {
		if let Some(entry) = entry_for(&manager, &name).await {
			entries.push(entry);
		}
	}

	Json(entries)
}

pub async fn load_plugin(
	req: HttpRequest,
	path: Path<String>,
	body: Bytes,
	state: Data<Arc<AppState>>,
) -> Result<HttpResponse, RestError> {
	let name = path.into_inner();

	if body.is_empty() {
		return Err(RestError::bad_request(
			ErrorCode::InvalidPluginBinary,
			"empty plugin binary",
			&req,
		));
	}

	let mut manager = state.plugin_manager.write().await;

	manager
		.load(&name, &body)
		.await
		.map_err(|err| host_error_to_rest(err, &req))?;

	match entry_for(&manager, &name).await {
		Some(entry) => Ok(HttpResponse::Created().json(entry)),
		None => Err(RestError::internal(
			ErrorCode::PluginRegistryInconsistent,
			format!("plugin {name} loaded but not listed"),
			&req,
		)),
	}
}

pub async fn unload_plugin(
	req: HttpRequest,
	path: Path<String>,
	state: Data<Arc<AppState>>,
) -> Result<HttpResponse, RestError> {
	let name = path.into_inner();
	let mut manager = state.plugin_manager.write().await;

	if !manager.is_loaded(&name) {
		return Err(RestError::not_found(
			ErrorCode::PluginNotFound,
			format!("plugin {name} not found"),
			&req,
		));
	}

	manager.unload(&name).await;

	Ok(HttpResponse::NoContent().finish())
}

pub async fn reload_plugin(
	req: HttpRequest,
	path: Path<String>,
	state: Data<Arc<AppState>>,
) -> Result<Json<PluginListEntry>, RestError> {
	let name = path.into_inner();
	let mut manager = state.plugin_manager.write().await;

	if !manager.is_loaded(&name) {
		return Err(RestError::not_found(
			ErrorCode::PluginNotFound,
			format!("plugin {name} not found"),
			&req,
		));
	}

	manager
		.hot_reload_from_file(&name)
		.await
		.map_err(|err| host_error_to_rest(err, &req))?;

	match entry_for(&manager, &name).await {
		Some(entry) => Ok(Json(entry)),
		None => Err(RestError::internal(
			ErrorCode::PluginRegistryInconsistent,
			format!("plugin {name} reloaded but not listed"),
			&req,
		)),
	}
}

pub async fn set_plugin_config(
	req: HttpRequest,
	path: Path<String>,
	body: Json<serde_json::Value>,
	state: Data<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, RestError> {
	let name = path.into_inner();
	let mut manager = state.plugin_manager.write().await;

	if !manager.is_loaded(&name) {
		return Err(RestError::not_found(
			ErrorCode::PluginNotFound,
			format!("plugin {name} not found"),
			&req,
		));
	}

	let config = body.into_inner();

	manager
		.set_plugin_config(&name, config.clone())
		.await
		.map_err(|err| host_error_to_rest(err, &req))?;

	Ok(Json(config))
}

pub async fn dispatch_route(
	req: HttpRequest,
	path: Path<(String, String)>,
	body: Bytes,
	state: Data<Arc<AppState>>,
) -> Result<HttpResponse, RestError> {
	let (name, rest) = path.into_inner();
	let route_path = format!("/{rest}");
	let method = req.method().as_str().to_string();
	let query = req.query_string().to_string();

	let headers: Vec<(String, String)> = req
		.headers()
		.iter()
		.map(|(name, value)| (name.to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned()))
		.collect();
	let headers_json = serde_json::to_string(&headers)
		.map_err(|err| RestError::internal(ErrorCode::PluginRouteFailed, format!("header encode failed: {err}"), &req))?;
	let body_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &body);

	let mut manager = state.plugin_manager.write().await;
	let response = manager
		.handle_route(&name, &method, &route_path, &query, &headers_json, &body_b64)
		.await
		.map_err(|err| host_error_to_rest(err, &req))?;

	let mut builder =
		HttpResponse::build(StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));

	for (name, value) in response.headers {
		if let (Ok(header_name), Ok(header_value)) = (HeaderName::from_str(&name), HeaderValue::from_str(&value)) {
			builder.append_header((header_name, header_value));
		}
	}

	Ok(builder.body(response.body))
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use actix_web::http::StatusCode;
	use actix_web::test::{TestRequest, call_and_read_body_json, call_service, init_service, read_body};
	use actix_web::web;
	use orpheus_plugins::PluginManager;

	use super::*;

	const GUEST_BYTES: &[u8] = include_bytes!("../../../plugins/tests/fixtures/example-plugin.wasm");

	async fn state_with_manager(manager: PluginManager) -> web::Data<Arc<AppState>> {
		let track_resolver = Arc::new(orpheus_track_resolver::TrackResolver::new().await);
		let runtime_config = Arc::new(
			crate::runtime_config::RuntimeConfigStore::in_memory(
				crate::runtime_config::RuntimeConfig::default(),
				track_resolver.configurable_source_names(),
			)
			.unwrap(),
		);
		let state = Arc::new(AppState::new(
			track_resolver,
			Arc::new(tokio::sync::RwLock::new(manager)),
			runtime_config,
		));

		web::Data::new(state)
	}

	async fn state_with_fixture() -> web::Data<Arc<AppState>> {
		let mut manager = PluginManager::new().await.expect("manager builds without wasm");

		manager.load("example", GUEST_BYTES).await.expect("fixture guest loads");

		state_with_manager(manager).await
	}

	#[actix_web::test]
	async fn admin_list_reflects_loaded_guest() {
		let app = init_service(
			actix_web::App::new()
				.app_data(state_with_fixture().await)
				.configure(crate::rest::configure),
		)
		.await;

		let body: serde_json::Value =
			call_and_read_body_json(&app, TestRequest::get().uri("/v4/plugins").to_request()).await;

		assert!(body.is_array());

		let entry = body
			.as_array()
			.unwrap()
			.iter()
			.find(|entry| entry["name"] == "example")
			.expect("guest listed");

		assert_eq!(entry["version"], "0.1.0");
		assert_eq!(entry["type"], "track_resolver");
		assert_eq!(entry["routes"], serde_json::json!([{ "method": "GET", "path": "/hello" }]));

		let capabilities = entry["capabilities"].as_array().expect("capabilities array");

		assert!(capabilities.contains(&serde_json::json!("resolve")));
		assert!(capabilities.contains(&serde_json::json!("handle-route")));
		assert!(capabilities.contains(&serde_json::json!("configure")));
	}

	#[actix_web::test]
	async fn guest_route_dispatch_returns_demo_payload() {
		let app = init_service(
			actix_web::App::new()
				.app_data(state_with_fixture().await)
				.configure(crate::rest::configure),
		)
		.await;

		let response = call_service(&app, TestRequest::get().uri("/v4/plugins/example/hello").to_request()).await;

		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(response.headers().get("content-type").unwrap(), "application/json");

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json, serde_json::json!({ "hello": "example-plugin" }));
	}

	#[actix_web::test]
	async fn guest_route_unknown_path_returns_rest_404_envelope() {
		let app = init_service(
			actix_web::App::new()
				.app_data(state_with_fixture().await)
				.configure(crate::rest::configure),
		)
		.await;

		let response = call_service(&app, TestRequest::get().uri("/v4/plugins/example/nope").to_request()).await;

		assert_eq!(response.status(), StatusCode::NOT_FOUND);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["status"], 404);
		assert_eq!(json["error"], "Not Found");
		assert!(json["message"].as_str().unwrap().contains("nope"));
		assert_eq!(json["path"], "/v4/plugins/example/nope");
		assert!(json["timestamp"].is_number());
		assert!(json.get("trace").is_none());
	}

	#[actix_web::test]
	async fn config_round_trip_updates_live_guest() {
		let app = init_service(
			actix_web::App::new()
				.app_data(state_with_fixture().await)
				.configure(crate::rest::configure),
		)
		.await;

		let stored: serde_json::Value = call_and_read_body_json(
			&app,
			TestRequest::put()
				.uri("/v4/plugins/example/config")
				.set_json(serde_json::json!({ "name": "live", "allow_hosts": [] }))
				.to_request(),
		)
		.await;

		assert_eq!(stored["name"], "live");

		let routed: serde_json::Value =
			call_and_read_body_json(&app, TestRequest::get().uri("/v4/plugins/example/hello").to_request()).await;

		assert_eq!(routed, serde_json::json!({ "hello": "live" }));
	}

	#[actix_web::test]
	async fn config_for_missing_plugin_returns_404_envelope() {
		let app = init_service(
			actix_web::App::new()
				.app_data(state_with_fixture().await)
				.configure(crate::rest::configure),
		)
		.await;

		let response = call_service(
			&app,
			TestRequest::put()
				.uri("/v4/plugins/ghost/config")
				.set_json(serde_json::json!({ "name": "ghost" }))
				.to_request(),
		)
		.await;

		assert_eq!(response.status(), StatusCode::NOT_FOUND);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["status"], 404);
		assert_eq!(json["path"], "/v4/plugins/ghost/config");
	}

	#[actix_web::test]
	async fn load_unload_cycle_through_admin() {
		let app = init_service(
			actix_web::App::new()
				.app_data(state_with_fixture().await)
				.configure(crate::rest::configure),
		)
		.await;

		let response = call_service(
			&app,
			TestRequest::post()
				.uri("/v4/plugins/staged")
				.insert_header(("content-type", "application/octet-stream"))
				.set_payload(web::Bytes::from_static(GUEST_BYTES))
				.to_request(),
		)
		.await;

		assert_eq!(response.status(), StatusCode::CREATED);

		let created = read_body(response).await;
		let created: serde_json::Value = serde_json::from_slice(&created).unwrap();

		assert_eq!(created["name"], "staged");

		let response = call_service(&app, TestRequest::delete().uri("/v4/plugins/staged").to_request()).await;

		assert_eq!(response.status(), StatusCode::NO_CONTENT);

		let response = call_service(&app, TestRequest::delete().uri("/v4/plugins/staged").to_request()).await;

		assert_eq!(response.status(), StatusCode::NOT_FOUND);
	}

	#[actix_web::test]
	async fn load_rejects_garbage_binary() {
		let app = init_service(
			actix_web::App::new()
				.app_data(state_with_fixture().await)
				.configure(crate::rest::configure),
		)
		.await;

		let response = call_service(
			&app,
			TestRequest::post()
				.uri("/v4/plugins/broken")
				.insert_header(("content-type", "application/octet-stream"))
				.set_payload(web::Bytes::from_static(b"not a wasm component"))
				.to_request(),
		)
		.await;

		assert_eq!(response.status(), StatusCode::BAD_REQUEST);
	}

	#[actix_web::test]
	async fn reload_from_file_serves_updated_guest() {
		let path = std::env::temp_dir().join(format!("orpheus-admin-reload-{}.wasm", std::process::id()));

		std::fs::write(&path, GUEST_BYTES).expect("write temp fixture");

		let mut manager = PluginManager::new().await.expect("manager builds without wasm");

		manager.load_from_file("example", &path).await.expect("fixture guest loads");

		let app = init_service(
			actix_web::App::new()
				.app_data(state_with_manager(manager).await)
				.configure(crate::rest::configure),
		)
		.await;

		let response = call_service(&app, TestRequest::post().uri("/v4/plugins/example/reload").to_request()).await;

		assert_eq!(response.status(), StatusCode::OK);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["name"], "example");

		let _ = std::fs::remove_file(&path);
	}
}
