use std::sync::Arc;

use dashmap::DashMap;
use discordrs::voice_runtime::{self, VoiceRuntimeConfig, VoiceRuntimeHandle};
use discordrs::{GuildId, UserId};
use orpheus_core::{Filters, Track, VoiceState};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use super::diagnostics::{VoiceDiagnosticKind, VoiceDiagnostics};
use super::manager::VoiceCommand;
use crate::audio_stream::{STUCK_TIMEOUT, stream_audio};
use crate::voice_dave::DaveController;
use crate::voice_events::{StreamOutcome, VoiceEvent, events_for_outcome};

mod actor;

pub(crate) use actor::run_voice_connection;

#[derive(Debug, Clone)]
struct PlaybackSettings {
	track: Track,
	seek_position: u64,
	volume: u32,
	filters: Filters,
	end_time: Option<u64>,
}

fn restart_settings(
	current: Option<&Track>,
	position_ms: u64,
	volume: u32,
	filters: &Filters,
	paused: bool,
	idle: bool,
	stream_active: bool,
) -> Option<PlaybackSettings> {
	if paused || idle || !stream_active {
		return None;
	}

	current.map(|track| PlaybackSettings {
		track: track.clone(),
		seek_position: position_ms,
		volume,
		filters: filters.clone(),
		end_time: track.end_time,
	})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProgressReport {
	generation: u64,
	position_ms: u64,
}

fn apply_progress(current_generation: u64, current_position: u64, report: ProgressReport) -> u64 {
	if report.generation != current_generation {
		return current_position;
	}

	report.position_ms
}

struct LiveConnection {
	runtime: Arc<VoiceRuntimeHandle>,
	controller: Arc<tokio::sync::Mutex<DaveController>>,
	stream_task: Option<tokio::task::JoinHandle<()>>,
	user_id: u64,
	voice_state: VoiceState,
	channel_id: u64,
	session_id: String,
	endpoint: String,
}

async fn teardown_live(live: &mut Option<LiveConnection>, guild_id: u64, diagnostics: &VoiceDiagnostics) {
	if let Some(conn) = live.take() {
		if let Some(task) = conn.stream_task {
			task.abort();
			let _ = task.await;
		}
		match Arc::try_unwrap(conn.runtime) {
			Ok(runtime) => {
				if let Err(e) = runtime.close().await {
					warn!("Voice disconnect for guild {} failed: {}", guild_id, e);
					diagnostics.record(
						guild_id,
						VoiceDiagnosticKind::Disconnected,
						Some(endpoint_label(&conn.endpoint)),
						None,
						None,
						Some("voice runtime close failed".to_string()),
					);
				} else {
					info!("Voice disconnected for guild {}", guild_id);
					diagnostics.record(
						guild_id,
						VoiceDiagnosticKind::Disconnected,
						Some(endpoint_label(&conn.endpoint)),
						None,
						None,
						None,
					);
				}
			}
			Err(_) => {
				warn!("Voice runtime for guild {} still referenced, skipping close", guild_id);
				diagnostics.record(
					guild_id,
					VoiceDiagnosticKind::Disconnected,
					Some(endpoint_label(&conn.endpoint)),
					None,
					None,
					Some("voice runtime remained referenced during close".to_string()),
				);
			}
		}
	}
}

async fn connect_voice(
	guild_id: u64,
	user_id: u64,
	state: &VoiceState,
	channel_id: u64,
) -> Result<LiveConnection, discordrs::DiscordError> {
	let guild = GuildId::try_from(guild_id).map_err(|_| discordrs::DiscordError::model("invalid guild id"))?;
	let user = UserId::try_from(user_id).map_err(|_| discordrs::DiscordError::model("invalid user id"))?;
	let endpoint = state.endpoint.replace("wss://", "").replace("https://", "");

	let config = VoiceRuntimeConfig::new(guild, user, state.session_id.clone(), state.token.clone(), endpoint.clone())
		.max_dave_protocol_version(davey::DAVE_PROTOCOL_VERSION as u8);

	let runtime = Arc::new(voice_runtime::connect(config).await?);
	let runtime_state = runtime.state();
	let dave_version = runtime_state
		.session_description
		.as_ref()
		.and_then(|d| d.dave_protocol_version)
		.unwrap_or(0);
	info!(
		"Voice guild {} connected: mode={:?} dave_version={}",
		guild_id, runtime_state.selected_mode, dave_version
	);

	Ok(LiveConnection {
		runtime,
		controller: Arc::new(tokio::sync::Mutex::new(DaveController::new(user_id, channel_id))),
		stream_task: None,
		user_id,
		voice_state: state.clone(),
		channel_id,
		session_id: state.session_id.clone(),
		endpoint,
	})
}

fn resolve_channel_id(channels: &DashMap<u64, u64>, guild_id: u64, voice_state: &VoiceState) -> u64 {
	channels
		.get(&guild_id)
		.map(|channel| *channel)
		.or_else(|| voice_state.channel_id.as_ref().and_then(|c| c.parse::<u64>().ok()))
		.unwrap_or(guild_id)
}

fn stream_active(conn: &LiveConnection) -> bool {
	conn.stream_task.as_ref().map(|task| !task.is_finished()).unwrap_or(false)
}

fn endpoint_label(endpoint: &str) -> String {
	endpoint.split('/').next().unwrap_or("unknown").to_string()
}

fn safe_voice_error(error: &str, voice_state: &VoiceState) -> String {
	let mut detail = error.to_string();
	for secret in [&voice_state.token, &voice_state.session_id] {
		if !secret.is_empty() {
			detail = detail.replace(secret, "[redacted]");
		}
	}

	detail.chars().take(256).collect()
}

fn stream_outcome_label(outcome: &StreamOutcome) -> Option<&'static str> {
	match outcome {
		StreamOutcome::Finished => None,
		StreamOutcome::SpawnFailed { .. } => Some("ffmpeg spawn failed"),
		StreamOutcome::NoAudio { .. } => Some("ffmpeg produced no audio"),
		StreamOutcome::EncoderFailed { .. } => Some("opus encoder failed"),
		StreamOutcome::DaveTimeout => Some("dave readiness timed out"),
		StreamOutcome::ReadFailed { .. } => Some("ffmpeg output read failed"),
		StreamOutcome::EncodeFailed { .. } => Some("opus encoding failed"),
		StreamOutcome::SendFailed { .. } => Some("discord audio send failed"),
		StreamOutcome::Stuck => Some("audio stream stopped producing frames"),
	}
}

#[allow(clippy::too_many_arguments)]
async fn start_stream(
	conn: &mut LiveConnection,
	guild_id: u64,
	settings: &PlaybackSettings,
	ffmpeg_path: &str,
	event_tx: &broadcast::Sender<VoiceEvent>,
	diagnostics: &Arc<VoiceDiagnostics>,
	loopback: &mpsc::Sender<VoiceCommand>,
	generation: u64,
) {
	if let Some(task) = conn.stream_task.take() {
		task.abort();
		let _ = task.await;
	}

	let runtime = Arc::clone(&conn.runtime);
	let controller = Arc::clone(&conn.controller);
	let dave_version = runtime
		.state()
		.session_description
		.as_ref()
		.and_then(|d| d.dave_protocol_version)
		.unwrap_or(0);
	let ffmpeg = ffmpeg_path.to_string();
	let diagnostics = Arc::clone(diagnostics);
	let track = settings.track.clone();
	let seek_position = settings.seek_position;
	let volume = settings.volume;
	let filters = settings.filters.clone();
	let end_time = settings.end_time;
	let tx = event_tx.clone();
	let loopback = loopback.clone();
	let progress_tx = loopback.clone();
	diagnostics.record(
		guild_id,
		VoiceDiagnosticKind::StreamStarted,
		Some(endpoint_label(&conn.endpoint)),
		Some(dave_version),
		None,
		None,
	);
	conn.stream_task = Some(tokio::spawn(async move {
		let outcome = stream_audio(
			runtime,
			controller,
			diagnostics.clone(),
			guild_id,
			dave_version,
			track.info.uri.clone().unwrap_or_default(),
			seek_position,
			volume,
			filters,
			end_time,
			ffmpeg,
			progress_tx,
			generation,
		)
		.await;

		diagnostics.record(
			guild_id,
			if matches!(&outcome, StreamOutcome::Finished) {
				VoiceDiagnosticKind::StreamEnded
			} else {
				VoiceDiagnosticKind::StreamFailed
			},
			None,
			Some(dave_version),
			None,
			stream_outcome_label(&outcome).map(str::to_string),
		);

		let _ = loopback.send(VoiceCommand::StreamEnded { generation }).await;
		for event in events_for_outcome(guild_id, &track, &outcome, STUCK_TIMEOUT.as_millis() as u64) {
			let _ = tx.send(event);
		}
	}));
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
mod tests {
	use super::*;

	fn fixture_track() -> Track {
		Track {
			encoded: None,
			info: orpheus_core::TrackInfo {
				identifier: "https://example.invalid/audio.opus".to_string(),
				is_seekable: false,
				author: "author".to_string(),
				length: 210000,
				is_stream: false,
				position: 0,
				title: "title".to_string(),
				uri: Some("https://example.invalid/audio.opus".to_string()),
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

	#[test]
	fn voice_channel_binding_supplies_channel_id_when_voice_state_omits_it() {
		let channels = DashMap::new();
		channels.insert(7, 42);
		let voice_state = VoiceState {
			token: "token".to_string(),
			endpoint: "endpoint".to_string(),
			session_id: "session".to_string(),
			channel_id: None,
		};

		assert_eq!(resolve_channel_id(&channels, 7, &voice_state), 42);
	}

	#[test]
	fn restart_applies_seek_position_volume_and_filters_to_live_stream() {
		let track = fixture_track();
		let filters = Filters {
			tremolo: Some(orpheus_core::TremoloFilter {
				frequency: 2.0,
				depth: 0.5,
			}),
			..Filters::default()
		};

		let settings = restart_settings(Some(&track), 65_000, 250, &filters, false, false, true).expect("restart");

		assert_eq!(settings.seek_position, 65_000);
		assert_eq!(settings.volume, 250);
		assert_eq!(settings.track.info.uri, track.info.uri);
		assert!(settings.filters.tremolo.is_some());
	}

	#[test]
	fn restart_stays_silent_without_a_live_stream() {
		let track = fixture_track();
		let filters = Filters::default();

		assert!(restart_settings(Some(&track), 10_000, 100, &filters, false, true, false).is_none());
		assert!(restart_settings(Some(&track), 10_000, 100, &filters, false, false, false).is_none());
		assert!(restart_settings(None, 10_000, 100, &filters, false, false, true).is_none());
	}

	#[test]
	fn restart_defers_while_paused() {
		let track = fixture_track();

		assert!(restart_settings(Some(&track), 10_000, 100, &Filters::default(), true, false, true).is_none());
	}

	#[test]
	fn progress_report_advances_live_position() {
		let report = ProgressReport {
			generation: 3,
			position_ms: 61_000,
		};

		assert_eq!(apply_progress(3, 60_000, report), 61_000);
	}

	#[test]
	fn stale_generation_report_leaves_position_untouched() {
		let report = ProgressReport {
			generation: 2,
			position_ms: 90_000,
		};

		assert_eq!(apply_progress(3, 61_000, report), 61_000);
	}

	#[test]
	fn stored_progress_carries_into_resume_settings() {
		let stored = apply_progress(
			1,
			10_000,
			ProgressReport {
				generation: 1,
				position_ms: 60_000,
			},
		);

		let settings =
			restart_settings(Some(&fixture_track()), stored, 100, &Filters::default(), false, false, true).expect("restart");

		assert_eq!(settings.seek_position, 60_000);
	}
}
