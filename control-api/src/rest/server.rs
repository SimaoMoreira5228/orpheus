use std::sync::Arc;

use actix_web::web::{Data, Json};
use actix_web::{HttpRequest, HttpResponse};
use orpheus_core::PlayerState;
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, RestError};
use crate::node_info::LavalinkInfo;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct Stats {
	pub players: i32,
	#[serde(rename = "playingPlayers")]
	pub playing_players: i32,
	pub uptime: i64,
	pub memory: MemoryInfo,
	pub cpu: CpuInfo,
	#[serde(rename = "frameStats")]
	pub frame_stats: Option<crate::websocket_messages::FrameStats>,
}

#[derive(Debug, Serialize)]
pub struct MemoryInfo {
	pub free: i64,
	pub used: i64,
	pub allocated: i64,
	pub reservable: i64,
}

#[derive(Debug, Serialize)]
pub struct CpuInfo {
	pub cores: i32,
	pub system_load: f32,
	pub lavalink_load: f32,
}

#[derive(Debug, Deserialize)]
pub struct FreeAddressRequest {
	pub address: String,
}

#[derive(Debug, Serialize)]
pub struct VersionResponse {
	pub version: String,
}

pub async fn get_info(state: Data<Arc<AppState>>) -> Json<LavalinkInfo> {
	let source_managers = state.track_resolver.source_names();
	let plugins = state.plugin_manager.read().await.get_all_plugin_info();

	Json(LavalinkInfo {
		version: crate::node_info::version_info(),
		build_time: crate::node_info::build_time_ms(),
		git: crate::node_info::git_info(),
		jvm: crate::node_info::runtime_name(),
		lavaplayer: crate::node_info::lavaplayer_version(),
		source_managers,
		filters: crate::node_info::enabled_filters(),
		plugins: crate::node_info::plugin_wire_info(&plugins),
	})
}

pub async fn get_stats(state: Data<Arc<AppState>>) -> Json<Stats> {
	let players = state.player_manager.get_all_players().await;
	let playing = players.iter().filter(|(_, p)| p.state == PlayerState::Playing).count();

	state.metrics.update_player_metrics(players.len(), playing);
	let snapshot = state.metrics.snapshot();

	Json(Stats {
		players: players.len() as i32,
		playing_players: playing as i32,
		uptime: snapshot.uptime_ms as i64,
		memory: MemoryInfo {
			free: snapshot.memory_free as i64,
			used: snapshot.memory_used as i64,
			allocated: snapshot.memory_total as i64,
			reservable: snapshot.memory_total as i64,
		},
		cpu: CpuInfo {
			cores: std::thread::available_parallelism()
				.map(|cores| cores.get() as i32)
				.unwrap_or(1),
			system_load: snapshot.system_load as f32,
			lavalink_load: snapshot.process_load as f32,
		},
		frame_stats: None,
	})
}

pub async fn get_version() -> Json<VersionResponse> {
	Json(VersionResponse {
		version: env!("CARGO_PKG_VERSION").to_string(),
	})
}

pub async fn get_routeplanner_status(state: Data<Arc<AppState>>) -> Result<HttpResponse, RestError> {
	match state.route_planner.get_status().await {
		Some(status) => Ok(HttpResponse::Ok().json(status)),
		None => Ok(HttpResponse::NoContent().finish()),
	}
}

pub async fn free_address(
	req: HttpRequest,
	state: Data<Arc<AppState>>,
	request: Json<FreeAddressRequest>,
) -> Result<HttpResponse, RestError> {
	if !state.route_planner.is_enabled().await {
		return Err(RestError::internal(
			ErrorCode::RoutePlannerDisabled,
			"Can't access disabled route planner",
			&req,
		));
	}

	let address: std::net::IpAddr = request
		.address
		.parse()
		.map_err(|e| RestError::bad_request(ErrorCode::InvalidAddress, format!("Invalid address: {e}"), &req))?;

	state.route_planner.unblock_ip(&address).await;

	Ok(HttpResponse::NoContent().finish())
}

pub async fn free_all(req: HttpRequest, state: Data<Arc<AppState>>) -> Result<HttpResponse, RestError> {
	if !state.route_planner.unblock_all().await {
		return Err(RestError::internal(
			ErrorCode::RoutePlannerDisabled,
			"Can't access disabled route planner",
			&req,
		));
	}

	Ok(HttpResponse::NoContent().finish())
}

pub async fn get_metrics(state: Data<Arc<AppState>>) -> HttpResponse {
	let metrics = &state.metrics.metrics;

	metrics.set_queue_depth(state.player_manager.queued_track_count().await as u64);
	let output = metrics.encode_metrics();

	HttpResponse::Ok()
		.content_type("text/plain; version=0.0.4; charset=utf-8")
		.body(output)
}

pub async fn plugin_unavailable(req: HttpRequest) -> Result<HttpResponse, RestError> {
	Err(RestError::not_found(
		ErrorCode::UnsupportedPluginEndpoint,
		"Plugin endpoint not supported by this server",
		&req,
	))
}

#[cfg(test)]
mod tests {
	use actix_web::http::StatusCode;
	use actix_web::test::{TestRequest, call_service, init_service, read_body};
	use actix_web::web;

	use super::*;

	#[actix_web::test]
	async fn plugin_route_returns_lavalink_404_envelope() {
		let app = init_service(actix_web::App::new().route(
			"/v4/sessions/{session_id}/players/{guild_id}/sponsorblock/categories",
			web::get().to(plugin_unavailable),
		))
		.await;

		let response = call_service(
			&app,
			TestRequest::get()
				.uri("/v4/sessions/abc/players/7/sponsorblock/categories")
				.to_request(),
		)
		.await;

		assert_eq!(response.status(), StatusCode::NOT_FOUND);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["status"], 404);
		assert_eq!(json["error"], "Not Found");
		assert!(json["message"].is_string());
		assert_eq!(json["path"], "/v4/sessions/abc/players/7/sponsorblock/categories");
		assert!(json["timestamp"].is_number());
		assert!(json.get("trace").is_none());
	}
}
