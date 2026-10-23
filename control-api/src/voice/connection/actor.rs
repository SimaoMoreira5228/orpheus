use std::sync::Arc;

use dashmap::DashMap;
use discordrs::voice::VoiceSpeakingFlags;
use orpheus_core::{Filters, Track};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info};

use super::super::diagnostics::{VoiceDiagnosticKind, VoiceDiagnostics};
use super::super::manager::{VoiceCommand, VoiceProgress};
use super::{
	LiveConnection, PlaybackSettings, ProgressReport, apply_progress, connect_voice, endpoint_label, resolve_channel_id,
	restart_settings, safe_voice_error, start_stream, stream_active, teardown_live,
};
use crate::voice_events::VoiceEvent;
use crate::websocket_messages::TrackEndReason;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_voice_connection(
	guild_id: u64,
	mut rx: mpsc::Receiver<VoiceCommand>,
	loopback: mpsc::Sender<VoiceCommand>,
	ffmpeg_path: String,
	channels: Arc<DashMap<u64, u64>>,
	event_tx: broadcast::Sender<VoiceEvent>,
	progress_tx: broadcast::Sender<VoiceProgress>,
	live_voice: Arc<DashMap<u64, ()>>,
	live_positions: Arc<DashMap<u64, u64>>,
	player_update_tx: broadcast::Sender<u64>,
	diagnostics: Arc<VoiceDiagnostics>,
) {
	info!("Starting voice connection handler for guild {}", guild_id);
	let mut live: Option<LiveConnection> = None;
	let mut pending: Option<PlaybackSettings> = None;
	let mut current: Option<Track> = None;
	let mut paused = false;
	let mut suspended = false;
	let mut idle = true;
	let mut position_ms = 0u64;
	let mut volume = 100u32;
	let mut filters = Filters::default();
	let mut generation = 0u64;

	while let Some(cmd) = rx.recv().await {
		match cmd {
			VoiceCommand::Connect(user_id, state) => {
				let endpoint = state.endpoint.replace("wss://", "").replace("https://", "");
				diagnostics.record(
					guild_id,
					VoiceDiagnosticKind::ConnectStarted,
					Some(endpoint_label(&endpoint)),
					None,
					None,
					None,
				);

				if let Some(current_conn) = live.as_ref() {
					if current_conn.session_id == state.session_id && current_conn.endpoint == endpoint {
						debug!(
							"Already connected with same session_id={}, skipping duplicate Connect",
							state.session_id
						);
						continue;
					}

					debug!(
						"Reconnecting with new session (old={}, new={})",
						current_conn.session_id, state.session_id
					);
					teardown_live(&mut live, guild_id, &diagnostics).await;
				}

				let channel_id = resolve_channel_id(&channels, guild_id, &state);
				diagnostics.record(
					guild_id,
					VoiceDiagnosticKind::ConnectStarted,
					Some(endpoint_label(&endpoint)),
					None,
					None,
					None,
				);
				match connect_voice(guild_id, user_id, &state, channel_id).await {
					Ok(conn) => {
						let runtime_state = conn.runtime.state();
						let dave_version = runtime_state
							.session_description
							.as_ref()
							.and_then(|description| description.dave_protocol_version)
							.unwrap_or(0);
						diagnostics.record(
							guild_id,
							VoiceDiagnosticKind::Connected,
							Some(endpoint_label(&endpoint)),
							Some(dave_version),
							None,
							Some(format!("mode={:?}", runtime_state.selected_mode)),
						);

						live = Some(conn);
						live_voice.insert(guild_id, ());
						let _ = player_update_tx.send(guild_id);

						if let Some(settings) = pending.take() {
							if paused {
								pending = Some(settings);
							} else {
								let _ = event_tx.send(VoiceEvent::Started {
									guild_id,
									track: settings.track.clone(),
								});
								current = Some(settings.track.clone());
								position_ms = settings.seek_position;
								live_positions.insert(guild_id, position_ms);
								idle = false;
								generation += 1;
								start_stream(
									live.as_mut().expect("just set"),
									guild_id,
									&settings,
									&ffmpeg_path,
									&event_tx,
									&diagnostics,
									&loopback,
									generation,
								)
								.await;
							}
						}
					}
					Err(e) => {
						live_voice.remove(&guild_id);
						live_positions.remove(&guild_id);
						diagnostics.record(
							guild_id,
							VoiceDiagnosticKind::ConnectFailed,
							Some(endpoint_label(&endpoint)),
							None,
							None,
							Some(safe_voice_error(&e.to_string(), &state)),
						);
						error!("Failed to connect to voice gateway: {}", e);
					}
				}
			}
			VoiceCommand::Reconnect => {
				let live_conn = match live.as_ref() {
					Some(conn) => (conn.user_id, conn.voice_state.clone()),
					None => continue,
				};
				let channel_id = resolve_channel_id(&channels, guild_id, &live_conn.1);
				if live.as_ref().map(|conn| conn.channel_id) == Some(channel_id) {
					continue;
				}

				info!("Reconnecting guild {} with voice channel {}", guild_id, channel_id);
				diagnostics.record(
					guild_id,
					VoiceDiagnosticKind::ReconnectStarted,
					Some(endpoint_label(&live_conn.1.endpoint)),
					None,
					None,
					None,
				);
				teardown_live(&mut live, guild_id, &diagnostics).await;
				live_voice.remove(&guild_id);
				live_positions.remove(&guild_id);
				match connect_voice(guild_id, live_conn.0, &live_conn.1, channel_id).await {
					Ok(conn) => {
						let runtime_state = conn.runtime.state();
						let dave_version = runtime_state
							.session_description
							.as_ref()
							.and_then(|description| description.dave_protocol_version)
							.unwrap_or(0);
						diagnostics.record(
							guild_id,
							VoiceDiagnosticKind::Connected,
							Some(endpoint_label(&live_conn.1.endpoint)),
							Some(dave_version),
							None,
							Some(format!("mode={:?}", runtime_state.selected_mode)),
						);

						live = Some(conn);
						live_voice.insert(guild_id, ());
						let _ = player_update_tx.send(guild_id);

						if let Some(settings) = pending.take() {
							if paused {
								pending = Some(settings);
							} else {
								let _ = event_tx.send(VoiceEvent::Started {
									guild_id,
									track: settings.track.clone(),
								});
								current = Some(settings.track.clone());
								position_ms = settings.seek_position;
								live_positions.insert(guild_id, position_ms);
								idle = false;
								generation += 1;
								start_stream(
									live.as_mut().expect("just set"),
									guild_id,
									&settings,
									&ffmpeg_path,
									&event_tx,
									&diagnostics,
									&loopback,
									generation,
								)
								.await;
							}
						}
					}
					Err(e) => {
						diagnostics.record(
							guild_id,
							VoiceDiagnosticKind::ConnectFailed,
							Some(endpoint_label(&live_conn.1.endpoint)),
							None,
							None,
							Some(safe_voice_error(&e.to_string(), &live_conn.1)),
						);
						error!("Failed to reconnect voice for guild {}: {}", guild_id, e);
					}
				}
			}
			VoiceCommand::Play {
				track,
				seek_position,
				volume: play_volume,
				filters: play_filters,
				end_time,
			} => {
				let track = *track;
				position_ms = seek_position;
				live_positions.insert(guild_id, position_ms);
				volume = play_volume;
				filters = play_filters.clone();
				idle = false;
				suspended = false;

				let settings = PlaybackSettings {
					track: track.clone(),
					seek_position,
					volume: play_volume,
					filters: play_filters,
					end_time: end_time.or(track.end_time),
				};

				let Some(conn) = live.as_mut() else {
					info!("Play for guild {} arrived before voice connected — queuing", guild_id);
					pending = Some(settings);
					continue;
				};

				if pending.take().is_some() {
					debug!("Dropping stale pending play for guild {}", guild_id);
				}

				if paused {
					pending = Some(settings);
					current = Some(track);
					continue;
				}

				if stream_active(conn) {
					if let Some(old) = current.replace(track.clone()) {
						let _ = event_tx.send(VoiceEvent::Ended {
							guild_id,
							track: old,
							reason: TrackEndReason::Replaced,
						});
					}
				} else {
					current = Some(track.clone());
				}

				let _ = event_tx.send(VoiceEvent::Started {
					guild_id,
					track: track.clone(),
				});
				generation += 1;
				start_stream(
					conn,
					guild_id,
					&settings,
					&ffmpeg_path,
					&event_tx,
					&diagnostics,
					&loopback,
					generation,
				)
				.await;
			}
			VoiceCommand::Stop => {
				pending = None;
				generation += 1;
				idle = true;
				suspended = false;
				live_positions.remove(&guild_id);

				if let Some(conn) = live.as_mut() {
					let active = stream_active(conn);

					if active && let Some(task) = conn.stream_task.take() {
						task.abort();
						let _ = task.await;
					}

					let _ = conn.runtime.set_speaking(VoiceSpeakingFlags::default(), 0);

					if active && let Some(track) = current.take() {
						let _ = event_tx.send(VoiceEvent::Ended {
							guild_id,
							track,
							reason: TrackEndReason::Stopped,
						});
					}
				}
			}
			VoiceCommand::Pause(next_paused) => {
				paused = next_paused;

				if !paused {
					let Some(conn) = live.as_mut() else {
						continue;
					};

					if let Some(settings) = pending.take() {
						position_ms = settings.seek_position;
						live_positions.insert(guild_id, position_ms);
						current = Some(settings.track.clone());
						idle = false;
						suspended = false;
						let _ = event_tx.send(VoiceEvent::Started {
							guild_id,
							track: settings.track.clone(),
						});
						generation += 1;
						start_stream(
							conn,
							guild_id,
							&settings,
							&ffmpeg_path,
							&event_tx,
							&diagnostics,
							&loopback,
							generation,
						)
						.await;
					} else if !idle && !stream_active(conn) {
						if let Some(track) = current.clone() {
							let end_time = track.end_time;
							let settings = PlaybackSettings {
								track,
								seek_position: position_ms,
								volume,
								filters: filters.clone(),
								end_time,
							};
							let silent = suspended;
							suspended = false;
							idle = false;

							if !silent {
								let _ = event_tx.send(VoiceEvent::Started {
									guild_id,
									track: settings.track.clone(),
								});
							}

							generation += 1;
							start_stream(
								conn,
								guild_id,
								&settings,
								&ffmpeg_path,
								&event_tx,
								&diagnostics,
								&loopback,
								generation,
							)
							.await;
						}
					} else {
						suspended = false;
					}

					continue;
				}

				suspended = false;

				if let Some(conn) = live.as_mut()
					&& stream_active(conn)
				{
					suspended = true;

					if let Some(task) = conn.stream_task.take() {
						task.abort();
						let _ = task.await;
					}

					let _ = conn.runtime.set_speaking(VoiceSpeakingFlags::default(), 0);
				}
			}
			VoiceCommand::Seek(position) => {
				position_ms = position;
				live_positions.insert(guild_id, position);

				if let Some(settings) = pending.as_mut() {
					settings.seek_position = position;
				}

				if paused {
					continue;
				}

				if let Some(conn) = live.as_mut() {
					let active = stream_active(conn);

					if let Some(settings) =
						restart_settings(current.as_ref(), position_ms, volume, &filters, paused, idle, active)
					{
						generation += 1;
						start_stream(
							conn,
							guild_id,
							&settings,
							&ffmpeg_path,
							&event_tx,
							&diagnostics,
							&loopback,
							generation,
						)
						.await;
					}
				}
			}
			VoiceCommand::SetVolume(next_volume) => {
				volume = next_volume;

				if let Some(settings) = pending.as_mut() {
					settings.volume = next_volume;
				}

				if paused {
					continue;
				}

				if let Some(conn) = live.as_mut() {
					let active = stream_active(conn);

					if let Some(settings) =
						restart_settings(current.as_ref(), position_ms, volume, &filters, paused, idle, active)
					{
						generation += 1;
						start_stream(
							conn,
							guild_id,
							&settings,
							&ffmpeg_path,
							&event_tx,
							&diagnostics,
							&loopback,
							generation,
						)
						.await;
					}
				}
			}
			VoiceCommand::SetFilters(next_filters) => {
				filters = next_filters.clone();

				if let Some(settings) = pending.as_mut() {
					settings.filters = next_filters;
				}

				if paused {
					continue;
				}

				if let Some(conn) = live.as_mut() {
					let active = stream_active(conn);

					if let Some(settings) =
						restart_settings(current.as_ref(), position_ms, volume, &filters, paused, idle, active)
					{
						generation += 1;
						start_stream(
							conn,
							guild_id,
							&settings,
							&ffmpeg_path,
							&event_tx,
							&diagnostics,
							&loopback,
							generation,
						)
						.await;
					}
				}
			}
			VoiceCommand::StreamEnded { generation: finished } => {
				if finished == generation {
					idle = true;
					suspended = false;
					live_positions.remove(&guild_id);
				}
			}
			VoiceCommand::Progress {
				generation: reported_generation,
				position_ms: reported_position,
			} => {
				let next = apply_progress(
					generation,
					position_ms,
					ProgressReport {
						generation: reported_generation,
						position_ms: reported_position,
					},
				);

				if next != position_ms {
					position_ms = next;
					live_positions.insert(guild_id, next);
					let _ = progress_tx.send(VoiceProgress {
						guild_id,
						generation: reported_generation,
						position_ms: next,
					});
				}
			}
			VoiceCommand::Disconnect(disconnected) => {
				let was_streaming = live.as_ref().map(stream_active).unwrap_or(false);
				info!("Disconnecting voice for guild {}", guild_id);
				teardown_live(&mut live, guild_id, &diagnostics).await;
				live_voice.remove(&guild_id);
				live_positions.remove(&guild_id);
				let _ = player_update_tx.send(guild_id);

				if was_streaming && let Some(track) = current.take() {
					let _ = event_tx.send(VoiceEvent::Ended {
						guild_id,
						track,
						reason: TrackEndReason::Cleanup,
					});
				}
				let _ = disconnected.send(());

				break;
			}
		}
	}
}
