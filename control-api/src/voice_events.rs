use orpheus_core::Track;

use crate::websocket_messages::{
	Event, Exception, ExceptionSeverity, OutgoingMessage, TrackEndEvent, TrackEndReason, TrackExceptionEvent,
	TrackStartEvent, TrackStuckEvent,
};

#[derive(Debug, Clone)]
pub enum VoiceEvent {
	Started {
		guild_id: u64,
		track: Track,
	},
	Ended {
		guild_id: u64,
		track: Track,
		reason: TrackEndReason,
	},
	Exception {
		guild_id: u64,
		track: Track,
		exception: StreamException,
	},
	Stuck {
		guild_id: u64,
		track: Track,
		threshold_ms: u64,
	},
}

impl VoiceEvent {
	pub fn guild_id(&self) -> u64 {
		match self {
			VoiceEvent::Started { guild_id, .. } => *guild_id,
			VoiceEvent::Ended { guild_id, .. } => *guild_id,
			VoiceEvent::Exception { guild_id, .. } => *guild_id,
			VoiceEvent::Stuck { guild_id, .. } => *guild_id,
		}
	}
}

#[derive(Debug, Clone)]
pub struct StreamException {
	pub message: String,
	pub severity: ExceptionSeverity,
	pub cause: String,
}

#[derive(Debug, Clone)]
pub enum StreamOutcome {
	Finished,
	SpawnFailed {
		message: String,
	},
	NoAudio {
		cause: String,
	},
	EncoderFailed {
		message: String,
	},
	DaveTimeout,
	ReadFailed {
		message: String,
	},
	EncodeFailed {
		message: String,
	},
	SendFailed {
		message: String,
	},
	Stuck,
}

pub fn severity_for_outcome(outcome: &StreamOutcome) -> Option<ExceptionSeverity> {
	match outcome {
		StreamOutcome::Finished | StreamOutcome::Stuck => None,
		StreamOutcome::NoAudio { .. } => Some(ExceptionSeverity::Common),
		StreamOutcome::DaveTimeout | StreamOutcome::ReadFailed { .. } | StreamOutcome::SendFailed { .. } => {
			Some(ExceptionSeverity::Suspicious)
		}
		StreamOutcome::SpawnFailed { .. } | StreamOutcome::EncoderFailed { .. } | StreamOutcome::EncodeFailed { .. } => {
			Some(ExceptionSeverity::Fault)
		}
	}
}

fn cause_for_outcome(outcome: &StreamOutcome) -> &'static str {
	match outcome {
		StreamOutcome::Finished | StreamOutcome::Stuck => "playback",
		StreamOutcome::SpawnFailed { .. } => "ffmpeg spawn",
		StreamOutcome::NoAudio { .. } => "ffmpeg output",
		StreamOutcome::EncoderFailed { .. } => "opus encoder",
		StreamOutcome::DaveTimeout => "dave handshake",
		StreamOutcome::ReadFailed { .. } => "pcm read",
		StreamOutcome::EncodeFailed { .. } => "opus encode",
		StreamOutcome::SendFailed { .. } => "discord send",
	}
}

fn message_for_outcome(outcome: &StreamOutcome) -> String {
	match outcome {
		StreamOutcome::Finished => "finished".to_string(),
		StreamOutcome::Stuck => "track stuck".to_string(),
		StreamOutcome::SpawnFailed { message } => format!("failed to spawn ffmpeg: {message}"),
		StreamOutcome::NoAudio { cause } => format!("stream produced no audio: {cause}"),
		StreamOutcome::EncoderFailed { message } => format!("failed to create opus encoder: {message}"),
		StreamOutcome::DaveTimeout => "dave session never became ready".to_string(),
		StreamOutcome::ReadFailed { message } => format!("failed to read ffmpeg output: {message}"),
		StreamOutcome::EncodeFailed { message } => format!("failed to encode opus frame: {message}"),
		StreamOutcome::SendFailed { message } => format!("failed to send audio frame: {message}"),
	}
}

pub fn events_for_outcome(
	guild_id: u64,
	track: &Track,
	outcome: &StreamOutcome,
	stuck_threshold_ms: u64,
) -> Vec<VoiceEvent> {
	let exception = || {
		let severity = severity_for_outcome(outcome).expect("failure outcome without severity");

		VoiceEvent::Exception {
			guild_id,
			track: track.clone(),
			exception: StreamException {
				message: message_for_outcome(outcome),
				severity,
				cause: cause_for_outcome(outcome).to_string(),
			},
		}
	};

	let ended = |reason: TrackEndReason| VoiceEvent::Ended {
		guild_id,
		track: track.clone(),
		reason,
	};

	match outcome {
		StreamOutcome::Finished => vec![ended(TrackEndReason::Finished)],
		StreamOutcome::Stuck => vec![
			VoiceEvent::Stuck {
				guild_id,
				track: track.clone(),
				threshold_ms: stuck_threshold_ms,
			},
			ended(TrackEndReason::Finished),
		],
		StreamOutcome::SpawnFailed { .. }
		| StreamOutcome::NoAudio { .. }
		| StreamOutcome::EncoderFailed { .. }
		| StreamOutcome::DaveTimeout => {
			vec![exception(), ended(TrackEndReason::LoadFailed)]
		}
		StreamOutcome::ReadFailed { .. } | StreamOutcome::EncodeFailed { .. } | StreamOutcome::SendFailed { .. } => {
			vec![exception(), ended(TrackEndReason::Finished)]
		}
	}
}

pub fn voice_event_to_outgoing(event: &VoiceEvent) -> OutgoingMessage {
	let guild_id = event.guild_id().to_string();

	let inner = match event {
		VoiceEvent::Started { track, .. } => Event::TrackStart(TrackStartEvent {
			track: track.clone(),
			guild_id,
		}),
		VoiceEvent::Ended { track, reason, .. } => Event::TrackEnd(TrackEndEvent {
			track: track.clone(),
			reason: reason.clone(),
			guild_id,
		}),
		VoiceEvent::Exception { track, exception, .. } => Event::TrackException(TrackExceptionEvent {
			track: track.clone(),
			exception: Exception {
				message: exception.message.clone(),
				severity: exception.severity,
				cause: exception.cause.clone(),
				cause_stack_trace: None,
			},
			guild_id,
		}),
		VoiceEvent::Stuck { track, threshold_ms, .. } => Event::TrackStuck(TrackStuckEvent {
			track: track.clone(),
			threshold_ms: *threshold_ms as i64,
			guild_id,
		}),
	};

	OutgoingMessage::Event(Box::new(inner))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fixture_track() -> Track {
		Track {
			encoded: Some("QAAAjQIAKE5vIFRpdGxlACtObyBBdXRob3IA".to_string()),
			info: orpheus_core::TrackInfo {
				identifier: "https://example.invalid/audio.opus".to_string(),
				is_seekable: false,
				author: "No Author".to_string(),
				length: 210000,
				is_stream: false,
				position: 0,
				title: "No Title".to_string(),
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
	fn end_reasons_match_lavalink_wire_values() {
		let cases = [
			(TrackEndReason::Finished, "\"finished\""),
			(TrackEndReason::LoadFailed, "\"loadFailed\""),
			(TrackEndReason::Stopped, "\"stopped\""),
			(TrackEndReason::Replaced, "\"replaced\""),
			(TrackEndReason::Cleanup, "\"cleanup\""),
		];

		for (reason, wire) in cases {
			assert_eq!(serde_json::to_string(&reason).unwrap(), wire);
		}
	}

	#[test]
	fn severities_match_lavalink_wire_values() {
		let cases = [
			(ExceptionSeverity::Common, "\"COMMON\""),
			(ExceptionSeverity::Suspicious, "\"SUSPICIOUS\""),
			(ExceptionSeverity::Fault, "\"FAULT\""),
		];

		for (severity, wire) in cases {
			assert_eq!(serde_json::to_string(&severity).unwrap(), wire);
		}
	}

	#[test]
	fn every_outcome_maps_to_a_terminal_event() {
		let track = fixture_track();

		let outcomes: Vec<(StreamOutcome, usize)> = vec![
			(StreamOutcome::Finished, 1),
			(
				StreamOutcome::SpawnFailed {
					message: "boom".to_string(),
				},
				2,
			),
			(
				StreamOutcome::NoAudio {
					cause: "empty".to_string(),
				},
				2,
			),
			(
				StreamOutcome::EncoderFailed {
					message: "boom".to_string(),
				},
				2,
			),
			(StreamOutcome::DaveTimeout, 2),
			(
				StreamOutcome::ReadFailed {
					message: "boom".to_string(),
				},
				2,
			),
			(
				StreamOutcome::EncodeFailed {
					message: "boom".to_string(),
				},
				2,
			),
			(
				StreamOutcome::SendFailed {
					message: "boom".to_string(),
				},
				2,
			),
			(StreamOutcome::Stuck, 2),
		];

		for (outcome, len) in &outcomes {
			let events = events_for_outcome(7, &track, outcome, 5000);

			assert_eq!(events.len(), *len);
			assert!(events.iter().all(|e| e.guild_id() == 7));
		}
	}

	#[test]
	fn unplayable_streams_end_load_failed_with_fault_exception() {
		let track = fixture_track();

		let outcome = StreamOutcome::SpawnFailed {
			message: "no such file".to_string(),
		};

		let events = events_for_outcome(7, &track, &outcome, 5000);

		let VoiceEvent::Exception { exception, .. } = &events[0] else {
			panic!("expected exception first");
		};

		assert_eq!(exception.severity, ExceptionSeverity::Fault);

		let VoiceEvent::Ended { reason, .. } = &events[1] else {
			panic!("expected end second");
		};

		assert!(matches!(reason, TrackEndReason::LoadFailed));
	}

	#[test]
	fn mid_stream_failures_end_finished_with_mapped_severity() {
		let track = fixture_track();

		let cases = [
			(
				StreamOutcome::ReadFailed {
					message: "reset".to_string(),
				},
				ExceptionSeverity::Suspicious,
			),
			(
				StreamOutcome::EncodeFailed {
					message: "bad pcm".to_string(),
				},
				ExceptionSeverity::Fault,
			),
			(
				StreamOutcome::SendFailed {
					message: "gone".to_string(),
				},
				ExceptionSeverity::Suspicious,
			),
		];

		for (outcome, severity) in cases {
			let events = events_for_outcome(9, &track, &outcome, 5000);

			let VoiceEvent::Exception { exception, .. } = &events[0] else {
				panic!("expected exception first");
			};

			assert_eq!(exception.severity, severity);

			let VoiceEvent::Ended { reason, .. } = &events[1] else {
				panic!("expected end second");
			};

			assert!(matches!(reason, TrackEndReason::Finished));
		}
	}

	#[test]
	fn stuck_carries_threshold_and_ends_finished() {
		let track = fixture_track();

		let events = events_for_outcome(3, &track, &StreamOutcome::Stuck, 5000);

		let VoiceEvent::Stuck { threshold_ms, .. } = &events[0] else {
			panic!("expected stuck first");
		};

		assert_eq!(*threshold_ms, 5000);

		let VoiceEvent::Ended { reason, .. } = &events[1] else {
			panic!("expected end second");
		};

		assert!(matches!(reason, TrackEndReason::Finished));
	}

	#[test]
	fn event_payloads_serialize_to_lavalink_shape() {
		let track = fixture_track();

		let started = voice_event_to_outgoing(&VoiceEvent::Started {
			guild_id: 42,
			track: track.clone(),
		});

		assert_eq!(
			serde_json::to_string(&started).unwrap(),
			"{\"op\":\"event\",\"type\":\"TrackStartEvent\",\"track\":{\"encoded\":\"QAAAjQIAKE5vIFRpdGxlACtObyBBdXRob3IA\",\"info\":{\"identifier\":\"https://example.invalid/audio.opus\",\"isSeekable\":false,\"author\":\"No Author\",\"length\":210000,\"isStream\":false,\"position\":0,\"title\":\"No Title\",\"uri\":\"https://example.invalid/audio.opus\",\"sourceName\":\"http\"},\"pluginInfo\":{},\"userData\":{}},\"guildId\":\"42\"}"
		);

		let ended = voice_event_to_outgoing(&VoiceEvent::Ended {
			guild_id: 42,
			track: track.clone(),
			reason: TrackEndReason::Replaced,
		});

		let ended_json = serde_json::to_value(&ended).unwrap();

		assert_eq!(ended_json["op"], "event");
		assert_eq!(ended_json["type"], "TrackEndEvent");
		assert_eq!(ended_json["guildId"], "42");
		assert_eq!(ended_json["reason"], "replaced");
		assert_eq!(ended_json["track"]["encoded"], "QAAAjQIAKE5vIFRpdGxlACtObyBBdXRob3IA");

		let exception = voice_event_to_outgoing(&VoiceEvent::Exception {
			guild_id: 42,
			track: track.clone(),
			exception: StreamException {
				message: "failed to read ffmpeg output: reset".to_string(),
				severity: ExceptionSeverity::Suspicious,
				cause: "pcm read".to_string(),
			},
		});

		let exception_json = serde_json::to_value(&exception).unwrap();

		assert_eq!(exception_json["op"], "event");
		assert_eq!(exception_json["type"], "TrackExceptionEvent");
		assert_eq!(exception_json["guildId"], "42");
		assert_eq!(exception_json["exception"]["severity"], "SUSPICIOUS");
		assert_eq!(exception_json["track"]["encoded"], "QAAAjQIAKE5vIFRpdGxlACtObyBBdXRob3IA");

		let stuck = voice_event_to_outgoing(&VoiceEvent::Stuck {
			guild_id: 42,
			track: track.clone(),
			threshold_ms: 5000,
		});

		let stuck_json = serde_json::to_value(&stuck).unwrap();

		assert_eq!(stuck_json["op"], "event");
		assert_eq!(stuck_json["type"], "TrackStuckEvent");
		assert_eq!(stuck_json["guildId"], "42");
		assert_eq!(stuck_json["thresholdMs"], 5000);
	}
}
