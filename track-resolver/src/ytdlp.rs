use std::sync::Arc;
use std::time::Duration;

use libytdlp::{Client, MediaInfo, PlayerClient, player_client_options};
use tracing::warn;

use crate::{
	LoadResult, LoadType, PlaylistInfo, ResolverError, Result, RichResults, SourceManager, Track, TrackInfo, new_track,
};

fn youtube_url(identifier: &str) -> Option<reqwest::Url> {
	let url = reqwest::Url::parse(identifier).ok()?;
	if !matches!(url.scheme(), "http" | "https") {
		return None;
	}

	let host = url.host_str()?;
	if host != "youtube.com" && !host.ends_with(".youtube.com") && host != "youtu.be" {
		return None;
	}

	Some(url)
}

fn is_youtube_url(identifier: &str) -> bool {
	youtube_url(identifier).is_some()
}

fn is_youtube_search(identifier: &str) -> bool {
	let lower = identifier.to_ascii_lowercase();

	lower.starts_with("ytsearch:") || lower.starts_with("ytmsearch:")
}

fn is_youtube_playlist(identifier: &str) -> bool {
	let Some(url) = youtube_url(identifier) else {
		return false;
	};

	let is_playlist_path = url
		.path_segments()
		.is_some_and(|segments| segments.into_iter().any(|segment| segment == "playlist"));
	let has_list_parameter = url.query_pairs().any(|(key, _)| key == "list");

	is_playlist_path || has_list_parameter
}

#[cfg(test)]
struct YoutubePlaylistPattern;

#[cfg(test)]
impl YoutubePlaylistPattern {
	fn is_match(&self, identifier: &str) -> bool {
		is_youtube_playlist(identifier)
	}
}

#[cfg(test)]
const YOUTUBE_PLAYLIST_REGEX: YoutubePlaylistPattern = YoutubePlaylistPattern;

// NOTE: probe must mirror ffmpeg's initial request (same UA, plain full GET,
// no Range): a ranged probe passes URLs that ffmpeg then fails to open.
const FFMPEG_USER_AGENT: &str =
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct YtDlpSourceManager {
	client: Arc<Client>,
}

impl YtDlpSourceManager {
	pub fn new(client: Arc<Client>) -> Self {
		Self { client }
	}

	async fn resolve_video(&self, identifier: &str) -> Result<LoadResult> {
		let url = if identifier.contains("://") {
			identifier.to_string()
		} else {
			format!("ytsearch1:{}", identifier)
		};

		for (client, options) in PlayerClient::RETRY_ORDER
			.into_iter()
			.zip(player_client_options(PlayerClient::RETRY_ORDER))
		{
			let info = match self.extract(&url, &options).await {
				Ok(info) => info,
				Err(e) => {
					warn!(
						"YouTube resolve with {} client failed: {}, trying next client",
						client.as_str(),
						e
					);
					continue;
				}
			};

			let info = match first_entry_watch_url(&info) {
				Some(watch_url) => match self.extract(&watch_url, &options).await {
					Ok(info) => info,
					Err(e) => {
						warn!(
							"YouTube resolve with {} client failed on search entry: {}, trying next client",
							client.as_str(),
							e
						);
						continue;
					}
				},
				None => info,
			};

			let Some(stream_url) = info.best_audio_url().map(|url| url.to_string()) else {
				warn!(
					"YouTube resolve with {} client found no audio stream, trying next client",
					client.as_str(),
				);
				continue;
			};

			if probe_stream_url(&stream_url).await {
				return Ok(track_from_info(&info, stream_url));
			}

			warn!(
				"YouTube stream URL from {} client not fetchable, trying next client",
				client.as_str(),
			);
		}

		Err(ResolverError::NoAudioStream)
	}

	async fn extract(&self, url: &str, options: &str) -> Result<MediaInfo> {
		let client = self.client.clone();
		let url = url.to_string();
		let options = options.to_string();

		tokio::task::spawn_blocking(move || client.extract_with_options(&url, &options))
			.await
			.map_err(|e| ResolverError::FetchError(format!("extract task failed: {}", e)))?
			.map_err(|e| ResolverError::FetchError(e.to_string()))
	}

	async fn search(&self, query: &str) -> Result<LoadResult> {
		let search_query = query.trim_start_matches("ytsearch:").trim_start_matches("ytmsearch:");

		let client = self.client.clone();
		let search_query = search_query.to_string();

		let info = tokio::task::spawn_blocking(move || client.search_flat(&search_query, 10))
			.await
			.map_err(|e| ResolverError::FetchError(format!("search task failed: {}", e)))?
			.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let tracks: Vec<Track> = info
			.entries()
			.into_iter()
			.enumerate()
			.filter_map(|(i, entry)| track_from_entry(&entry, i))
			.collect();

		Ok(LoadResult {
			load_type: LoadType::Search,
			tracks,
			playlist_info: None,
		})
	}

	pub(crate) async fn rich_search(&self, query: &str) -> Result<RichResults> {
		let result = self.search(query).await?;

		Ok(RichResults {
			tracks: result.tracks,
			..RichResults::empty()
		})
	}

	async fn load_playlist(&self, identifier: &str) -> Result<LoadResult> {
		let info = self.extract(identifier, "{}").await?;

		let name = info.title().map(|title| title.to_string());

		let tracks: Vec<Track> = info
			.entries()
			.into_iter()
			.enumerate()
			.filter_map(|(i, entry)| {
				let id = entry.id()?;
				let video_url = format!("https://www.youtube.com/watch?v={}", id);

				Some(new_track(TrackInfo {
					identifier: video_url.clone(),
					is_seekable: true,
					author: entry.uploader().unwrap_or_default().to_string(),
					length: entry.duration_secs().map(|d| d as u64 * 1000).unwrap_or(0),
					is_stream: false,
					position: i as u64,
					title: entry.title().unwrap_or_default().to_string(),
					uri: Some(video_url),
					artwork_url: entry
						.thumbnail()
						.map(|url| url.to_string())
						.or_else(|| entry.thumbnails().first().map(|url| url.to_string())),
					isrc: None,
					source_name: "youtube".to_string(),
				}))
			})
			.collect();

		if tracks.is_empty() {
			return Err(ResolverError::NotFound("No tracks found in playlist".to_string()));
		}

		Ok(LoadResult {
			load_type: LoadType::Playlist,
			tracks,
			playlist_info: Some(PlaylistInfo {
				name,
				selected_track: None,
			}),
		})
	}
}

fn track_from_entry(entry: &libytdlp::MediaEntry, position: usize) -> Option<Track> {
	let id = entry.id()?;
	let video_url = format!("https://www.youtube.com/watch?v={id}");
	let artwork_url = entry
		.thumbnail()
		.map(|url| url.to_string())
		.or_else(|| entry.thumbnails().first().map(|url| url.to_string()));

	Some(new_track(TrackInfo {
		identifier: video_url.clone(),
		is_seekable: true,
		author: entry.uploader().unwrap_or_default().to_string(),
		length: entry.duration_secs().map(|d| d as u64 * 1000).unwrap_or(0),
		is_stream: false,
		position: position as u64,
		title: entry.title().unwrap_or_default().to_string(),
		uri: Some(entry.url().map(|url| url.to_string()).unwrap_or(video_url)),
		artwork_url,
		isrc: None,
		source_name: "youtube".to_string(),
	}))
}
fn first_entry_watch_url(info: &MediaInfo) -> Option<String> {
	let entries = info.entries();
	if !entries.is_empty() && info.best_audio_url().is_none() {
		let id = entries.first()?.id()?;
		return Some(format!("https://www.youtube.com/watch?v={}", id));
	}

	None
}

fn track_from_info(info: &MediaInfo, stream_url: String) -> LoadResult {
	let video_url = info
		.webpage_url()
		.map(|url| url.to_string())
		.or_else(|| info.id().map(|id| format!("https://www.youtube.com/watch?v={}", id)))
		.unwrap_or_default();

	let is_stream = info.is_live().unwrap_or(false);

	LoadResult {
		load_type: LoadType::Track,
		tracks: vec![new_track(TrackInfo {
			identifier: video_url.clone(),
			is_seekable: !is_stream,
			author: info.uploader().or_else(|| info.channel()).unwrap_or_default().to_string(),
			length: info.duration_secs().map(|d| d as u64 * 1000).unwrap_or(0),
			is_stream,
			position: 0,
			title: info.title().unwrap_or_default().to_string(),
			uri: Some(stream_url),
			artwork_url: info.thumbnail().map(|url| url.to_string()),
			isrc: None,
			source_name: "youtube".to_string(),
		})],
		playlist_info: None,
	}
}

async fn probe_stream_url(stream_url: &str) -> bool {
	let client = match reqwest::Client::builder().timeout(Duration::from_secs(10)).build() {
		Ok(client) => client,
		Err(_) => return true,
	};

	let status = client
		.get(stream_url)
		.header("User-Agent", FFMPEG_USER_AGENT)
		.header("Accept", "*/*")
		.send()
		.await
		.map(|response| response.status());

	matches!(
		status,
		Ok(status) if status == reqwest::StatusCode::OK || status == reqwest::StatusCode::PARTIAL_CONTENT
	)
}

impl SourceManager for YtDlpSourceManager {
	fn name(&self) -> &'static str {
		"youtube"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		is_youtube_url(identifier)
			|| is_youtube_search(identifier)
			|| is_youtube_playlist(identifier)
			|| (!identifier.contains("://")
				&& !identifier.contains("soundcloud")
				&& !identifier.contains("bandcamp")
				&& !identifier.contains(".mp3"))
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		if is_youtube_search(identifier) {
			return self.search(identifier).await;
		}

		if is_youtube_playlist(identifier) {
			return self.load_playlist(identifier).await;
		}

		if is_youtube_url(identifier) || !identifier.contains("://") {
			return self.resolve_video(identifier).await;
		}

		Err(ResolverError::UnsupportedSource("Not a YouTube identifier".to_string()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn manager() -> YtDlpSourceManager {
		let client = Arc::new(Client::new("{}").expect("test client"));

		YtDlpSourceManager::new(client)
	}

	#[test]
	fn mix_watch_url_is_accepted_and_takes_the_playlist_path() {
		let manager = manager();
		let mix = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ";

		assert!(manager.can_handle(mix));
		assert!(YOUTUBE_PLAYLIST_REGEX.is_match(mix));
	}

	#[test]
	fn mix_playlist_url_is_accepted() {
		let manager = manager();

		assert!(manager.can_handle("https://www.youtube.com/playlist?list=RDdQw4w9WgXcQ"));
		assert!(manager.can_handle("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ"));
	}

	#[test]
	fn lookalike_hosts_and_non_youtube_playlist_urls_are_rejected() {
		let manager = manager();

		assert!(!manager.can_handle("https://notyoutube.com/watch?v=dQw4w9WgXcQ"));
		assert!(!manager.can_handle("https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ"));
		assert!(!manager.can_handle("https://example.com/youtube.com/watch?v=dQw4w9WgXcQ"));
		assert!(!YOUTUBE_PLAYLIST_REGEX.is_match("https://example.com/watch?list=not-youtube"));
	}
}
