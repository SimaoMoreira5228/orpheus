use std::collections::VecDeque;
use std::iter::Iterator;

use rand::prelude::SliceRandom;
use serde::{Deserialize, Serialize};

use super::filters::Filters;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
pub struct TrackInfo {
	pub identifier: String,
	pub is_seekable: bool,
	pub author: String,
	pub length: u64,
	pub is_stream: bool,
	pub position: u64,
	pub title: String,
	#[serde(default)]
	pub uri: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub artwork_url: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub isrc: Option<String>,
	pub source_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
	pub name: Option<String>,
	pub selected_track: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoadResultType {
	Track,
	Playlist,
	Search,
	Empty,
	Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadResult {
	pub load_type: LoadResultType,
	pub tracks: Vec<Track>,
	pub playlist_info: Option<PlaylistInfo>,
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
pub struct RichResults {
	#[serde(default)]
	pub tracks: Vec<Track>,
	#[serde(default)]
	pub albums: Vec<AlbumRef>,
	#[serde(default)]
	pub artists: Vec<ArtistRef>,
	#[serde(default)]
	pub playlists: Vec<PlaylistRef>,
}

impl RichResults {
	pub fn empty() -> Self {
		Self::default()
	}

	pub fn merge(parts: Vec<RichResults>) -> Self {
		let mut merged = RichResults::empty();

		for part in parts {
			merged.tracks.extend(part.tracks);
			merged.albums.extend(part.albums);
			merged.artists.extend(part.artists);
			merged.playlists.extend(part.playlists);
		}

		merged
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadFailure {
	pub message: String,
	pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadTracksResponse {
	pub load_type: LoadResultType,
	pub tracks: Vec<Track>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub playlist_info: Option<PlaylistInfo>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub failure: Option<LoadFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PlayerState {
	Stopped,
	Playing,
	Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RepeatMode {
	Off,
	Track,
	Queue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceState {
	pub token: String,
	pub endpoint: String,
	pub session_id: String,
	#[serde(default, deserialize_with = "string_or_number_opt")]
	pub channel_id: Option<String>,
}

pub fn string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
	D: serde::Deserializer<'de>,
{
	use serde::de::{self, Visitor};

	struct StringOrNumber;

	impl Visitor<'_> for StringOrNumber {
		type Value = String;

		fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
			formatter.write_str("a string or integer snowflake")
		}

		fn visit_str<E>(self, value: &str) -> Result<String, E>
		where
			E: de::Error,
		{
			Ok(value.to_string())
		}

		fn visit_string<E>(self, value: String) -> Result<String, E>
		where
			E: de::Error,
		{
			Ok(value)
		}

		fn visit_u64<E>(self, value: u64) -> Result<String, E>
		where
			E: de::Error,
		{
			Ok(value.to_string())
		}

		fn visit_i64<E>(self, value: i64) -> Result<String, E>
		where
			E: de::Error,
		{
			Ok(value.to_string())
		}
	}

	deserializer.deserialize_any(StringOrNumber)
}

pub fn string_or_number_opt<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	use serde::de::{self, Deserialize};

	match Option::<serde_json::Value>::deserialize(deserializer)? {
		None | Some(serde_json::Value::Null) => Ok(None),
		Some(serde_json::Value::String(value)) => Ok(Some(value)),
		Some(serde_json::Value::Number(value)) => Ok(Some(value.to_string())),
		Some(other) => Err(de::Error::custom(format!("expected string or number, found {other}"))),
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueInfo {
	pub current: Option<Track>,
	pub queue: Vec<Track>,
	pub history: Vec<Track>,
	pub position: usize,
	pub repeat: RepeatMode,
	pub autoplay: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Player {
	pub guild_id: u64,
	pub queue: VecDeque<Track>,
	pub current: Option<Track>,
	pub position: u64,
	pub volume: u32,
	pub filters: Filters,
	pub state: PlayerState,
	pub repeat: RepeatMode,
	pub voice_state: Option<VoiceState>,
	pub shuffle: bool,
	pub shuffle_positions: Vec<usize>,
	pub history: VecDeque<Track>,
	pub autoplay: bool,
}

impl Track {
	pub fn playable_uri(&self) -> Option<&str> {
		self.info.uri.as_deref().filter(|uri| !uri.is_empty())
	}
}

impl Player {
	pub fn new(guild_id: u64) -> Self {
		Self {
			guild_id,
			queue: VecDeque::new(),
			current: None,
			position: 0,
			volume: 100,
			filters: Filters::default(),
			state: PlayerState::Stopped,
			repeat: RepeatMode::Off,
			voice_state: None,
			shuffle: false,
			shuffle_positions: Vec::new(),
			history: VecDeque::new(),
			autoplay: false,
		}
	}

	pub fn play(&mut self) {
		if self.current.is_some() {
			self.state = PlayerState::Playing;
		}
	}

	pub fn pause(&mut self) {
		if self.state == PlayerState::Playing {
			self.state = PlayerState::Paused;
		}
	}

	pub fn stop(&mut self) {
		self.state = PlayerState::Stopped;
		self.position = 0;
		if let Some(track) = self.current.clone() {
			self.history.push_back(track);
		}
		self.current = None;
	}

	pub fn add_track(&mut self, track: Track) {
		self.queue.push_back(track);
	}

	pub fn add_tracks(&mut self, tracks: Vec<Track>) {
		for track in tracks {
			self.queue.push_back(track);
		}
	}

	pub fn insert_tracks(&mut self, position: usize, tracks: Vec<Track>) {
		for (offset, track) in tracks.into_iter().enumerate() {
			let insert_position = position.saturating_add(offset).min(self.queue.len());
			self.queue.insert(insert_position, track);
		}
	}

	#[allow(clippy::should_implement_trait)]
	pub fn next(&mut self) -> Option<Track> {
		self.position = 0;

		if let Some(track) = self.current.clone() {
			self.history.push_back(track);
		}

		match self.repeat {
			RepeatMode::Track => self.current.clone(),
			RepeatMode::Queue => {
				if let Some(track) = self.queue.pop_front() {
					self.queue.push_back(track.clone());
					self.current = Some(track.clone());
					self.state = PlayerState::Playing;
					return Some(track);
				}
				None
			}
			RepeatMode::Off => {
				if let Some(track) = self.queue.pop_front() {
					self.current = Some(track.clone());
					self.state = PlayerState::Playing;
					return Some(track);
				}
				self.current = None;
				self.state = PlayerState::Stopped;
				None
			}
		}
	}

	pub fn previous(&mut self) -> Option<Track> {
		self.position = 0;

		if let Some(track) = self.history.pop_back() {
			if let Some(current) = self.current.clone() {
				self.queue.push_front(current);
			}
			self.current = Some(track.clone());
			self.state = PlayerState::Playing;
			return Some(track);
		}
		None
	}

	pub fn seek(&mut self, position: u64) {
		self.position = position;
	}

	pub fn set_volume(&mut self, volume: u32) {
		self.volume = volume.min(1000);
	}

	pub fn set_filters(&mut self, filters: Filters) {
		self.filters = filters;
	}

	pub fn set_repeat(&mut self, mode: RepeatMode) {
		self.repeat = mode;
	}

	pub fn set_autoplay(&mut self, enabled: bool) {
		self.autoplay = enabled;
	}

	pub fn autoplay_next(&mut self, track: Track) {
		self.position = 0;

		if let Some(track) = self.current.clone() {
			self.history.push_back(track);
		}

		self.current = Some(track);
		self.state = PlayerState::Playing;
	}

	pub fn shuffle_queue(&mut self) {
		if self.queue.is_empty() {
			return;
		}

		let queue_len = self.queue.len();
		let mut indices: Vec<usize> = (0..queue_len).collect();
		let mut rng = rand::rng();
		indices.shuffle(&mut rng);
		self.shuffle_positions = indices;
		self.shuffle = true;

		let mut shuffled_queue = VecDeque::new();
		for idx in &self.shuffle_positions {
			if let Some(track) = self.queue.get(*idx).cloned() {
				shuffled_queue.push_back(track);
			}
		}
		self.queue = shuffled_queue;
	}

	pub fn unshuffle_queue(&mut self) {
		if !self.shuffle || self.shuffle_positions.is_empty() {
			return;
		}

		let mut unshuffled_queue = VecDeque::new();
		let mut tracks: Vec<Track> = self.queue.drain(..).collect();

		for idx in 0..self.shuffle_positions.len() {
			let orig_idx = self.shuffle_positions.iter().position(|&x| x == idx);
			if let Some(pos) = orig_idx
				&& pos < tracks.len()
			{
				unshuffled_queue.push_back(tracks.remove(0));
			}
		}

		for remaining in tracks {
			unshuffled_queue.push_back(remaining);
		}

		self.queue = unshuffled_queue;
		self.shuffle_positions.clear();
		self.shuffle = false;
	}

	pub fn get_queue_info(&self) -> QueueInfo {
		QueueInfo {
			current: self.current.clone(),
			queue: self.queue.iter().cloned().collect(),
			history: self.history.iter().cloned().collect(),
			position: 0,
			repeat: self.repeat,
			autoplay: self.autoplay,
		}
	}

	pub fn get_queue(&self) -> Vec<Track> {
		self.queue.iter().cloned().collect()
	}

	pub fn remove_from_queue(&mut self, position: usize) -> Option<Track> {
		if position >= self.queue.len() {
			return None;
		}
		self.queue.remove(position)
	}

	pub fn clear_queue(&mut self) {
		self.queue.clear();
	}

	pub fn skip_to(&mut self, position: usize) -> Option<Track> {
		if position >= self.queue.len() {
			self.position = 0;

			if let Some(track) = self.current.clone() {
				self.history.push_back(track);
			}

			self.current = None;
			self.state = PlayerState::Stopped;
			return None;
		}

		self.position = 0;

		if let Some(track) = self.current.clone() {
			self.history.push_back(track);
		}

		for _ in 0..position {
			self.queue.pop_front();
		}

		if let Some(track) = self.queue.pop_front() {
			self.current = Some(track.clone());
			self.state = PlayerState::Playing;
			return Some(track);
		}

		None
	}

	pub fn get_history(&self) -> Vec<Track> {
		self.history.iter().cloned().collect()
	}

	pub fn clear_history(&mut self) {
		self.history.clear();
	}

	pub fn save_state(&self) -> PlayerStateSnapshot {
		PlayerStateSnapshot {
			guild_id: self.guild_id,
			queue: self.queue.iter().cloned().collect(),
			current: self.current.clone(),
			position: self.position,
			volume: self.volume,
			filters: self.filters.clone(),
			state: self.state,
			repeat: self.repeat,
			shuffle: self.shuffle,
			shuffle_positions: self.shuffle_positions.clone(),
		}
	}

	pub fn restore_state(&mut self, snapshot: PlayerStateSnapshot) {
		self.queue = VecDeque::from(snapshot.queue);
		self.current = snapshot.current;
		self.position = snapshot.position;
		self.volume = snapshot.volume;
		self.filters = snapshot.filters;
		self.state = snapshot.state;
		self.repeat = snapshot.repeat;
		self.shuffle = snapshot.shuffle;
		self.shuffle_positions = snapshot.shuffle_positions;
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStateSnapshot {
	pub guild_id: u64,
	pub queue: Vec<Track>,
	pub current: Option<Track>,
	pub position: u64,
	pub volume: u32,
	pub filters: Filters,
	pub state: PlayerState,
	pub repeat: RepeatMode,
	pub shuffle: bool,
	pub shuffle_positions: Vec<usize>,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn track_with_uri(uri: Option<&str>) -> Track {
		Track {
			encoded: None,
			info: TrackInfo {
				identifier: "id".to_string(),
				is_seekable: true,
				author: "author".to_string(),
				length: 200_000,
				is_stream: false,
				position: 0,
				title: "title".to_string(),
				uri: uri.map(|uri| uri.to_string()),
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

	fn queued_player() -> Player {
		let mut player = Player::new(7);
		player.current = Some(track_with_uri(Some("https://example.invalid/current.opus")));
		player
			.queue
			.push_back(track_with_uri(Some("https://example.invalid/next.opus")));
		player
			.queue
			.push_back(track_with_uri(Some("https://example.invalid/later.opus")));
		player.state = PlayerState::Playing;
		player
	}

	#[test]
	fn playable_uri_rejects_missing_and_blank() {
		assert!(
			track_with_uri(Some("https://example.invalid/audio.opus"))
				.playable_uri()
				.is_some()
		);
		assert!(track_with_uri(Some("")).playable_uri().is_none());
		assert!(track_with_uri(None).playable_uri().is_none());
	}

	#[test]
	fn null_uri_survives_json_wire_as_null() {
		let track = track_with_uri(None);
		let json = serde_json::to_value(&track).unwrap();

		assert!(json["info"]["uri"].is_null());

		let missing: Track = serde_json::from_str(r#"{"info":{"identifier":"id","isSeekable":true,"author":"a","length":0,"isStream":false,"position":0,"title":"t","sourceName":"http"}}"#).unwrap();

		assert!(missing.playable_uri().is_none());

		let explicit_null: Track =
			serde_json::from_str(r#"{"info":{"identifier":"id","isSeekable":true,"author":"a","length":0,"isStream":false,"position":0,"title":"t","uri":null,"sourceName":"http"}}"#).unwrap();

		assert!(explicit_null.playable_uri().is_none());
	}

	#[test]
	fn skip_to_switches_the_played_track() {
		let mut player = queued_player();

		let track = player.skip_to(1).expect("skip target must exist");

		assert_eq!(track.playable_uri(), Some("https://example.invalid/later.opus"));
		assert_eq!(
			player.current.as_ref().and_then(|current| current.playable_uri()),
			Some("https://example.invalid/later.opus")
		);
		assert_eq!(player.state, PlayerState::Playing);
		assert_eq!(player.position, 0);
	}

	#[test]
	fn skip_past_the_end_removes_the_current_track() {
		let mut player = queued_player();

		assert!(player.skip_to(9).is_none());
		assert!(player.current.is_none());
		assert_eq!(player.state, PlayerState::Stopped);
		assert_eq!(player.history.len(), 1);
	}

	#[test]
	fn pending_edits_leave_the_current_track_untouched() {
		let mut player = queued_player();
		let current_uri = player
			.current
			.clone()
			.and_then(|current| current.playable_uri().map(|uri| uri.to_string()));

		player.shuffle_queue();
		player.set_repeat(RepeatMode::Queue);
		player.remove_from_queue(0);

		assert_eq!(
			player
				.current
				.clone()
				.and_then(|current| current.playable_uri().map(|uri| uri.to_string())),
			current_uri
		);
		assert_eq!(player.state, PlayerState::Playing);
	}

	#[test]
	fn inserting_tracks_preserves_order_and_appends_out_of_range_positions() {
		let mut player = queued_player();

		player.insert_tracks(
			1,
			vec![
				track_with_uri(Some("https://example.invalid/inserted-a.opus")),
				track_with_uri(Some("https://example.invalid/inserted-b.opus")),
			],
		);
		player.insert_tracks(
			usize::MAX,
			vec![track_with_uri(Some("https://example.invalid/appended.opus"))],
		);

		let uris: Vec<_> = player.queue.iter().map(|track| track.playable_uri().unwrap()).collect();

		assert_eq!(
			uris,
			[
				"https://example.invalid/next.opus",
				"https://example.invalid/inserted-a.opus",
				"https://example.invalid/inserted-b.opus",
				"https://example.invalid/later.opus",
				"https://example.invalid/appended.opus",
			]
		);
	}

	#[test]
	fn autoplay_next_continues_from_an_empty_queue() {
		let mut player = queued_player();
		player.queue.clear();

		let continued = track_with_uri(Some("https://example.invalid/autoplay.opus"));
		player.autoplay_next(continued);

		assert_eq!(
			player.current.as_ref().and_then(|current| current.playable_uri()),
			Some("https://example.invalid/autoplay.opus")
		);
		assert_eq!(player.state, PlayerState::Playing);
		assert_eq!(player.position, 0);
		assert_eq!(player.history.len(), 1);
		assert!(player.queue.is_empty());
	}

	#[test]
	fn volume_clamps_to_lavalink_range() {
		let mut player = Player::new(7);
		assert_eq!(player.volume, 100);

		player.set_volume(500);
		assert_eq!(player.volume, 500);

		player.set_volume(1500);
		assert_eq!(player.volume, 1000);

		player.set_volume(0);
		assert_eq!(player.volume, 0);
	}
}
