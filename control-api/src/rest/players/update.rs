use std::sync::Arc;

use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpRequest, HttpResponse};
use orpheus_core::{Filters, PlayerState, Track, VoiceState};
use serde::Deserialize;

use super::super::sessions::{parse_guild_id, require_session};
use super::super::tracks::{ensure_playable, merge_no_replace};
use super::{PlayerResponse, PlayerUpdate, UpdatePlayerTrack, live_position, player_response};
use crate::errors::{ErrorCode, RestError};
use crate::state::AppState;

#[derive(Debug, Deserialize, Default)]
pub struct NoReplaceQuery {
	#[serde(default, rename = "noReplace")]
	pub no_replace: Option<bool>,
}

pub(crate) fn validate_patch(
	track: &Option<Option<UpdatePlayerTrack>>,
	voice: &Option<VoiceState>,
	end_time: &Option<Option<i64>>,
	filters: &Option<Filters>,
	filter_preset: &Option<String>,
) -> Result<(), (ErrorCode, String)> {
	if let Some(Some(update)) = track
		&& update.encoded.is_some()
		&& update.identifier.is_some()
	{
		return Err((
			ErrorCode::ConflictingTrackFields,
			"Cannot specify both encodedTrack and identifier".to_string(),
		));
	}

	if let Some(state) = voice
		&& (state.token.is_empty() || state.endpoint.is_empty() || state.session_id.is_empty())
	{
		return Err((
			ErrorCode::InvalidVoiceState,
			"token, endpoint and sessionId must be provided in voice state".to_string(),
		));
	}

	if let Some(Some(value)) = end_time
		&& *value <= 0
	{
		return Err((ErrorCode::InvalidEndTime, "End time must be greater than 0".to_string()));
	}

	if let Some(filters) = filters
		&& let Err(reason) = crate::audio_filters::validate_filters(filters)
	{
		return Err((ErrorCode::InvalidFilterValue, reason));
	}

	if filters.is_some() && filter_preset.is_some() {
		return Err((
			ErrorCode::ConflictingTrackFields,
			"Cannot specify both filters and filterPreset".to_string(),
		));
	}

	Ok(())
}

fn merge_track_update(
	track: Option<Option<UpdatePlayerTrack>>,
	encoded_track: Option<Option<String>>,
	legacy_identifier: Option<String>,
) -> Result<Option<Option<UpdatePlayerTrack>>, String> {
	if track.is_some() && (encoded_track.is_some() || legacy_identifier.is_some()) {
		return Err("Cannot specify both track and encodedTrack/identifier".to_string());
	}

	if track.is_some() {
		return Ok(track);
	}

	match (encoded_track, legacy_identifier) {
		(None, None) => Ok(None),
		(Some(None), None) => Ok(Some(None)),
		(Some(encoded), identifier) => Ok(Some(Some(UpdatePlayerTrack {
			encoded,
			identifier,
			user_data: None,
		}))),
		(None, Some(identifier)) => Ok(Some(Some(UpdatePlayerTrack {
			encoded: None,
			identifier: Some(identifier),
			user_data: None,
		}))),
	}
}

pub async fn update_player(
	req: HttpRequest,
	path: Path<(String, String)>,
	params: Query<NoReplaceQuery>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
	update: Json<PlayerUpdate>,
) -> Result<Json<PlayerResponse>, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	let update = update.into_inner();
	let opts = update.player_options;

	let track_update = match merge_track_update(opts.track, opts.encoded_track, opts.legacy_identifier) {
		Ok(track_update) => track_update,
		Err(message) => return Err(RestError::new(ErrorCode::ConflictingTrackFields, Some(message), req.path())),
	};

	if let Err((code, message)) =
		validate_patch(&track_update, &opts.voice, &opts.end_time, &opts.filters, &opts.filter_preset)
	{
		return Err(RestError::new(code, Some(message), req.path()));
	}

	let no_replace = merge_no_replace(params.no_replace, opts.no_replace);
	let end_time = opts.end_time.flatten().filter(|value| *value > 0).map(|value| value as u64);

	let preset_filters = match opts.filter_preset.as_deref() {
		Some(name) => Some(state.filter_presets.get(name).ok_or_else(|| {
			RestError::not_found(
				ErrorCode::FilterPresetNotFound,
				format!("Filter preset not found: {name}"),
				&req,
			)
		})?),
		None => None,
	};

	let user_id = ws_state.sessions.get(&session_id).map(|session| session.user_id).unwrap_or(0);

	let player_lock = state.player_manager.get_or_create(guild_id).await;

	let mut play_request: Option<(Track, u64, u32, Filters, Option<u64>)> = None;
	let mut stop_request = false;
	let mut pause_request: Option<bool> = None;
	let mut seek_request: Option<u64> = None;
	let mut volume_request: Option<u32> = None;
	let mut filters_request: Option<Filters> = None;
	let mut voice_request: Option<VoiceState> = None;
	let mut changed = false;
	let mut invalidate_prefetch = false;

	let snapshot = {
		let mut player = player_lock.write().await;

		if let Some(volume) = opts.volume {
			let clamped = volume.max(0) as u32;
			player.set_volume(clamped);
			volume_request = Some(player.volume);
			changed = true;
		}

		if let Some(position) = opts.position {
			let clamped = position.max(0) as u64;
			player.seek(clamped);
			seek_request = Some(clamped);
			changed = true;
		}

		if let Some(paused) = opts.paused {
			if paused {
				player.pause();
			} else {
				player.play();
			}

			pause_request = Some(paused);
			changed = true;
		}

		let mut started_new_playback = false;

		match track_update {
			Some(Some(track_update)) => {
				let UpdatePlayerTrack {
					encoded,
					identifier,
					user_data,
				} = track_update;
				let user_data = user_data.unwrap_or(serde_json::json!({}));

				let mut resolved = None;

				if let Some(encoded) = encoded {
					match orpheus_track_resolver::decode_track(&encoded) {
						Ok(decoded) => {
							let mut track = decoded.track;

							track.user_data = user_data;
							resolved = Some(track);
						}
						Err(e) => {
							tracing::warn!(
								"Track update for guild {} — ignoring undecodable track: {}; keeping current track",
								guild_id,
								e
							);
						}
					}
				} else if let Some(identifier) = identifier {
					match state.track_resolver.load(&identifier).await {
						Ok((result, _)) => {
							if let Err((code, message)) = ensure_playable(&result.load_type) {
								return Err(RestError::new(code, Some(message), req.path()));
							}

							if let Some(mut track) = result.tracks.into_iter().next() {
								track.user_data = user_data;
								resolved = Some(track);
							} else {
								tracing::warn!(
									"Track update for guild {} — identifier resolved no tracks; keeping current track",
									guild_id
								);
							}
						}
						Err(e) => {
							tracing::warn!(
								"Track update for guild {} — failed to load identifier: {}; keeping current track",
								guild_id,
								e
							);
						}
					}
				} else if let Some(current) = player.current.as_mut() {
					current.user_data = user_data;
					changed = true;
				}

				if let Some(mut track) = resolved {
					if opts.end_time.is_some() {
						track.end_time = end_time;
					}

					let track_end = track.end_time;

					changed = true;
					tracing::debug!("Track resolved: title={:?} uri={:?}", track.info.title, track.info.uri);

					if no_replace {
						player.add_track(track);
						invalidate_prefetch = true;
					} else {
						started_new_playback = true;
						invalidate_prefetch = true;
						player.current = Some(track.clone());

						let has_voice = player.voice_state.is_some() || opts.voice.is_some();

						tracing::debug!(
							"Track update for guild {}: has_voice_state={}, player_state={:?}",
							guild_id,
							has_voice,
							player.state
						);

						if player.state != PlayerState::Paused {
							if track.playable_uri().is_some() {
								if !has_voice {
									tracing::info!(
										"Track set for guild {} before voice connected — queuing until bot joins",
										guild_id
									);
								}

								let seek_base = seek_request
									.or_else(|| state.voice_connection_manager.current_position(guild_id))
									.unwrap_or(player.position);

								play_request = Some((track, seek_base, player.volume, player.filters.clone(), track_end));
							} else {
								tracing::warn!("Track has empty URI, cannot play for guild {}", guild_id);
							}
						}
					}
				}
			}
			Some(None) => {
				player.stop();
				stop_request = true;
				changed = true;
				invalidate_prefetch = true;
			}
			None => {}
		}

		if !stop_request
			&& !started_new_playback
			&& let Some(marker) = opts.end_time
			&& let Some(current) = player.current.as_mut()
		{
			current.end_time = marker.map(|value| value.max(0) as u64);
			changed = true;
		}

		if let Some(voice) = opts.voice.clone() {
			player.voice_state = Some(voice.clone());
			voice_request = Some(voice);
			changed = true;
		}

		if voice_request.is_some()
			&& play_request.is_none()
			&& let Some(track) = player.current.clone()
			&& let Some(position) = ws_state.take_restart_resume(guild_id)
		{
			player.seek(position);
			player.play();
			let end_time = track.end_time;
			play_request = Some((track, position, player.volume, player.filters.clone(), end_time));
			changed = true;
		}

		if let Some(incoming) = preset_filters.or(opts.filters.clone()) {
			player.set_filters(incoming.clone());
			filters_request = Some(incoming);
			changed = true;
		}

		player_response(
			&player,
			player.guild_id,
			live_position(&state, player.guild_id, player.position),
			state.voice_connection_manager.is_connected(player.guild_id),
		)
	};

	if invalidate_prefetch {
		state.prefetch.invalidate(guild_id);
	}

	let voice_mgr = &state.voice_connection_manager;

	if let Some(requested) = volume_request {
		voice_mgr.set_volume(guild_id, requested).await;
	}

	if let Some(requested) = seek_request {
		voice_mgr.seek(guild_id, requested).await;
	}

	if let Some(requested) = pause_request {
		voice_mgr.pause(guild_id, requested).await;
	}

	if let Some(requested) = filters_request {
		voice_mgr.set_filters(guild_id, requested).await;
	}

	if let Some(voice) = voice_request {
		voice_mgr.connect(guild_id, user_id, voice).await;
	}

	if let Some((track, seek_position, play_volume, play_filters, play_end)) = play_request {
		let playback_uri = state.track_resolver.playback_url(&track).await;

		let mut voice_track = track;

		voice_track.info.uri = Some(playback_uri);

		tracing::info!(
			"Starting audio for guild {} via voice manager: uri={:?}",
			guild_id,
			voice_track.info.uri
		);
		voice_mgr
			.play(guild_id, voice_track, seek_position, play_volume, play_filters, play_end)
			.await;
	} else if stop_request {
		voice_mgr.stop(guild_id).await;
	}

	if changed {
		state.voice_connection_manager.request_player_update(guild_id);
	}

	Ok(Json(snapshot))
}

pub async fn delete_player(
	req: HttpRequest,
	path: Path<(String, String)>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
) -> Result<HttpResponse, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	state.prefetch.invalidate(guild_id);
	state.voice_connection_manager.destroy(guild_id).await;
	state.player_manager.remove(guild_id).await;

	Ok(HttpResponse::NoContent().finish())
}

#[cfg(test)]
mod tests {
	use orpheus_core::{EqualizerBand, TimescaleFilter};

	use super::*;

	fn valid_voice() -> VoiceState {
		VoiceState {
			token: "token".to_string(),
			endpoint: "endpoint".to_string(),
			session_id: "session".to_string(),
			channel_id: Some("7".to_string()),
		}
	}

	#[test]
	fn patch_rejects_encoded_and_identifier_together() {
		let track = Some(Some(UpdatePlayerTrack {
			encoded: Some("abc".to_string()),
			identifier: Some("ytsearch:test".to_string()),
			user_data: None,
		}));

		let result = validate_patch(&track, &None, &None, &None, &None);

		assert_eq!(
			result,
			Err((
				ErrorCode::ConflictingTrackFields,
				"Cannot specify both encodedTrack and identifier".to_string()
			))
		);
	}

	#[test]
	fn flat_track_fields_deserialize() {
		let encoded: PlayerUpdate = serde_json::from_str(r#"{"encodedTrack": "abc"}"#).unwrap();

		assert!(encoded.player_options.track.is_none());
		assert_eq!(encoded.player_options.encoded_track, Some(Some("abc".to_string())));
		assert!(encoded.player_options.legacy_identifier.is_none());

		let nulled: PlayerUpdate = serde_json::from_str(r#"{"encodedTrack": null}"#).unwrap();

		assert_eq!(nulled.player_options.encoded_track, Some(None));

		let identifier: PlayerUpdate = serde_json::from_str(r#"{"identifier": "ytsearch:test"}"#).unwrap();

		assert_eq!(identifier.player_options.legacy_identifier.as_deref(), Some("ytsearch:test"));
		assert!(identifier.player_options.encoded_track.is_none());
	}

	fn nested_encoded_track() -> Option<Option<UpdatePlayerTrack>> {
		Some(Some(UpdatePlayerTrack {
			encoded: Some("abc".to_string()),
			identifier: None,
			user_data: None,
		}))
	}

	#[test]
	fn track_update_rejects_nested_and_flat_together() {
		let error = merge_track_update(nested_encoded_track(), Some(Some("flat".to_string())), None).unwrap_err();

		assert_eq!(error, "Cannot specify both track and encodedTrack/identifier");

		let error = merge_track_update(nested_encoded_track(), None, Some("ytsearch:test".to_string())).unwrap_err();

		assert_eq!(error, "Cannot specify both track and encodedTrack/identifier");
	}

	#[test]
	fn flat_track_fields_merge_into_track_update() {
		let merged = merge_track_update(None, Some(Some("abc".to_string())), None).unwrap();

		match merged {
			Some(Some(update)) => {
				assert_eq!(update.encoded.as_deref(), Some("abc"));
				assert!(update.identifier.is_none());
			}
			other => panic!("expected flat encoded track, got {other:?}"),
		}

		let merged = merge_track_update(None, None, Some("ytsearch:test".to_string())).unwrap();

		match merged {
			Some(Some(update)) => {
				assert!(update.encoded.is_none());
				assert_eq!(update.identifier.as_deref(), Some("ytsearch:test"));
			}
			other => panic!("expected flat identifier track, got {other:?}"),
		}

		assert!(matches!(merge_track_update(None, Some(None), None).unwrap(), Some(None)));
		assert!(merge_track_update(None, None, None).unwrap().is_none());
		assert!(merge_track_update(nested_encoded_track(), None, None).unwrap().is_some());
	}

	#[test]
	fn patch_rejects_non_positive_end_time() {
		for value in [0, -1, -60_000] {
			let result = validate_patch(&None, &None, &Some(Some(value)), &None, &None);

			assert_eq!(
				result,
				Err((ErrorCode::InvalidEndTime, "End time must be greater than 0".to_string()))
			);
		}

		assert!(validate_patch(&None, &None, &None, &None, &None).is_ok());
		assert!(validate_patch(&None, &None, &Some(None), &None, &None).is_ok());
		assert!(validate_patch(&None, &None, &Some(Some(1)), &None, &None).is_ok());
	}

	#[test]
	fn patch_rejects_voice_state_without_required_fields() {
		let mut missing_token = valid_voice();
		missing_token.token.clear();

		let mut missing_endpoint = valid_voice();
		missing_endpoint.endpoint.clear();

		let mut missing_session = valid_voice();
		missing_session.session_id.clear();

		for voice in [missing_token, missing_endpoint, missing_session] {
			let result = validate_patch(&None, &Some(voice), &None, &None, &None);

			assert_eq!(
				result,
				Err((
					ErrorCode::InvalidVoiceState,
					"token, endpoint and sessionId must be provided in voice state".to_string()
				))
			);
		}

		assert!(validate_patch(&None, &None, &None, &None, &None).is_ok());
		assert!(validate_patch(&None, &Some(valid_voice()), &None, &None, &None).is_ok());
	}

	#[test]
	fn patch_accepts_lavalink_voice_state_without_channel_id() {
		let voice: VoiceState = serde_json::from_str(r#"{"token":"t","endpoint":"e","sessionId":"s"}"#).unwrap();

		assert!(voice.channel_id.is_none());
		assert!(validate_patch(&None, &Some(voice), &None, &None, &None).is_ok());
	}

	#[test]
	fn patch_rejects_invalid_filter_values() {
		let bad_band = Filters {
			equalizer: Some(vec![EqualizerBand { band: 99, gain: 0.5 }]),
			..Filters::default()
		};

		let result = validate_patch(&None, &None, &None, &Some(bad_band), &None);

		assert_eq!(
			result,
			Err((
				ErrorCode::InvalidFilterValue,
				"equalizer band 99 out of range 0-14".to_string()
			))
		);

		let bad_speed = Filters {
			timescale: Some(TimescaleFilter {
				speed: 0.0,
				pitch: 1.0,
				rate: 1.0,
			}),
			..Filters::default()
		};

		let result = validate_patch(&None, &None, &None, &Some(bad_speed), &None);

		assert!(matches!(result, Err((ErrorCode::InvalidFilterValue, _))));

		let both = validate_patch(&None, &None, &None, &Some(Filters::default()), &Some("bass".to_string()));

		assert_eq!(
			both,
			Err((
				ErrorCode::ConflictingTrackFields,
				"Cannot specify both filters and filterPreset".to_string()
			))
		);

		let good = Filters {
			normalization: Some(true),
			..Filters::default()
		};

		assert!(validate_patch(&None, &None, &None, &Some(good), &None).is_ok());
		assert!(validate_patch(&None, &None, &None, &None, &None).is_ok());
		assert!(validate_patch(&None, &None, &None, &None, &Some("bass".to_string())).is_ok());
	}

	#[test]
	fn no_replace_query_wins_over_body() {
		assert!(!merge_no_replace(None, None));
		assert!(merge_no_replace(None, Some(true)));
		assert!(!merge_no_replace(None, Some(false)));
		assert!(merge_no_replace(Some(true), None));
		assert!(merge_no_replace(Some(true), Some(false)));
		assert!(!merge_no_replace(Some(false), Some(true)));
	}

	#[test]
	fn track_field_distinguishes_omitted_from_explicit_null() {
		let omitted: PlayerUpdate = serde_json::from_str("{}").unwrap();

		assert!(omitted.player_options.track.is_none());

		let explicit_null: PlayerUpdate = serde_json::from_str(r#"{"track": null}"#).unwrap();

		assert!(explicit_null.player_options.track.is_some());
		assert!(matches!(explicit_null.player_options.track, Some(None)));

		let empty_object: PlayerUpdate = serde_json::from_str(r#"{"track": {}}"#).unwrap();

		assert!(matches!(empty_object.player_options.track, Some(Some(_))));

		let with_encoded: PlayerUpdate = serde_json::from_str(r#"{"track": {"encoded": "abc"}}"#).unwrap();

		assert!(matches!(with_encoded.player_options.track, Some(Some(_))));
	}

	#[test]
	fn end_time_field_distinguishes_omitted_null_and_value() {
		let omitted: PlayerUpdate = serde_json::from_str("{}").unwrap();

		assert!(omitted.player_options.end_time.is_none());

		let cleared: PlayerUpdate = serde_json::from_str(r#"{"endTime": null}"#).unwrap();

		assert_eq!(cleared.player_options.end_time, Some(None));

		let valued: PlayerUpdate = serde_json::from_str(r#"{"endTime": 60000}"#).unwrap();

		assert_eq!(valued.player_options.end_time, Some(Some(60_000)));
	}

	#[test]
	fn no_replace_body_field_deserializes() {
		let update: PlayerUpdate = serde_json::from_str(r#"{"noReplace": true}"#).unwrap();

		assert_eq!(update.player_options.no_replace, Some(true));
	}

	#[test]
	fn voice_channel_id_accepts_string_or_number() {
		let voice: VoiceState =
			serde_json::from_str(r#"{"token": "t", "endpoint": "e", "sessionId": "s", "channelId": 123}"#).unwrap();

		assert_eq!(voice.channel_id.as_deref(), Some("123"));
	}
}
