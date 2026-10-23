use std::sync::Arc;

use actix_web::HttpRequest;
use actix_web::web::{Data, Json, Path};
use orpheus_core::{Player, PlayerState};

use super::super::sessions::{parse_guild_id, require_session};
use super::{PlayerResponse, PlayerStateResponse, VoiceStateResponse};
use crate::errors::{ErrorCode, RestError};
use crate::state::AppState;

fn player_state_to_response(_state: PlayerState, position: u64, connected: bool) -> PlayerStateResponse {
	PlayerStateResponse {
		time: std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_millis() as u64,
		position,
		connected,
		// NOTE: the voice transport exposes no RTT measurement; -1 is Lavalink's own
		// sentinel for unknown ping, and lavalink-rs maps it back to None.
		ping: -1,
	}
}

pub(crate) fn live_position(state: &AppState, guild_id: u64, fallback: u64) -> u64 {
	state.voice_connection_manager.current_position(guild_id).unwrap_or(fallback)
}

pub(crate) fn player_response(player: &Player, guild_id: u64, position: u64, connected: bool) -> PlayerResponse {
	PlayerResponse {
		guild_id: guild_id.to_string(),
		track: player.current.clone(),
		volume: player.volume as i32,
		paused: player.state == PlayerState::Paused,
		state: player_state_to_response(player.state, position, connected),
		voice: player.voice_state.as_ref().map(|v| VoiceStateResponse {
			token: v.token.clone(),
			endpoint: v.endpoint.clone(),
			session_id: v.session_id.clone(),
		}),
		filters: player.filters.clone(),
	}
}

pub async fn get_all_players(
	req: HttpRequest,
	path: Path<String>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
) -> Result<Json<Vec<PlayerResponse>>, RestError> {
	let session_id = path.into_inner();

	require_session(&ws_state.sessions, &session_id, &req)?;

	let players = state.player_manager.get_all_players().await;
	let responses: Vec<PlayerResponse> = players
		.iter()
		.map(|(guild_id, player)| PlayerResponse {
			guild_id: guild_id.to_string(),
			track: player.current.clone(),
			volume: player.volume as i32,
			paused: player.state == PlayerState::Paused,
			state: player_state_to_response(
				player.state,
				live_position(&state, *guild_id, player.position),
				state.voice_connection_manager.is_connected(*guild_id),
			),
			voice: player.voice_state.as_ref().map(|v| VoiceStateResponse {
				token: v.token.clone(),
				endpoint: v.endpoint.clone(),
				session_id: v.session_id.clone(),
			}),
			filters: player.filters.clone(),
		})
		.collect();

	Ok(Json(responses))
}

pub async fn get_player(
	req: HttpRequest,
	path: Path<(String, String)>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
) -> Result<Json<PlayerResponse>, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	let player_lock = match state.player_manager.get(guild_id).await {
		Some(player) => player,
		None => return Err(RestError::not_found(ErrorCode::PlayerNotFound, "Player not found", &req)),
	};
	let player = player_lock.read().await;

	Ok(Json(player_response(
		&player,
		guild_id,
		live_position(&state, guild_id, player.position),
		state.voice_connection_manager.is_connected(guild_id),
	)))
}
