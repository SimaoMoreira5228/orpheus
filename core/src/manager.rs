use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::RwLock;

use super::player::Player;

pub struct PlayerManager {
	players: DashMap<u64, Arc<RwLock<Player>>>,
}

impl PlayerManager {
	pub fn new() -> Self {
		Self { players: DashMap::new() }
	}

	pub async fn get(&self, guild_id: u64) -> Option<Arc<RwLock<Player>>> {
		self.players.get(&guild_id).map(|entry| Arc::clone(entry.value()))
	}

	pub async fn get_or_create(&self, guild_id: u64) -> Arc<RwLock<Player>> {
		Arc::clone(
			self.players
				.entry(guild_id)
				.or_insert_with(|| Arc::new(RwLock::new(Player::new(guild_id))))
				.value(),
		)
	}

	pub async fn remove(&self, guild_id: u64) {
		self.players.remove(&guild_id);
	}

	pub async fn list(&self) -> Vec<u64> {
		self.players.iter().map(|entry| *entry.key()).collect()
	}

	pub async fn get_player_count(&self) -> usize {
		self.players.len()
	}

	pub async fn queued_track_count(&self) -> usize {
		let players: Vec<_> = self.players.iter().map(|entry| Arc::clone(entry.value())).collect();
		let mut queued = 0;

		for player in players {
			queued += player.read().await.queue.len();
		}

		queued
	}

	pub async fn get_all_players(&self) -> Vec<(u64, Player)> {
		let players: Vec<_> = self
			.players
			.iter()
			.map(|entry| (*entry.key(), Arc::clone(entry.value())))
			.collect();
		let mut result = Vec::new();
		for (guild_id, player_arc) in players {
			let player = player_arc.read().await;
			result.push((guild_id, player.clone()));
		}
		result
	}
}

impl Default for PlayerManager {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Track, TrackInfo};

	fn queued_track(name: &str) -> Track {
		Track {
			encoded: None,
			info: TrackInfo {
				identifier: name.to_string(),
				is_seekable: true,
				author: "author".to_string(),
				length: 0,
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

	#[tokio::test]
	async fn queued_track_count_sums_pending_queues() {
		let manager = PlayerManager::new();

		assert_eq!(manager.queued_track_count().await, 0);

		manager
			.get_or_create(7)
			.await
			.write()
			.await
			.add_tracks(vec![queued_track("a"), queued_track("b")]);
		manager
			.get_or_create(8)
			.await
			.write()
			.await
			.add_tracks(vec![queued_track("c")]);

		assert_eq!(manager.queued_track_count().await, 3);
	}
}
