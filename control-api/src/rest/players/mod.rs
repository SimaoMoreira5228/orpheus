use orpheus_core::{Filters, Track, VoiceState};
use serde::{Deserialize, Serialize};

use super::tracks::nullable_option;

mod read;
mod update;

pub use read::{get_all_players, get_player};
pub(crate) use read::{live_position, player_response};
pub use update::{NoReplaceQuery, delete_player, update_player};

#[derive(Debug, Serialize)]
pub struct PlayerStateResponse {
	pub time: u64,
	pub position: u64,
	pub connected: bool,
	pub ping: i64,
}

#[derive(Debug, Serialize)]
pub struct VoiceStateResponse {
	pub token: String,
	pub endpoint: String,
	#[serde(rename = "sessionId")]
	pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct PlayerResponse {
	#[serde(rename = "guildId")]
	pub guild_id: String,
	pub track: Option<Track>,
	pub volume: i32,
	pub paused: bool,
	pub state: PlayerStateResponse,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub voice: Option<VoiceStateResponse>,
	pub filters: Filters,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePlayerTrack {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub encoded: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub identifier: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user_data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct PlayerUpdate {
	#[serde(default, flatten)]
	pub player_options: PlayerOptions,
}

#[derive(Debug, Deserialize, Default)]
pub struct PlayerOptions {
	#[serde(default, deserialize_with = "nullable_option")]
	pub track: Option<Option<UpdatePlayerTrack>>,
	#[serde(default, rename = "encodedTrack", deserialize_with = "nullable_option")]
	pub encoded_track: Option<Option<String>>,
	#[serde(default, rename = "identifier")]
	pub legacy_identifier: Option<String>,
	#[serde(default, rename = "volume")]
	pub volume: Option<i32>,
	#[serde(default, rename = "position")]
	pub position: Option<i64>,
	#[serde(default, rename = "endTime", deserialize_with = "nullable_option")]
	pub end_time: Option<Option<i64>>,
	#[serde(default, rename = "paused")]
	pub paused: Option<bool>,
	#[serde(default, rename = "voice")]
	pub voice: Option<VoiceState>,
	#[serde(default)]
	pub filters: Option<Filters>,
	#[serde(default, rename = "filterPreset")]
	pub filter_preset: Option<String>,
	#[serde(default, rename = "noReplace")]
	pub no_replace: Option<bool>,
}
