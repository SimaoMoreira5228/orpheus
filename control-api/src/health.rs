use std::sync::Arc;
use std::sync::atomic::Ordering;

use actix_web::web::{self, Data, Json};
use orpheus_core::PlayerState;
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
	pub status: String,
	pub players: usize,
	pub uptime: u64,
	pub version: String,
}

#[derive(Serialize)]
pub struct StatsResponse {
	pub players: PlayersStats,
	pub memory: MemoryStats,
	pub cpu: CpuStats,
	pub frames: FramesStats,
}

#[derive(Serialize)]
pub struct PlayersStats {
	pub total: usize,
	pub playing: usize,
	pub paused: usize,
}

#[derive(Serialize)]
pub struct MemoryStats {
	pub used: u64,
	pub total: u64,
}

#[derive(Serialize)]
pub struct CpuStats {
	pub usage: f32,
}

#[derive(Serialize)]
pub struct FramesStats {
	pub sent: u64,
	pub nulled: u64,
	pub deficit: u64,
}

#[derive(Serialize)]
pub struct InfoResponse {
	pub name: String,
	pub version: String,
	pub description: String,
}

pub async fn health(state: Data<Arc<AppState>>) -> Json<HealthResponse> {
	let uptime = state.uptime();

	Json(HealthResponse {
		status: "ok".to_string(),
		players: state.player_manager.get_player_count().await,
		uptime,
		version: env!("CARGO_PKG_VERSION").to_string(),
	})
}

pub async fn stats(state: Data<Arc<AppState>>) -> Json<StatsResponse> {
	state.metrics.update_system_metrics();

	let players = state.player_manager.get_all_players().await;
	let total = players.len();
	let playing = players.iter().filter(|(_, p)| p.state == PlayerState::Playing).count();
	let paused = players.iter().filter(|(_, p)| p.state == PlayerState::Paused).count();

	let metrics = &state.metrics.metrics;
	let used_mem = metrics.memory_used_bytes.load(Ordering::Relaxed);
	let total_mem = metrics.memory_total_bytes.load(Ordering::Relaxed);
	let cpu_usage = metrics.system_cpu_load() as f32;

	Json(StatsResponse {
		players: PlayersStats { total, playing, paused },
		memory: MemoryStats {
			used: used_mem,
			total: total_mem,
		},
		cpu: CpuStats { usage: cpu_usage },
		frames: FramesStats {
			sent: 0,
			nulled: 0,
			deficit: 0,
		},
	})
}

pub async fn info() -> Json<InfoResponse> {
	Json(InfoResponse {
		name: "orpheus".to_string(),
		version: env!("CARGO_PKG_VERSION").to_string(),
		description: "A music bot with Lavalink-like protocol".to_string(),
	})
}

pub async fn version() -> Json<serde_json::Value> {
	Json(serde_json::json!({
		"version": env!("CARGO_PKG_VERSION"),
		"name": "orpheus"
	}))
}

pub fn configure(cfg: &mut web::ServiceConfig) {
	cfg.service(web::resource("/health").route(web::get().to(health)))
		.service(web::resource("/stats").route(web::get().to(stats)))
		.service(web::resource("/info").route(web::get().to(info)))
		.service(web::resource("/version").route(web::get().to(version)));
}
