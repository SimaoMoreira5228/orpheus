use std::time::{SystemTime, UNIX_EPOCH};

use orpheus_core::{Filters, Track};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum OutgoingMessage {
	#[serde(rename = "ready")]
	Ready {
		#[serde(rename = "sessionId")]
		session_id: String,
		resumed: bool,
		#[serde(rename = "sourceManagers", default)]
		source_managers: Vec<String>,
	},
	#[serde(rename = "playerUpdate")]
	PlayerUpdate {
		#[serde(rename = "guildId")]
		guild_id: String,
		state: PlayerState,
	},
	#[serde(rename = "stats")]
	Stats(Stats),
	#[serde(rename = "event")]
	Event(Box<Event>),
	#[serde(rename = "loadtracks")]
	LoadTracks(LoadTracksResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
	pub time: i64,
	pub position: i64,
	pub connected: bool,
	pub ping: i64,
}

impl PlayerState {
	pub fn snapshot(position_ms: i64, connected: bool) -> Self {
		Self {
			time: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64,
			position: position_ms,
			connected,
			// NOTE: the voice transport exposes no RTT measurement; -1 is Lavalink's own
			// sentinel for unknown ping, and lavalink-rs maps it back to None.
			ping: -1,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
	pub players: usize,
	#[serde(rename = "playingPlayers")]
	pub playing_players: usize,
	pub uptime: u64,
	pub memory: Memory,
	pub cpu: Cpu,
	#[serde(rename = "frameStats")]
	pub frame_stats: Option<FrameStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
	pub free: u64,
	pub used: u64,
	pub allocated: u64,
	pub reservable: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cpu {
	pub cores: i32,
	pub system_load: f64,
	pub lavalink_load: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameStats {
	pub sent: i64,
	pub nulled: i64,
	pub deficit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
	#[serde(rename = "TrackStartEvent")]
	TrackStart(TrackStartEvent),
	#[serde(rename = "TrackEndEvent")]
	TrackEnd(TrackEndEvent),
	#[serde(rename = "TrackExceptionEvent")]
	TrackException(TrackExceptionEvent),
	#[serde(rename = "TrackStuckEvent")]
	TrackStuck(TrackStuckEvent),
	#[serde(rename = "WebSocketClosedEvent")]
	WebSocketClosed(WebSocketClosedEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackStartEvent {
	pub track: Track,
	#[serde(rename = "guildId")]
	pub guild_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackEndEvent {
	pub track: Track,
	pub reason: TrackEndReason,
	#[serde(rename = "guildId")]
	pub guild_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackEndReason {
	Finished,
	LoadFailed,
	Stopped,
	Replaced,
	Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ExceptionSeverity {
	Common,
	Suspicious,
	Fault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackExceptionEvent {
	pub track: Track,
	pub exception: Exception,
	#[serde(rename = "guildId")]
	pub guild_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exception {
	pub message: String,
	pub severity: ExceptionSeverity,
	pub cause: String,
	#[serde(rename = "causeStackTrace")]
	pub cause_stack_trace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackStuckEvent {
	pub track: Track,
	#[serde(rename = "thresholdMs")]
	pub threshold_ms: i64,
	#[serde(rename = "guildId")]
	pub guild_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketClosedEvent {
	pub code: i32,
	pub reason: String,
	#[serde(rename = "byRemote")]
	pub by_remote: bool,
	#[serde(rename = "guildId")]
	pub guild_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum IncomingMessage {
	#[serde(rename = "ping")]
	Ping {
		#[serde(rename = "timestamp")]
		timestamp: i64,
	},
	#[serde(rename = "destroy")]
	Destroy {
		#[serde(rename = "guildId")]
		guild_id: String,
	},
	#[serde(rename = "configureResuming")]
	ConfigureResuming {
		#[serde(rename = "key")]
		key: Option<String>,
		#[serde(rename = "timeout")]
		timeout: Option<i32>,
	},
	#[serde(rename = "playerUpdate")]
	PlayerUpdate {
		#[serde(rename = "guildId")]
		guild_id: String,
		#[serde(rename = "data")]
		data: Box<PlayerUpdateData>,
	},
	#[serde(rename = "voiceUpdate")]
	VoiceUpdate {
		#[serde(rename = "guildId")]
		guild_id: String,
		#[serde(rename = "data")]
		data: VoiceUpdateData,
	},
	#[serde(rename = "loadtracks")]
	LoadTracks {
		identifier: String,
	},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerUpdateTrackData {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub encoded: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub identifier: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerUpdateData {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub track: Option<PlayerUpdateTrackData>,
	#[serde(rename = "position", default)]
	pub position: Option<i64>,
	#[serde(rename = "endTime", default)]
	pub end_time: Option<i64>,
	#[serde(rename = "volume", default)]
	pub volume: Option<f64>,
	#[serde(rename = "paused", default)]
	pub paused: Option<bool>,
	#[serde(default)]
	pub voice: Option<VoiceUpdateData>,
	#[serde(rename = "filters", default)]
	pub filters: Option<Filters>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceUpdateData {
	#[serde(rename = "token")]
	pub token: String,
	#[serde(rename = "endpoint")]
	pub endpoint: String,
	#[serde(rename = "sessionId")]
	pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerUpdatePayload {
	pub position: Option<f64>,
	pub volume: Option<f64>,
	pub paused: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUpdatePayload {
	pub resuming: Option<bool>,
	pub timeout: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LoadTracksResponseData {
	Track(Box<Track>),
	Playlist(PlaylistTracksDataMsg),
	Search(Vec<Track>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistTracksDataMsg {
	pub info: PlaylistInfoMsg,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub plugin_info: Option<serde_json::Value>,
	pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadTracksResponse {
	#[serde(rename = "loadType")]
	pub load_type: String,
	pub data: LoadTracksResponseData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfoMsg {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(rename = "selectedTrack", default, skip_serializing_if = "Option::is_none")]
	pub selected_track: Option<i64>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn player_update_matches_lavalink_shape() {
		let update = OutgoingMessage::PlayerUpdate {
			guild_id: "42".to_string(),
			state: PlayerState::snapshot(12_000, true),
		};

		let json = serde_json::to_value(&update).unwrap();

		assert_eq!(json["op"], "playerUpdate");
		assert_eq!(json["guildId"], "42");
		assert_eq!(json["state"]["connected"], true);
		assert_eq!(json["state"]["ping"], -1);
		assert!(json["state"]["position"].is_i64());
		assert_eq!(json["state"]["position"], 12_000);
		assert!(json["state"]["time"].is_i64());
	}

	#[test]
	fn stats_matches_lavalink_shape() {
		let stats = OutgoingMessage::Stats(Stats {
			players: 2,
			playing_players: 1,
			uptime: 90_000,
			memory: Memory {
				free: 10,
				used: 20,
				allocated: 30,
				reservable: 30,
			},
			cpu: Cpu {
				cores: 4,
				system_load: 0.5,
				lavalink_load: 0.25,
			},
			frame_stats: None,
		});

		let json = serde_json::to_value(&stats).unwrap();

		assert_eq!(json["op"], "stats");
		assert_eq!(json["players"], 2);
		assert_eq!(json["playingPlayers"], 1);
		assert_eq!(json["uptime"], 90_000);
		assert_eq!(json["memory"]["reservable"], 30);
		assert!(json["memory"].get("reserved").is_none());
		assert_eq!(json["cpu"]["cores"], 4);
		assert!(json["frameStats"].is_null());
	}
}
