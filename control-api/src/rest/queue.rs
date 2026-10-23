use std::sync::Arc;

use actix_web::HttpRequest;
use actix_web::web::{Data, Json, Path};
use orpheus_core::{Filters, RepeatMode, Track};
use serde::{Deserialize, Serialize};

use super::sessions::{parse_guild_id, require_session};
use crate::errors::RestError;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct QueueResponse {
	pub current: Option<Track>,
	pub queue: Vec<Track>,
	pub history: Vec<Track>,
	pub position: usize,
	pub repeat: RepeatMode,
	pub autoplay: bool,
}

#[derive(Debug, Deserialize)]
pub struct AddToQueueRequest {
	pub tracks: Vec<Track>,
	#[serde(default)]
	pub position: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct QueueUpdateRequest {
	#[serde(default)]
	pub shuffle: Option<bool>,
	#[serde(default)]
	pub repeat: Option<RepeatMode>,
	#[serde(default)]
	pub position: Option<usize>,
	#[serde(default)]
	pub autoplay: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct QueuePositionResponse {
	pub track: Option<Track>,
	pub position: usize,
}

pub async fn get_queue(
	req: HttpRequest,
	path: Path<(String, String)>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
) -> Result<Json<QueueResponse>, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	let player_lock = state.player_manager.get_or_create(guild_id).await;
	let player = player_lock.read().await;
	let queue_info = player.get_queue_info();

	Ok(Json(QueueResponse {
		current: queue_info.current,
		queue: queue_info.queue,
		history: queue_info.history,
		position: queue_info.position,
		repeat: queue_info.repeat,
		autoplay: queue_info.autoplay,
	}))
}

pub async fn add_to_queue(
	req: HttpRequest,
	path: Path<(String, String)>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
	request: Json<AddToQueueRequest>,
) -> Result<Json<QueueResponse>, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	let player_lock = state.player_manager.get_or_create(guild_id).await;
	let mut player = player_lock.write().await;

	let req = request.into_inner();
	if let Some(position) = req.position {
		player.insert_tracks(position, req.tracks);
	} else {
		player.add_tracks(req.tracks);
	}
	state.prefetch.invalidate(guild_id);

	let queue_info = player.get_queue_info();
	Ok(Json(QueueResponse {
		current: queue_info.current,
		queue: queue_info.queue,
		history: queue_info.history,
		position: queue_info.position,
		repeat: queue_info.repeat,
		autoplay: queue_info.autoplay,
	}))
}

/// Orpheus queue extension: positions index the pending queue in order
/// (position 0 is the first pending track). Standard clients ignore this endpoint.
/// Removing the current track is done with PATCH `{"track": null}` (standard Lavalink stop).
pub async fn remove_from_queue(
	req: HttpRequest,
	path: Path<(String, String, usize)>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
) -> Result<Json<QueuePositionResponse>, RestError> {
	let (session_id, guild_raw, position) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	let player_lock = state.player_manager.get_or_create(guild_id).await;

	let track = {
		let mut player = player_lock.write().await;

		player.remove_from_queue(position)
	};
	if track.is_some() {
		state.prefetch.invalidate(guild_id);
	}

	Ok(Json(QueuePositionResponse { track, position }))
}

pub async fn update_queue(
	req: HttpRequest,
	path: Path<(String, String)>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
	request: Json<QueueUpdateRequest>,
) -> Result<Json<QueueResponse>, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	let player_lock = state.player_manager.get_or_create(guild_id).await;

	let mut play_request: Option<(Track, u32, Filters, Option<u64>)> = None;
	let mut stop_request = false;
	let mut changed = false;
	let mut invalidate_prefetch = false;

	let response = {
		let mut player = player_lock.write().await;

		let req = request.into_inner();

		if let Some(shuffle) = req.shuffle {
			invalidate_prefetch = true;
			if shuffle {
				player.shuffle_queue();
			} else {
				player.unshuffle_queue();
			}
		}

		if let Some(repeat) = req.repeat {
			invalidate_prefetch = true;
			player.set_repeat(repeat);
		}

		if let Some(autoplay) = req.autoplay {
			invalidate_prefetch = true;
			player.set_autoplay(autoplay);
		}

		if let Some(position) = req.position {
			invalidate_prefetch = true;
			let had_current = player.current.is_some();

			match player.skip_to(position) {
				Some(track) => {
					changed = true;

					if track.playable_uri().is_some() {
						let end_time = track.end_time;
						play_request = Some((track, player.volume, player.filters.clone(), end_time));
					}
				}
				None => {
					if had_current && player.current.is_none() {
						changed = true;
						stop_request = true;
					}
				}
			}
		}

		let queue_info = player.get_queue_info();

		QueueResponse {
			current: queue_info.current,
			queue: queue_info.queue,
			history: queue_info.history,
			position: queue_info.position,
			repeat: queue_info.repeat,
			autoplay: queue_info.autoplay,
		}
	};
	if invalidate_prefetch {
		state.prefetch.invalidate(guild_id);
	}

	if let Some((track, volume, filters, end_time)) = play_request {
		let playback_uri = state.track_resolver.playback_url(&track).await;

		let mut voice_track = track;

		voice_track.info.uri = Some(playback_uri);

		state
			.voice_connection_manager
			.play(guild_id, voice_track, 0, volume, filters, end_time)
			.await;
	} else if stop_request {
		state.voice_connection_manager.stop(guild_id).await;
	}

	if changed {
		state.voice_connection_manager.request_player_update(guild_id);
	}

	Ok(Json(response))
}

#[cfg(test)]
mod tests {
	use orpheus_core::{Player, PlayerState, TrackInfo};

	use super::*;

	fn queued_track(title: &str) -> Track {
		Track {
			encoded: None,
			info: TrackInfo {
				identifier: "id".to_string(),
				is_seekable: true,
				author: "author".to_string(),
				length: 200_000,
				is_stream: false,
				position: 0,
				title: title.to_string(),
				uri: Some(format!("https://example.invalid/{title}.opus")),
				artwork_url: None,
				isrc: None,
				source_name: "http".to_string(),
			},
			start_time: None,
			end_time: None,
			plugin_info: serde_json::json!({}),
			user_data: serde_json::json!({}),
		}
	}

	fn playing_player() -> Player {
		let mut player = Player::new(7);
		player.current = Some(queued_track("current"));
		player.queue.push_back(queued_track("next"));
		player.queue.push_back(queued_track("later"));
		player.state = PlayerState::Playing;
		player
	}

	#[test]
	fn delete_position_zero_removes_first_pending_track() {
		let mut player = playing_player();

		let track = player.remove_from_queue(0);

		assert_eq!(track.map(|track| track.info.title).as_deref(), Some("next"));
		assert_eq!(
			player.current.as_ref().map(|track| track.info.title.as_str()),
			Some("current")
		);
		assert_eq!(player.state, PlayerState::Playing);
		assert_eq!(player.queue.len(), 1);
		assert!(player.history.is_empty());
	}

	#[test]
	fn delete_out_of_range_is_a_silent_no_op() {
		let mut player = playing_player();

		assert!(player.remove_from_queue(9).is_none());
		assert_eq!(
			player.current.as_ref().map(|track| track.info.title.as_str()),
			Some("current")
		);
		assert_eq!(player.state, PlayerState::Playing);
		assert_eq!(player.queue.len(), 2);
	}
}
