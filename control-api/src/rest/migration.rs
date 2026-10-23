use std::sync::Arc;

use actix_web::HttpRequest;
use actix_web::web::{Data, Json, Path};
use orpheus_core::PlayerSnapshot;

use super::players::{PlayerResponse, live_position, player_response};
use super::sessions::{parse_guild_id, require_session};
use crate::errors::{ErrorCode, RestError};
use crate::state::AppState;

pub async fn export_player(
	req: HttpRequest,
	path: Path<(String, String)>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
) -> Result<Json<PlayerSnapshot>, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	let player_lock = match state.player_manager.get(guild_id).await {
		Some(player) => player,
		None => return Err(RestError::not_found(ErrorCode::PlayerNotFound, "Player not found", &req)),
	};
	let player = player_lock.read().await;

	Ok(Json(player.snapshot()))
}

pub async fn import_player(
	req: HttpRequest,
	path: Path<(String, String)>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
	snapshot: Json<PlayerSnapshot>,
) -> Result<Json<PlayerResponse>, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	let snapshot = snapshot.into_inner();

	if snapshot.version != orpheus_core::SNAPSHOT_VERSION {
		return Err(RestError::bad_request(
			ErrorCode::UnsupportedMigrationVersion,
			format!("Unsupported player snapshot version: {}", snapshot.version),
			&req,
		));
	}

	let player_lock = state.player_manager.get_or_create(guild_id).await;
	let response = {
		let mut player = player_lock.write().await;

		player.restore(snapshot);

		player_response(
			&player,
			guild_id,
			live_position(&state, guild_id, player.position),
			state.voice_connection_manager.is_connected(guild_id),
		)
	};

	state.voice_connection_manager.request_player_update(guild_id);

	Ok(Json(response))
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use actix_web::http::StatusCode;
	use actix_web::test::{TestRequest, call_service, init_service, read_body};
	use actix_web::web;

	use super::*;
	use crate::rest::players::get_player;
	use crate::state::AppState;

	async fn migration_state() -> (web::Data<Arc<AppState>>, web::Data<Arc<crate::websocket::WebSocketState>>) {
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

	fn track_json(identifier: &str, title: &str) -> serde_json::Value {
		serde_json::json!({
			"info": {
				"identifier": identifier,
				"isSeekable": true,
				"author": "author",
				"length": 1000,
				"isStream": false,
				"position": 0,
				"title": title,
				"uri": identifier,
				"sourceName": "local"
			}
		})
	}

	fn snapshot_json(version: u32, current: Option<serde_json::Value>, queue: Vec<serde_json::Value>) -> serde_json::Value {
		serde_json::json!({
			"version": version,
			"current": current,
			"queue": queue,
			"history": [],
			"position": 65000,
			"volume": 250,
			"filters": {},
			"repeat": "Off",
			"shuffle": false,
			"shufflePositions": [],
			"autoplay": false
		})
	}

	#[actix_web::test]
	async fn export_missing_player_is_not_found() {
		let (state, ws_state) = migration_state().await;
		let session = ws_state.sessions.create(7).id;
		let app = init_service(actix_web::App::new().app_data(state).app_data(ws_state).service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/export").route(web::get().to(export_player)),
		))
		.await;

		let missing = call_service(
			&app,
			TestRequest::get()
				.uri(&format!("/v4/sessions/{session}/players/7/export"))
				.to_request(),
		)
		.await;

		assert_eq!(missing.status(), StatusCode::NOT_FOUND);

		let body = read_body(missing).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["code"], "PLAYER_NOT_FOUND");
	}

	#[actix_web::test]
	async fn import_rejects_unknown_snapshot_version() {
		let (state, ws_state) = migration_state().await;
		let session = ws_state.sessions.create(7).id;
		let app = init_service(actix_web::App::new().app_data(state).app_data(ws_state).service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/import").route(web::post().to(import_player)),
		))
		.await;

		let bad = call_service(
			&app,
			TestRequest::post()
				.uri(&format!("/v4/sessions/{session}/players/7/import"))
				.set_json(snapshot_json(99, None, vec![]))
				.to_request(),
		)
		.await;

		assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

		let body = read_body(bad).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["code"], "UNSUPPORTED_MIGRATION_VERSION");
	}

	#[actix_web::test]
	async fn import_applies_snapshot_and_export_round_trips() {
		let (state, ws_state) = migration_state().await;
		let session = ws_state.sessions.create(7).id;
		let app = init_service(
			actix_web::App::new()
				.app_data(state)
				.app_data(ws_state)
				.service(
					web::resource("/v4/sessions/{session_id}/players/{guild_id}/import")
						.route(web::post().to(import_player)),
				)
				.service(
					web::resource("/v4/sessions/{session_id}/players/{guild_id}/export").route(web::get().to(export_player)),
				)
				.service(web::resource("/v4/sessions/{session_id}/players/{guild_id}").route(web::get().to(get_player))),
		)
		.await;

		let player_path = format!("/v4/sessions/{session}/players/7");
		let track = track_json("file:///song.mp3", "song");

		let imported = call_service(
			&app,
			TestRequest::post()
				.uri(&format!("{player_path}/import"))
				.set_json(snapshot_json(1, Some(track.clone()), vec![track]))
				.to_request(),
		)
		.await;

		assert_eq!(imported.status(), StatusCode::OK);

		let body = read_body(imported).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert!(json["paused"].as_bool().unwrap());
		assert_eq!(json["track"]["info"]["identifier"], "file:///song.mp3");
		assert_eq!(json["state"]["position"], 65000);

		let exported = call_service(&app, TestRequest::get().uri(&format!("{player_path}/export")).to_request()).await;

		assert_eq!(exported.status(), StatusCode::OK);

		let body = read_body(exported).await;
		let exported_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(exported_json["version"], 1);
		assert_eq!(exported_json["current"]["info"]["identifier"], "file:///song.mp3");
		assert_eq!(exported_json["queue"].as_array().unwrap().len(), 1);

		let player = call_service(&app, TestRequest::get().uri(&player_path).to_request()).await;

		assert_eq!(player.status(), StatusCode::OK);

		let body = read_body(player).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert!(json["paused"].as_bool().unwrap());
		assert_eq!(json["track"]["info"]["identifier"], "file:///song.mp3");
	}
}
