use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use dashmap::{DashMap, DashSet};
use orpheus_core::{PlayerManager, PlayerState, RepeatMode, Track};
use orpheus_track_resolver::TrackResolver;
use parking_lot::Mutex;

use crate::runtime_config::RuntimeConfigStore;
use crate::voice::{VoiceConnectionManager, VoiceProgress};

const MAX_CACHED_GUILDS: usize = 512;

struct CachedPlayback {
	identifier: String,
	uri: String,
	created_at: Instant,
}

pub struct PrefetchCoordinator {
	players: Arc<PlayerManager>,
	resolver: Arc<TrackResolver>,
	config: Arc<RuntimeConfigStore>,
	cached: DashMap<u64, CachedPlayback>,
	in_flight: DashSet<u64>,
	active: AtomicUsize,
	cache_lock: Mutex<()>,
}

impl PrefetchCoordinator {
	pub fn new(players: Arc<PlayerManager>, resolver: Arc<TrackResolver>, config: Arc<RuntimeConfigStore>) -> Self {
		Self {
			players,
			resolver,
			config,
			cached: DashMap::new(),
			in_flight: DashSet::new(),
			active: AtomicUsize::new(0),
			cache_lock: Mutex::new(()),
		}
	}

	pub fn spawn(self: Arc<Self>, voice: Arc<VoiceConnectionManager>) {
		let mut progress = voice.subscribe_progress();

		tokio::spawn(async move {
			loop {
				match progress.recv().await {
					Ok(update) => self.consider(update).await,
					Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
					Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
				}
			}
		});
	}

	pub fn take(&self, guild_id: u64, track: &Track) -> Option<String> {
		let config = self.config.current();
		if !config.prefetch.enabled {
			self.cached.remove(&guild_id);
			return None;
		}

		let cached = self.cached.remove(&guild_id)?.1;
		if cached.identifier != track.info.identifier
			|| cached.created_at.elapsed().as_millis() > u128::from(config.prefetch.max_url_age_ms)
		{
			return None;
		}

		Some(cached.uri)
	}

	pub fn invalidate(&self, guild_id: u64) {
		self.cached.remove(&guild_id);
	}

	async fn consider(self: &Arc<Self>, progress: VoiceProgress) {
		let config = self.config.current();
		if !config.prefetch.enabled || self.in_flight.contains(&progress.guild_id) {
			return;
		}

		let Some(lock) = self.players.get(progress.guild_id).await else {
			return;
		};
		let (current, next) = {
			let player = lock.read().await;
			if player.state != PlayerState::Playing || player.repeat == RepeatMode::Track {
				return;
			}

			let Some(current) = player.current.as_ref() else {
				return;
			};
			let Some(next) = player.queue.front() else {
				return;
			};
			if !should_prefetch(current, next, progress.position_ms, config.prefetch.lookahead_ms) {
				return;
			}

			(current.clone(), next.clone())
		};

		let cached_is_fresh = self.cached.get(&progress.guild_id).is_some_and(|cached| {
			cached.identifier == next.info.identifier
				&& cached.created_at.elapsed().as_millis() <= u128::from(config.prefetch.max_url_age_ms)
		});
		if cached_is_fresh {
			return;
		}
		if self.cached.contains_key(&progress.guild_id) {
			self.cached.remove(&progress.guild_id);
		}

		if !self.in_flight.insert(progress.guild_id) {
			return;
		}

		if !self.reserve(config.prefetch.max_concurrent) {
			self.in_flight.remove(&progress.guild_id);
			return;
		}

		let coordinator = self.clone();
		tokio::spawn(async move {
			let uri = coordinator.resolver.playback_url(&next).await;
			coordinator.cache_if_still_next(progress.guild_id, &current, &next, uri).await;
			coordinator.in_flight.remove(&progress.guild_id);
			coordinator.active.fetch_sub(1, Ordering::AcqRel);
		});
	}

	async fn cache_if_still_next(&self, guild_id: u64, current: &Track, next: &Track, uri: String) {
		let config = self.config.current();
		if !config.prefetch.enabled
			|| uri.is_empty()
			|| next.info.uri.as_deref() == Some(uri.as_str())
			|| orpheus_track_resolver::baked_stream_is_expired(&uri, now_unix())
		{
			return;
		}

		let Some(lock) = self.players.get(guild_id).await else {
			return;
		};
		let player = lock.read().await;
		let still_next = player.state == PlayerState::Playing
			&& player
				.current
				.as_ref()
				.is_some_and(|playing| playing.info.identifier == current.info.identifier)
			&& player
				.queue
				.front()
				.is_some_and(|queued| queued.info.identifier == next.info.identifier);
		drop(player);
		if !still_next {
			return;
		}

		let _cache_lock = self.cache_lock.lock();
		if self.cached.len() >= MAX_CACHED_GUILDS
			&& !self.cached.contains_key(&guild_id)
			&& let Some(oldest) = self
				.cached
				.iter()
				.min_by_key(|entry| entry.created_at)
				.map(|entry| *entry.key())
		{
			self.cached.remove(&oldest);
		}

		self.cached.insert(
			guild_id,
			CachedPlayback {
				identifier: next.info.identifier.clone(),
				uri,
				created_at: Instant::now(),
			},
		);
	}

	fn reserve(&self, limit: usize) -> bool {
		Self::reserve_active(&self.active, limit)
	}

	fn reserve_active(active: &AtomicUsize, limit: usize) -> bool {
		let mut count = active.load(Ordering::Acquire);
		loop {
			if count >= limit {
				return false;
			}

			match active.compare_exchange(count, count + 1, Ordering::AcqRel, Ordering::Acquire) {
				Ok(_) => return true,
				Err(actual) => count = actual,
			}
		}
	}
}

fn now_unix() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

fn is_resolvable(track: &Track) -> bool {
	!track.info.is_stream
		&& track.info.length > 0
		&& track.info.uri.as_deref().is_some_and(|uri| !uri.is_empty())
		&& matches!(track.info.source_name.as_str(), "youtube" | "spotify")
}

fn should_prefetch(current: &Track, next: &Track, position_ms: u64, lookahead_ms: u64) -> bool {
	if current.info.is_stream || current.info.length == 0 || !is_resolvable(next) {
		return false;
	}

	let end = current.end_time.unwrap_or(current.info.length).min(current.info.length);
	end.saturating_sub(position_ms) <= lookahead_ms
}

#[cfg(test)]
mod tests {
	use orpheus_core::TrackInfo;

	use super::*;

	fn track(source: &str, is_stream: bool) -> Track {
		Track {
			encoded: None,
			info: TrackInfo {
				identifier: format!("{source}-track"),
				is_seekable: true,
				author: "artist".to_string(),
				length: 200_000,
				is_stream,
				position: 0,
				title: "track".to_string(),
				uri: Some("https://example.invalid/audio".to_string()),
				artwork_url: None,
				isrc: None,
				source_name: source.to_string(),
			},
			start_time: None,
			end_time: None,
			plugin_info: serde_json::Value::Null,
			user_data: serde_json::Value::Null,
		}
	}

	#[test]
	fn prefetch_is_limited_to_dynamic_tracks_near_a_finite_track_end() {
		let current = track("youtube", false);
		let next = track("youtube", false);
		let live_current = track("twitch", true);
		let live_next = track("twitch", true);
		let static_next = track("http", false);

		assert!(!should_prefetch(&current, &next, 184_999, 15_000));
		assert!(should_prefetch(&current, &next, 185_000, 15_000));
		assert!(!should_prefetch(&live_current, &next, 195_000, 15_000));
		assert!(!should_prefetch(&current, &live_next, 195_000, 15_000));
		assert!(!should_prefetch(&current, &static_next, 195_000, 15_000));
	}

	#[test]
	fn prefetch_reservations_never_exceed_the_concurrency_limit() {
		let active = AtomicUsize::new(0);

		assert!(!PrefetchCoordinator::reserve_active(&active, 0));
		assert!(PrefetchCoordinator::reserve_active(&active, 2));
		assert!(PrefetchCoordinator::reserve_active(&active, 2));
		assert!(!PrefetchCoordinator::reserve_active(&active, 2));
		assert_eq!(active.load(Ordering::Acquire), 2);
	}
}
