use std::sync::Arc;

use orpheus_core::{PlayerManager, PlayerState, Track};
use orpheus_track_resolver::TrackResolver;

use crate::prefetch::PrefetchCoordinator;
use crate::voice::VoiceConnectionManager;
use crate::voice_events::VoiceEvent;
use crate::websocket_messages::TrackEndReason;

fn is_video_id_char(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

fn valid_video_id(candidate: &str) -> Option<String> {
	if candidate.len() == 11 && candidate.bytes().all(is_video_id_char) {
		return Some(candidate.to_string());
	}

	None
}

fn query_video_id(query: &str) -> Option<String> {
	for pair in query.split('&') {
		let Some((key, value)) = pair.split_once('=') else {
			continue;
		};

		if key == "v" {
			return valid_video_id(value);
		}
	}

	None
}

fn path_video_id(path: &str) -> Option<String> {
	for prefix in ["/embed/", "/shorts/", "/v/", "/live/"] {
		if let Some(rest) = path.strip_prefix(prefix) {
			let id = rest.split(['/', '?', '#']).next().unwrap_or_default();

			if let Some(valid) = valid_video_id(id) {
				return Some(valid);
			}
		}
	}

	None
}

pub fn youtube_video_id(identifier: &str) -> Option<String> {
	let identifier = identifier.trim();

	if identifier.is_empty() {
		return None;
	}

	if !identifier.contains("://") {
		return valid_video_id(identifier);
	}

	let url = reqwest::Url::parse(identifier).ok()?;
	if !matches!(url.scheme(), "http" | "https") {
		return None;
	}

	let host = url.host_str()?;

	if !is_youtube_host(host) {
		return None;
	}

	if host == "youtu.be" || host.ends_with(".youtu.be") {
		let id = url
			.path()
			.strip_prefix('/')
			.unwrap_or_default()
			.split('/')
			.next()
			.unwrap_or_default();

		return valid_video_id(id);
	}

	if let Some(query) = url.query()
		&& let Some(id) = query_video_id(query)
	{
		return Some(id);
	}

	path_video_id(url.path())
}

fn is_youtube_host(host: &str) -> bool {
	["youtube.com", "youtu.be"]
		.iter()
		.any(|domain| host == *domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.')))
}

pub fn mix_url_for(video_id: &str) -> String {
	format!("https://www.youtube.com/watch?v={video_id}&list=RD{video_id}")
}

pub fn select_autoplay_track(current_video_id: &str, tracks: &[Track]) -> Option<Track> {
	tracks
		.iter()
		.find(|track| {
			youtube_video_id(&track.info.identifier)
				.or_else(|| track.info.uri.as_deref().and_then(youtube_video_id))
				.is_some_and(|id| id != current_video_id)
		})
		.cloned()
}

async fn queue_drained(players: &Arc<PlayerManager>, guild_id: u64, finished_identifier: &str) -> bool {
	let Some(lock) = players.get(guild_id).await else {
		return false;
	};

	let player = lock.read().await;

	if !player.autoplay || !player.queue.is_empty() {
		return false;
	}

	player
		.current
		.as_ref()
		.is_some_and(|current| current.info.identifier == finished_identifier)
}

async fn continue_autoplay(
	players: &Arc<PlayerManager>,
	resolver: &Arc<TrackResolver>,
	voice: &Arc<VoiceConnectionManager>,
	guild_id: u64,
	finished: &Track,
) -> bool {
	if !queue_drained(players, guild_id, &finished.info.identifier).await {
		return false;
	}

	let video_id =
		youtube_video_id(&finished.info.identifier).or_else(|| finished.info.uri.as_deref().and_then(youtube_video_id));

	let Some(video_id) = video_id else {
		return false;
	};

	let loaded = resolver.load(&mix_url_for(&video_id)).await;

	let tracks = match loaded {
		Ok((result, _)) => result.tracks,
		Err(e) => {
			tracing::warn!("Autoplay for guild {} abandoned mix load: {}", guild_id, e);

			return false;
		}
	};

	let Some(next) = select_autoplay_track(&video_id, &tracks) else {
		return false;
	};

	let play = {
		let Some(lock) = players.get(guild_id).await else {
			return false;
		};

		let mut player = lock.write().await;

		if !player.autoplay || !player.queue.is_empty() {
			return false;
		}

		let current_matches = player
			.current
			.as_ref()
			.is_some_and(|current| current.info.identifier == finished.info.identifier);

		if !current_matches {
			return false;
		}

		player.autoplay_next(next.clone());

		(player.volume, player.filters.clone(), next.end_time)
	};

	let playback = resolver.playback_url(&next).await;

	let mut voice_track = next;
	voice_track.info.uri = Some(playback);

	tracing::info!("Autoplay for guild {} continues with {:?}", guild_id, voice_track.info.title);

	voice.play(guild_id, voice_track, 0, play.0, play.1, play.2).await;
	voice.request_player_update(guild_id);
	true
}

async fn advance_queue(
	players: &Arc<PlayerManager>,
	resolver: &Arc<TrackResolver>,
	voice: &Arc<VoiceConnectionManager>,
	prefetch: &Arc<PrefetchCoordinator>,
	guild_id: u64,
	finished: &Track,
) {
	let next = {
		let Some(lock) = players.get(guild_id).await else {
			return;
		};
		let mut player = lock.write().await;
		let current_matches = player
			.current
			.as_ref()
			.is_some_and(|current| current.info.identifier == finished.info.identifier);
		if !current_matches || player.state != PlayerState::Playing {
			return;
		}

		let Some(track) = player.next() else {
			return;
		};

		(track, player.volume, player.filters.clone())
	};

	let prepared_uri = prefetch.take(guild_id, &next.0);
	let playback_uri = match prepared_uri {
		Some(uri) => uri,
		None => resolver.playback_url(&next.0).await,
	};

	let still_current = if let Some(lock) = players.get(guild_id).await {
		let player = lock.read().await;
		player.state == PlayerState::Playing
			&& player
				.current
				.as_ref()
				.is_some_and(|current| current.info.identifier == next.0.info.identifier)
	} else {
		false
	};

	if !still_current {
		return;
	}

	let mut voice_track = next.0;
	voice_track.info.uri = Some(playback_uri);
	voice
		.play(guild_id, voice_track.clone(), 0, next.1, next.2, voice_track.end_time)
		.await;
	voice.request_player_update(guild_id);
}

pub fn spawn_autoplay(
	players: Arc<PlayerManager>,
	resolver: Arc<TrackResolver>,
	voice: Arc<VoiceConnectionManager>,
	prefetch: Arc<PrefetchCoordinator>,
) {
	let mut events = voice.subscribe();

	tokio::spawn(async move {
		loop {
			match events.recv().await {
				Ok(VoiceEvent::Ended { guild_id, track, reason }) => {
					if matches!(reason, TrackEndReason::Finished) {
						let continued = continue_autoplay(&players, &resolver, &voice, guild_id, &track).await;
						if !continued {
							advance_queue(&players, &resolver, &voice, &prefetch, guild_id, &track).await;
						}
					} else {
						prefetch.invalidate(guild_id);
					}
				}
				Ok(_) => {}
				Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
				Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
			}
		}
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	fn mix_track(identifier: &str, uri: Option<&str>) -> Track {
		Track {
			encoded: None,
			info: orpheus_core::TrackInfo {
				identifier: identifier.to_string(),
				is_seekable: true,
				author: "author".to_string(),
				length: 200_000,
				is_stream: false,
				position: 0,
				title: "title".to_string(),
				uri: uri.map(|uri| uri.to_string()),
				artwork_url: None,
				isrc: None,
				source_name: "youtube".to_string(),
			},
			start_time: None,
			end_time: None,
			plugin_info: serde_json::json!({}),
			user_data: serde_json::json!({}),
		}
	}

	#[test]
	fn watch_url_yields_its_video_id() {
		assert_eq!(
			youtube_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
			Some("dQw4w9WgXcQ".to_string())
		);
	}

	#[test]
	fn watch_url_with_mix_list_still_yields_the_video_id() {
		assert_eq!(
			youtube_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ"),
			Some("dQw4w9WgXcQ".to_string())
		);
	}

	#[test]
	fn short_and_bare_forms_yield_the_video_id() {
		assert_eq!(
			youtube_video_id("https://youtu.be/dQw4w9WgXcQ"),
			Some("dQw4w9WgXcQ".to_string())
		);
		assert_eq!(youtube_video_id("dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".to_string()));
		assert_eq!(
			youtube_video_id("https://music.youtube.com/watch?v=dQw4w9WgXcQ"),
			Some("dQw4w9WgXcQ".to_string())
		);
	}

	#[test]
	fn embed_and_shorts_paths_yield_the_video_id() {
		assert_eq!(
			youtube_video_id("https://www.youtube.com/embed/dQw4w9WgXcQ"),
			Some("dQw4w9WgXcQ".to_string())
		);
		assert_eq!(
			youtube_video_id("https://www.youtube.com/shorts/dQw4w9WgXcQ"),
			Some("dQw4w9WgXcQ".to_string())
		);
	}

	#[test]
	fn non_youtube_and_malformed_identifiers_yield_nothing() {
		assert!(youtube_video_id("").is_none());
		assert!(youtube_video_id("https://example.com/watch?v=dQw4w9WgXcQ").is_none());
		assert!(youtube_video_id("https://notyoutube.com/watch?v=dQw4w9WgXcQ").is_none());
		assert!(youtube_video_id("https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ").is_none());
		assert!(youtube_video_id("https://notyoutu.be/dQw4w9WgXcQ").is_none());
		assert!(youtube_video_id("ftp://youtube.com/watch?v=dQw4w9WgXcQ").is_none());
		assert!(youtube_video_id("https://www.youtube.com/watch?v=too-short").is_none());
		assert!(youtube_video_id("not an id!!").is_none());
		assert!(youtube_video_id("ytsearch:lofi hip hop").is_none());
	}

	#[test]
	fn youtube_url_ports_and_mixed_case_schemes_are_parsed() {
		assert_eq!(
			youtube_video_id("HTTPS://www.youtube.com:443/watch?v=dQw4w9WgXcQ"),
			Some("dQw4w9WgXcQ".to_string())
		);
	}

	#[test]
	fn mix_url_carries_the_video_id_and_mix_list() {
		let url = mix_url_for("dQw4w9WgXcQ");

		assert_eq!(url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ");
	}

	#[test]
	fn selection_takes_the_first_entry_past_the_current_track() {
		let tracks = vec![
			mix_track("https://www.youtube.com/watch?v=dQw4w9WgXcQ", None),
			mix_track("https://www.youtube.com/watch?v=9bZkp7q19f0", None),
		];

		let next = select_autoplay_track("dQw4w9WgXcQ", &tracks).expect("mix follow-up");

		assert_eq!(next.info.identifier, "https://www.youtube.com/watch?v=9bZkp7q19f0");
	}

	#[test]
	fn selection_returns_nothing_for_an_empty_or_self_only_mix() {
		assert!(select_autoplay_track("dQw4w9WgXcQ", &[]).is_none());

		let tracks = vec![mix_track("https://www.youtube.com/watch?v=dQw4w9WgXcQ", None)];

		assert!(select_autoplay_track("dQw4w9WgXcQ", &tracks).is_none());
	}

	#[test]
	fn selection_skips_entries_without_a_usable_video_id() {
		let tracks = vec![
			mix_track("ytsearch:unresolved", None),
			mix_track("https://www.youtube.com/watch?v=9bZkp7q19f0", None),
		];

		let next = select_autoplay_track("dQw4w9WgXcQ", &tracks).expect("mix follow-up");

		assert_eq!(next.info.identifier, "https://www.youtube.com/watch?v=9bZkp7q19f0");
	}

	#[test]
	fn selection_falls_back_to_the_playback_uri_for_the_video_id() {
		let tracks = vec![mix_track("ytsearch:unresolved", Some("https://youtu.be/9bZkp7q19f0"))];

		let next = select_autoplay_track("dQw4w9WgXcQ", &tracks).expect("mix follow-up");

		assert_eq!(next.info.uri.as_deref(), Some("https://youtu.be/9bZkp7q19f0"));
	}
}
