use orpheus_core::{Filters, RepeatMode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
	pub identifier: String,
	pub is_seekable: bool,
	pub author: String,
	pub length: u64,
	pub is_stream: bool,
	pub position: u64,
	pub title: String,
	pub uri: Option<String>,
	pub artwork_url: Option<String>,
	pub isrc: Option<String>,
	pub source_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
	pub encoded: Option<String>,
	pub info: TrackInfo,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub start_time: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub end_time: Option<u64>,
	#[serde(default)]
	pub plugin_info: serde_json::Value,
	#[serde(default)]
	pub user_data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
	pub name: Option<String>,
	pub selected_track: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoadResultType {
	Track,
	Playlist,
	Search,
	Empty,
	Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LoadData {
	Track(Box<Track>),
	Playlist(PlaylistData),
	Search(Vec<Track>),
	Error(LoadFailureData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadFailureData {
	pub message: String,
	pub severity: String,
	pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistData {
	pub info: PlaylistInfo,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub plugin_info: Option<serde_json::Value>,
	pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunItem {
	#[serde(rename = "loadType")]
	pub load_type: LoadResultType,
	pub data: LoadData,
	#[serde(rename = "playbackUrl")]
	pub playback_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumRef {
	pub id: String,
	pub name: String,
	pub artist: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub artwork_url: Option<String>,
	pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistRef {
	pub id: String,
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub artwork_url: Option<String>,
	pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistRef {
	pub id: String,
	pub name: String,
	pub author: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub artwork_url: Option<String>,
	pub uri: String,
	pub track_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RichSearchResponse {
	#[serde(default)]
	pub tracks: Vec<Track>,
	#[serde(default)]
	pub albums: Vec<AlbumRef>,
	#[serde(default)]
	pub artists: Vec<ArtistRef>,
	#[serde(default)]
	pub playlists: Vec<PlaylistRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadResult {
	#[serde(rename = "loadType")]
	pub load_type: LoadResultType,
	pub data: LoadData,
}

impl LoadResult {
	pub fn first_track(&self) -> Option<&Track> {
		match &self.data {
			LoadData::Track(track) => Some(track.as_ref()),
			LoadData::Playlist(playlist) => playlist.tracks.first(),
			LoadData::Search(tracks) => tracks.first(),
			LoadData::Error(_) => None,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStateInfo {
	pub time: u64,
	pub position: u64,
	pub connected: bool,
	// NOTE: the server reports -1 for unknown ping, so this must stay signed.
	pub ping: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceStateInfo {
	pub token: String,
	pub endpoint: String,
	#[serde(rename = "sessionId")]
	pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
	#[serde(rename = "guildId")]
	pub guild_id: String,
	pub track: Option<Track>,
	pub volume: i32,
	pub paused: bool,
	pub state: PlayerStateInfo,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub voice: Option<VoiceStateInfo>,
	#[serde(default)]
	pub filters: Filters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerTrackUpdate {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub encoded: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub identifier: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user_data: Option<serde_json::Value>,
}

// NOTE: serde cannot tell omitted apart from explicit null for nested Option,
// so nullable PATCH fields need this instead of the derived impl.
fn nullable_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
	D: serde::Deserializer<'de>,
	T: serde::Deserialize<'de>,
{
	Ok(Some(Option::<T>::deserialize(deserializer)?))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStateUpdate {
	pub token: String,
	pub endpoint: String,
	pub session_id: String,
	#[serde(default, deserialize_with = "orpheus_core::string_or_number_opt")]
	pub channel_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdate {
	#[serde(default, deserialize_with = "nullable_option", skip_serializing_if = "Option::is_none")]
	pub track: Option<Option<PlayerTrackUpdate>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub position: Option<i64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub volume: Option<i32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub paused: Option<bool>,
	#[serde(default, deserialize_with = "nullable_option", skip_serializing_if = "Option::is_none")]
	pub end_time: Option<Option<u64>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub filters: Option<Filters>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub filter_preset: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub voice: Option<VoiceStateUpdate>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub no_replace: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueResponse {
	pub current: Option<Track>,
	pub queue: Vec<Track>,
	pub history: Vec<Track>,
	pub position: usize,
	pub repeat: RepeatMode,
	pub autoplay: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddToQueueRequest {
	pub tracks: Vec<Track>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub position: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueUpdateRequest {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub shuffle: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub repeat: Option<RepeatMode>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub position: Option<usize>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub autoplay: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuePositionResponse {
	pub track: Option<Track>,
	pub position: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SessionUpdate {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub resuming: Option<bool>,
	#[serde(default, rename = "timeout", skip_serializing_if = "Option::is_none")]
	pub timeout_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
	pub resuming: bool,
	#[serde(rename = "timeout")]
	pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeVersion {
	pub semver: String,
	pub major: i32,
	pub minor: i32,
	pub patch: i32,
	pub pre_release: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
	pub branch: String,
	pub commit: String,
	pub commit_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
	pub name: String,
	pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
	pub version: NodeVersion,
	pub build_time: u64,
	pub git: GitInfo,
	pub jvm: String,
	pub lavaplayer: String,
	#[serde(default)]
	pub source_managers: Vec<String>,
	#[serde(default)]
	pub filters: Vec<String>,
	#[serde(default)]
	pub plugins: Vec<PluginInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInfo {
	pub free: i64,
	pub used: i64,
	pub allocated: i64,
	pub reservable: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuInfo {
	pub cores: i32,
	pub system_load: f32,
	pub lavalink_load: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
	pub players: i32,
	pub playing_players: i32,
	pub uptime: i64,
	pub memory: MemoryInfo,
	pub cpu: CpuInfo,
	#[serde(default)]
	pub frame_stats: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
	pub version: String,
}

#[cfg(test)]
mod tests {
	use orpheus_core::PlayerSnapshot;

	use super::*;

	#[test]
	fn player_update_end_time_distinguishes_omitted_null_and_value() {
		let omitted: PlayerUpdate = serde_json::from_str("{}").unwrap();

		assert!(omitted.end_time.is_none());
		assert!(serde_json::to_value(&omitted).unwrap().get("endTime").is_none());

		let cleared: PlayerUpdate = serde_json::from_str(r#"{"endTime": null}"#).unwrap();

		assert_eq!(cleared.end_time, Some(None));
		assert!(serde_json::to_value(&cleared).unwrap()["endTime"].is_null());

		let valued: PlayerUpdate = serde_json::from_str(r#"{"endTime": 60000}"#).unwrap();

		assert_eq!(valued.end_time, Some(Some(60_000)));
	}

	#[test]
	fn player_update_track_distinguishes_omitted_from_explicit_null() {
		let omitted: PlayerUpdate = serde_json::from_str("{}").unwrap();

		assert!(omitted.track.is_none());

		let stopped: PlayerUpdate = serde_json::from_str(r#"{"track": null}"#).unwrap();

		assert!(matches!(stopped.track, Some(None)));
		assert!(serde_json::to_value(&stopped).unwrap()["track"].is_null());

		let started: PlayerUpdate = serde_json::from_str(r#"{"track": {"encoded": "abc"}}"#).unwrap();

		assert!(matches!(started.track, Some(Some(_))));
	}

	#[test]
	fn player_update_uses_camel_case_wire_names() {
		let update = PlayerUpdate {
			no_replace: Some(true),
			filters: Some(Filters::default()),
			filter_preset: Some("bass".to_string()),
			voice: Some(VoiceStateUpdate {
				token: "token".to_string(),
				endpoint: "endpoint".to_string(),
				session_id: "session".to_string(),
				channel_id: Some("7".to_string()),
			}),
			..Default::default()
		};
		let json = serde_json::to_value(&update).unwrap();

		assert_eq!(json["noReplace"], true);
		assert_eq!(json["filterPreset"], "bass");
		assert_eq!(json["voice"]["sessionId"], "session");
		assert_eq!(json["voice"]["channelId"], "7");
		assert!(json["filters"].is_object());
		assert!(json.get("endTime").is_none());

		let numeric_channel: VoiceStateUpdate =
			serde_json::from_str(r#"{"token": "t", "endpoint": "e", "sessionId": "s", "channelId": 123}"#).unwrap();

		assert_eq!(numeric_channel.channel_id.as_deref(), Some("123"));
	}

	#[test]
	fn player_state_matches_server_response_shape() {
		let state: PlayerState = serde_json::from_str(
			r#"{
				"guildId": "7", "track": null, "volume": 100, "paused": false,
				"state": {"time": 1, "position": 0, "connected": true, "ping": -1},
				"filters": {}
			}"#,
		)
		.unwrap();

		assert_eq!(state.guild_id, "7");
		assert_eq!(state.volume, 100);
		assert_eq!(state.state.ping, -1);
		assert!(state.voice.is_none());
	}

	#[test]
	fn queue_payloads_carry_autoplay_and_repeat() {
		let update = QueueUpdateRequest {
			autoplay: Some(true),
			..Default::default()
		};
		let json = serde_json::to_value(&update).unwrap();

		assert_eq!(json, serde_json::json!({ "autoplay": true }));

		let queue: QueueResponse = serde_json::from_str(
			r#"{"current": null, "queue": [], "history": [], "position": 0, "repeat": "Track", "autoplay": true}"#,
		)
		.unwrap();

		assert!(queue.autoplay);
		assert_eq!(queue.repeat, RepeatMode::Track);
	}

	#[test]
	fn player_snapshot_round_trips() {
		let snapshot: PlayerSnapshot = serde_json::from_str(
			r#"{"version": 1, "current": null, "queue": [], "history": [], "position": 0, "volume": 100, "filters": {}, "repeat": "Off", "shuffle": false, "shufflePositions": [], "autoplay": true}"#,
		)
		.unwrap();

		assert_eq!(snapshot.version, 1);
		assert!(snapshot.current.is_none());
		assert!(snapshot.autoplay);

		let back = serde_json::to_value(&snapshot).unwrap();

		assert_eq!(back["version"], 1);
		assert_eq!(back["shufflePositions"], serde_json::json!([]));
	}

	#[test]
	fn dry_run_item_carries_playback_url() {
		let item: DryRunItem = serde_json::from_str(
			r#"{"loadType": "track", "data": {"info": {"identifier": "x", "isSeekable": true, "author": "a", "length": 0, "isStream": false, "position": 0, "title": "t", "uri": "file:///x.mp3", "artworkUrl": null, "isrc": null, "sourceName": "local"}}, "playbackUrl": "file:///x.mp3"}"#,
		)
		.unwrap();

		assert_eq!(item.load_type, LoadResultType::Track);
		assert_eq!(item.playback_url.as_deref(), Some("file:///x.mp3"));

		let failed: DryRunItem = serde_json::from_str(
			r#"{"loadType": "error", "data": {"message": "No matches", "severity": "COMMON", "code": "NO_MATCHES"}, "playbackUrl": null}"#,
		)
		.unwrap();

		assert_eq!(failed.load_type, LoadResultType::Error);
		assert!(failed.playback_url.is_none());
	}

	#[test]
	fn rich_search_response_deserializes_all_types() {
		let response: RichSearchResponse = serde_json::from_str(
			r#"{"tracks": [], "albums": [{"id": "a", "name": "Album", "artist": "Art", "uri": "https://open.spotify.com/album/a"}], "artists": [{"id": "r", "name": "Art", "uri": "https://open.spotify.com/artist/r"}], "playlists": [{"id": "p", "name": "List", "author": "Someone", "uri": "https://open.spotify.com/playlist/p", "trackCount": 10}]}"#,
		)
		.unwrap();

		assert!(response.tracks.is_empty());
		assert_eq!(response.albums[0].artist, "Art");
		assert_eq!(response.artists[0].id, "r");
		assert_eq!(response.playlists[0].track_count, 10);
	}

	#[test]
	fn error_load_result_carries_code_and_no_track() {
		let result: LoadResult = serde_json::from_str(
			r#"{"loadType": "error", "data": {"message": "No matches", "severity": "COMMON", "code": "NO_MATCHES"}}"#,
		)
		.unwrap();

		assert_eq!(result.load_type, LoadResultType::Error);
		assert!(result.first_track().is_none());

		match result.data {
			LoadData::Error(failure) => {
				assert_eq!(failure.code, "NO_MATCHES");
				assert_eq!(failure.severity, "COMMON");
			}
			other => panic!("expected error data, got {other:?}"),
		}
	}
}
