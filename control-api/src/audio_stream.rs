use std::sync::Arc;
use std::time::Duration;

use discordrs::voice::VoiceSpeakingFlags;
use discordrs::voice_runtime::VoiceRuntimeHandle;
use orpheus_core::Filters;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tracing::{debug, error, info, warn};

use crate::voice::{VoiceCommand, VoiceDiagnosticKind, VoiceDiagnostics};
use crate::voice_dave::DaveController;
use crate::voice_events::StreamOutcome;

pub const STUCK_TIMEOUT: Duration = Duration::from_secs(5);
pub const DAVE_READY_TIMEOUT: Duration = Duration::from_secs(15);
pub const FRAME_DURATION_MS: u64 = 20;
pub const PROGRESS_EVERY_FRAMES: u64 = 50;

pub fn stop_bound_ms(end_time: Option<u64>, seek_position: u64) -> Option<u64> {
	end_time.map(|end| end.saturating_sub(seek_position).max(20))
}

pub fn progress_position_ms(seek_base_ms: u64, frames_sent: u64) -> u64 {
	seek_base_ms.saturating_add(frames_sent.saturating_mul(FRAME_DURATION_MS))
}

fn record_dave_epoch(
	diagnostics: &VoiceDiagnostics,
	guild_id: u64,
	dave_version: u8,
	last_epoch: &mut Option<u64>,
	epoch: Option<u64>,
) {
	let Some(epoch) = epoch else {
		return;
	};
	if *last_epoch == Some(epoch) {
		return;
	}

	diagnostics.record(
		guild_id,
		VoiceDiagnosticKind::DaveEpoch,
		None,
		Some(dave_version),
		Some(epoch),
		None,
	);
	*last_epoch = Some(epoch);
}

const SILENCE_FRAME: &[u8] = &[0xF8, 0xFF, 0xFE];

async fn send_frame(
	runtime: &VoiceRuntimeHandle,
	controller: &Arc<tokio::sync::Mutex<DaveController>>,
	dave_version: u8,
	opus: &[u8],
) -> Result<(), discordrs::DiscordError> {
	if dave_version > 0 {
		let mut guard = controller.lock().await;
		let Some(session) = guard.session_mut() else {
			return Err(discordrs::DiscordError::model("no DAVE session"));
		};
		if !session.is_ready() {
			return Err(discordrs::DiscordError::model("DAVE session not ready"));
		}
		runtime
			.send_opus_frame_with_dave(opus, Duration::from_millis(20), session)
			.await?;
	} else {
		runtime.send_opus_frame(opus, Duration::from_millis(20)).await?;
	}

	Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn stream_audio(
	runtime: Arc<VoiceRuntimeHandle>,
	controller: Arc<tokio::sync::Mutex<DaveController>>,
	diagnostics: Arc<VoiceDiagnostics>,
	guild_id: u64,
	dave_version: u8,
	track_uri: String,
	seek_position: u64,
	volume: u32,
	filters: Filters,
	end_time: Option<u64>,
	ffmpeg_path: String,
	progress_tx: tokio::sync::mpsc::Sender<VoiceCommand>,
	generation: u64,
) -> StreamOutcome {
	let stop_at_ms = stop_bound_ms(end_time, seek_position);

	let mut child = match Command::new(&ffmpeg_path)
		.args(crate::audio_filters::ffmpeg_input_args(
			&track_uri,
			seek_position,
			volume,
			&filters,
			stop_at_ms,
		))
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.kill_on_drop(true)
		.spawn()
	{
		Ok(child) => child,
		Err(e) => {
			return StreamOutcome::SpawnFailed { message: e.to_string() };
		}
	};

	if let Some(stderr) = child.stderr.take() {
		tokio::spawn(async move {
			use tokio::io::AsyncBufReadExt;
			let mut lines = tokio::io::BufReader::new(stderr).lines();
			while let Ok(Some(line)) = lines.next_line().await {
				debug!("[ffmpeg] {}", line);
			}
		});
	}

	let mut stdout = match child.stdout.take() {
		Some(stdout) => stdout,
		None => {
			return StreamOutcome::NoAudio {
				cause: "ffmpeg stdout unavailable".to_string(),
			};
		}
	};

	let mut encoder = match audiopus::coder::Encoder::new(
		audiopus::SampleRate::Hz48000,
		audiopus::Channels::Stereo,
		audiopus::Application::Audio,
	) {
		Ok(encoder) => encoder,
		Err(e) => {
			return StreamOutcome::EncoderFailed { message: e.to_string() };
		}
	};

	if let Err(e) = encoder.set_bitrate(audiopus::Bitrate::BitsPerSecond(128000)) {
		warn!("Failed to set Opus bitrate: {}", e);
	}

	let mut dave_watch = runtime.subscribe();
	let mut dave_epoch = None;
	{
		let mut guard = controller.lock().await;
		if let Err(e) = guard.drive(&runtime).await {
			warn!("Initial DAVE drive failed: {}", e);
		}
		record_dave_epoch(&diagnostics, guild_id, dave_version, &mut dave_epoch, guard.joined_epoch());
	}

	if dave_version > 0 {
		let start = std::time::Instant::now();
		loop {
			let guard = controller.lock().await;
			let ready = guard.is_ready();
			record_dave_epoch(&diagnostics, guild_id, dave_version, &mut dave_epoch, guard.joined_epoch());
			drop(guard);

			if ready {
				break;
			}
			if start.elapsed() > DAVE_READY_TIMEOUT {
				warn!("DAVE session never became ready, aborting playback");
				let _ = child.kill().await;
				return StreamOutcome::DaveTimeout;
			}
			let _ = tokio::time::timeout(Duration::from_millis(100), dave_watch.changed()).await;
			let mut guard = controller.lock().await;
			if let Err(e) = guard.drive(&runtime).await {
				warn!("DAVE drive failed: {}", e);
			}
			record_dave_epoch(&diagnostics, guild_id, dave_version, &mut dave_epoch, guard.joined_epoch());
		}
	}

	if let Err(e) = runtime.set_speaking(VoiceSpeakingFlags::MICROPHONE, 0) {
		warn!("Failed to send speaking update: {}", e);
	}

	let mut last_frame_time = std::time::Instant::now();
	let mut frame_count: u64 = 0;
	let mut send_failures: u32 = 0;
	let mut interval = tokio::time::interval(Duration::from_millis(20));
	interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

	let frame_size = 960;
	let mut pcm_buf = vec![0i16; frame_size * 2];
	let mut byte_buf = vec![0u8; frame_size * 2 * 2];
	let mut output_buf = [0u8; 4000];

	const MAX_SEND_FAILURES: u32 = 50;

	let outcome = loop {
		if last_frame_time.elapsed() > STUCK_TIMEOUT && frame_count > 0 {
			warn!("Track stuck — no frames for {:?} after {} frames", STUCK_TIMEOUT, frame_count);
			break StreamOutcome::Stuck;
		}

		match dave_watch.has_changed() {
			Err(e) => {
				debug!("DAVE watch closed, ending stream: {}", e);
				break StreamOutcome::SendFailed {
					message: format!("voice runtime closed: {e}"),
				};
			}
			Ok(true) => {
				dave_watch.borrow_and_update();
				let mut guard = controller.lock().await;
				if let Err(e) = guard.drive(&runtime).await {
					warn!("DAVE drive failed: {}", e);
				}
				record_dave_epoch(&diagnostics, guild_id, dave_version, &mut dave_epoch, guard.joined_epoch());
			}
			Ok(false) => {}
		}

		match stdout.read_exact(&mut byte_buf).await {
			Ok(_) => {
				last_frame_time = std::time::Instant::now();

				for (sample, chunk) in pcm_buf.iter_mut().zip(byte_buf.as_chunks::<2>().0.iter()) {
					*sample = i16::from_le_bytes(*chunk);
				}
			}
			Err(e) => {
				if e.kind() == std::io::ErrorKind::UnexpectedEof {
					info!("End of stream");
				} else {
					error!("Read error: {}", e);
				}

				if frame_count == 0 {
					break StreamOutcome::NoAudio {
						cause: format!("ffmpeg exited before producing audio: {e}"),
					};
				}

				if e.kind() != std::io::ErrorKind::UnexpectedEof {
					break StreamOutcome::ReadFailed { message: e.to_string() };
				}

				break StreamOutcome::Finished;
			}
		}

		let encoded_len = match encoder.encode(&pcm_buf, &mut output_buf) {
			Ok(len) => len,
			Err(e) => {
				break StreamOutcome::EncodeFailed { message: e.to_string() };
			}
		};

		if encoded_len > 0 {
			match send_frame(&runtime, &controller, dave_version, &output_buf[..encoded_len]).await {
				Ok(_) => {
					send_failures = 0;
					frame_count += 1;

					if frame_count.is_multiple_of(PROGRESS_EVERY_FRAMES) {
						let _ = progress_tx.try_send(VoiceCommand::Progress {
							generation,
							position_ms: progress_position_ms(seek_position, frame_count),
						});
					}
				}
				Err(e) => {
					send_failures += 1;
					warn!("Failed to send audio: {}", e);

					if send_failures >= MAX_SEND_FAILURES {
						break StreamOutcome::SendFailed { message: e.to_string() };
					}
				}
			}
		}

		interval.tick().await;
	};

	if matches!(outcome, StreamOutcome::Finished) {
		for _ in 0..5 {
			let _ = send_frame(&runtime, &controller, dave_version, SILENCE_FRAME).await;
			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	}

	let _ = runtime.set_speaking(VoiceSpeakingFlags::default(), 0);
	let _ = child.kill().await;
	info!("Audio stream ended — sent {} frames", frame_count);

	if matches!(outcome, StreamOutcome::Finished) {
		let _ = progress_tx.try_send(VoiceCommand::Progress {
			generation,
			position_ms: progress_position_ms(seek_position, frame_count),
		});
	}

	outcome
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn stop_bound_counts_from_seek_position() {
		assert_eq!(stop_bound_ms(Some(60_000), 10_000), Some(50_000));
		assert_eq!(stop_bound_ms(None, 10_000), None);
	}

	#[test]
	fn stop_bound_never_collapses_to_zero() {
		assert_eq!(stop_bound_ms(Some(5_000), 10_000), Some(20));
		assert_eq!(stop_bound_ms(Some(10_000), 10_000), Some(20));
	}

	#[test]
	fn progress_accumulates_frames_from_seek_base() {
		assert_eq!(progress_position_ms(60_000, 50), 61_000);
		assert_eq!(progress_position_ms(0, 0), 0);
	}

	#[test]
	fn progress_saturates_instead_of_wrapping() {
		assert_eq!(progress_position_ms(u64::MAX, u64::MAX), u64::MAX);
	}
}
