use std::collections::BTreeMap;
use std::sync::Arc;

use actix_web::web::{Data, Json, Path};
use actix_web::{HttpRequest, HttpResponse};
use orpheus_core::Filters;

use crate::audio_filters::validate_filters;
use crate::errors::{ErrorCode, RestError};
use crate::state::AppState;

pub async fn put_preset(
	req: HttpRequest,
	path: Path<String>,
	state: Data<Arc<AppState>>,
	filters: Json<Filters>,
) -> Result<Json<Filters>, RestError> {
	let name = path.into_inner();
	let filters = filters.into_inner();

	validate_filters(&filters).map_err(|reason| RestError::bad_request(ErrorCode::InvalidFilterValue, reason, &req))?;

	state.filter_presets.put(name, filters.clone());

	Ok(Json(filters))
}

pub async fn list_presets(state: Data<Arc<AppState>>) -> Json<BTreeMap<String, Filters>> {
	Json(state.filter_presets.list())
}

pub async fn get_preset(
	req: HttpRequest,
	path: Path<String>,
	state: Data<Arc<AppState>>,
) -> Result<Json<Filters>, RestError> {
	let name = path.into_inner();

	match state.filter_presets.get(&name) {
		Some(filters) => Ok(Json(filters)),
		None => Err(RestError::not_found(
			ErrorCode::FilterPresetNotFound,
			format!("Filter preset not found: {name}"),
			&req,
		)),
	}
}

pub async fn delete_preset(
	req: HttpRequest,
	path: Path<String>,
	state: Data<Arc<AppState>>,
) -> Result<HttpResponse, RestError> {
	let name = path.into_inner();

	if !state.filter_presets.remove(&name) {
		return Err(RestError::not_found(
			ErrorCode::FilterPresetNotFound,
			format!("Filter preset not found: {name}"),
			&req,
		));
	}

	Ok(HttpResponse::NoContent().finish())
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use actix_web::http::StatusCode;
	use actix_web::test::{TestRequest, call_service, init_service, read_body};
	use actix_web::{App, web};

	use super::*;
	use crate::state::AppState;

	async fn states() -> (Data<Arc<AppState>>, Data<Arc<crate::websocket::WebSocketState>>) {
		let track_resolver = Arc::new(orpheus_track_resolver::TrackResolver::new().await);
		let plugin_manager = Arc::new(tokio::sync::RwLock::new(
			orpheus_plugins::PluginManager::new()
				.await
				.expect("manager builds without wasm"),
		));
		let runtime_config = Arc::new(
			crate::runtime_config::RuntimeConfigStore::in_memory(
				crate::runtime_config::RuntimeConfig::default(),
				track_resolver.configurable_source_names(),
			)
			.unwrap(),
		);
		let state = Arc::new(AppState::new(track_resolver.clone(), plugin_manager, runtime_config));
		let ws_state = Arc::new(crate::websocket::WebSocketState::new(
			state.player_manager.clone(),
			"secret".to_string(),
			track_resolver,
			state.voice_connection_manager.clone(),
			state.metrics.clone(),
			state.prefetch.clone(),
		));

		(web::Data::new(state), web::Data::new(ws_state))
	}

	fn bass_boost() -> serde_json::Value {
		serde_json::json!({"equalizer": [{"band": 0, "gain": 0.8}]})
	}

	#[actix_web::test]
	async fn preset_crud_round_trip() {
		let (state, _) = states().await;
		let app = init_service(
			App::new()
				.app_data(state)
				.service(web::resource("/v4/filters/presets").route(web::get().to(list_presets)))
				.service(
					web::resource("/v4/filters/presets/{name}")
						.route(web::put().to(put_preset))
						.route(web::get().to(get_preset))
						.route(web::delete().to(delete_preset)),
				),
		)
		.await;

		let put = call_service(
			&app,
			TestRequest::put()
				.uri("/v4/filters/presets/bass")
				.set_json(bass_boost())
				.to_request(),
		)
		.await;

		assert_eq!(put.status(), StatusCode::OK);

		let body = read_body(put).await;
		let stored: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(stored["equalizer"][0]["gain"], 0.8);

		let get = call_service(&app, TestRequest::get().uri("/v4/filters/presets/bass").to_request()).await;

		assert_eq!(get.status(), StatusCode::OK);

		let bad = call_service(
			&app,
			TestRequest::put()
				.uri("/v4/filters/presets/broken")
				.set_json(serde_json::json!({"equalizer": [{"band": 99, "gain": 0.5}]}))
				.to_request(),
		)
		.await;

		assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

		let body = read_body(bad).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["code"], "INVALID_FILTER_VALUE");

		let delete = call_service(&app, TestRequest::delete().uri("/v4/filters/presets/bass").to_request()).await;

		assert_eq!(delete.status(), StatusCode::NO_CONTENT);

		let missing = call_service(&app, TestRequest::get().uri("/v4/filters/presets/bass").to_request()).await;

		assert_eq!(missing.status(), StatusCode::NOT_FOUND);

		let body = read_body(missing).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["code"], "FILTER_PRESET_NOT_FOUND");

		for (name, gain) in [("bass", 0.8), ("flat", 0.0)] {
			let put = call_service(
				&app,
				TestRequest::put()
					.uri(&format!("/v4/filters/presets/{name}"))
					.set_json(serde_json::json!({"equalizer": [{"band": 0, "gain": gain}]}))
					.to_request(),
			)
			.await;

			assert_eq!(put.status(), StatusCode::OK);
		}

		let list = call_service(&app, TestRequest::get().uri("/v4/filters/presets").to_request()).await;

		assert_eq!(list.status(), StatusCode::OK);

		let body = read_body(list).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["bass"]["equalizer"][0]["gain"], 0.8);
		assert_eq!(json["flat"]["equalizer"][0]["gain"], 0.0);
	}

	#[actix_web::test]
	async fn patch_applies_preset_by_name() {
		let (state, ws_state) = states().await;
		let session = ws_state.sessions.create(7).id;
		let app = init_service(
			App::new()
				.app_data(state)
				.app_data(ws_state)
				.service(web::resource("/v4/filters/presets/{name}").route(web::put().to(put_preset)))
				.service(
					web::resource("/v4/sessions/{session_id}/players/{guild_id}")
						.route(web::patch().to(crate::rest::players::update_player)),
				),
		)
		.await;

		let put = call_service(
			&app,
			TestRequest::put()
				.uri("/v4/filters/presets/bass")
				.set_json(bass_boost())
				.to_request(),
		)
		.await;

		assert_eq!(put.status(), StatusCode::OK);

		let player_path = format!("/v4/sessions/{session}/players/7");
		let applied = call_service(
			&app,
			TestRequest::patch()
				.uri(&player_path)
				.set_json(serde_json::json!({"filterPreset": "bass"}))
				.to_request(),
		)
		.await;

		assert_eq!(applied.status(), StatusCode::OK);

		let body = read_body(applied).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["filters"]["equalizer"][0]["gain"], 0.8);

		let unknown = call_service(
			&app,
			TestRequest::patch()
				.uri(&player_path)
				.set_json(serde_json::json!({"filterPreset": "nope"}))
				.to_request(),
		)
		.await;

		assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

		let body = read_body(unknown).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["code"], "FILTER_PRESET_NOT_FOUND");

		let conflict = call_service(
			&app,
			TestRequest::patch()
				.uri(&player_path)
				.set_json(serde_json::json!({"filterPreset": "bass", "filters": {}}))
				.to_request(),
		)
		.await;

		assert_eq!(conflict.status(), StatusCode::BAD_REQUEST);
	}
}
