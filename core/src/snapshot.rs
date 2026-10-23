use serde::{Deserialize, Serialize};

use super::filters::Filters;
use super::player::{Player, PlayerState, RepeatMode, Track};

pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
	pub version: u32,
	pub current: Option<Track>,
	pub queue: Vec<Track>,
	pub history: Vec<Track>,
	pub position: u64,
	pub volume: u32,
	pub filters: Filters,
	pub repeat: RepeatMode,
	pub shuffle: bool,
	pub shuffle_positions: Vec<usize>,
	pub autoplay: bool,
}

impl Player {
	pub fn snapshot(&self) -> PlayerSnapshot {
		PlayerSnapshot {
			version: SNAPSHOT_VERSION,
			current: self.current.clone(),
			queue: self.queue.iter().cloned().collect(),
			history: self.history.iter().cloned().collect(),
			position: self.position,
			volume: self.volume,
			filters: self.filters.clone(),
			repeat: self.repeat,
			shuffle: self.shuffle,
			shuffle_positions: self.shuffle_positions.clone(),
			autoplay: self.autoplay,
		}
	}

	pub fn restore(&mut self, snapshot: PlayerSnapshot) {
		self.current = snapshot.current;
		self.queue = snapshot.queue.into();
		self.history = snapshot.history.into();
		self.position = snapshot.position;
		self.volume = snapshot.volume;
		self.filters = snapshot.filters;
		self.repeat = snapshot.repeat;
		self.shuffle = snapshot.shuffle;
		self.shuffle_positions = snapshot.shuffle_positions;
		self.autoplay = snapshot.autoplay;
		self.state = if self.current.is_some() {
			PlayerState::Paused
		} else {
			PlayerState::Stopped
		};
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::TrackInfo;

	fn queued_track(name: &str) -> Track {
		Track {
			encoded: Some(format!("encoded-{name}")),
			info: TrackInfo {
				identifier: name.to_string(),
				is_seekable: true,
				author: "author".to_string(),
				length: 1000,
				is_stream: false,
				position: 0,
				title: name.to_string(),
				uri: Some(name.to_string()),
				artwork_url: None,
				isrc: None,
				source_name: "test".to_string(),
			},
			start_time: None,
			end_time: None,
			plugin_info: serde_json::json!({}),
			user_data: serde_json::json!({}),
		}
	}

	#[test]
	fn snapshot_round_trip_preserves_playback_state() {
		let mut player = Player::new(7);

		player.add_tracks(vec![queued_track("a"), queued_track("b")]);
		player.add_tracks(vec![queued_track("c")]);
		player.next();
		player.next();
		player.set_volume(250);

		let snapshot = player.snapshot();

		assert_eq!(snapshot.version, SNAPSHOT_VERSION);
		assert_eq!(snapshot.current.as_ref().unwrap().info.identifier, "b");
		assert_eq!(snapshot.queue.len(), 1);
		assert_eq!(snapshot.history.len(), 1);
		assert_eq!(snapshot.volume, 250);

		let mut restored = Player::new(9);

		restored.restore(snapshot);

		assert_eq!(restored.current.as_ref().unwrap().info.identifier, "b");
		assert_eq!(restored.queue.len(), 1);
		assert_eq!(restored.queue[0].info.identifier, "c");
		assert_eq!(restored.history.len(), 1);
		assert_eq!(restored.history[0].info.identifier, "a");
		assert_eq!(restored.volume, 250);
		assert_eq!(restored.state, PlayerState::Paused);
		assert_eq!(restored.guild_id, 9);
	}

	#[test]
	fn restore_without_current_track_stops() {
		let mut restored = Player::new(7);

		restored.restore(PlayerSnapshot {
			version: SNAPSHOT_VERSION,
			current: None,
			queue: vec![],
			history: vec![],
			position: 0,
			volume: 100,
			filters: Filters::default(),
			repeat: RepeatMode::Off,
			shuffle: false,
			shuffle_positions: vec![],
			autoplay: false,
		});

		assert!(restored.current.is_none());
		assert_eq!(restored.state, PlayerState::Stopped);
	}
}
