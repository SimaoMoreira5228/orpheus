use std::sync::Arc;

use libytdlp::Client;

use crate::{LoadResult, LoadType, ResolverError, Result, SourceManager, Track, TrackInfo, new_track};

fn has_m3u_suffix(url_lower: &str) -> bool {
	url_lower.ends_with(".m3u") || url_lower.ends_with(".m3u8")
}

fn has_pls_suffix(url_lower: &str) -> bool {
	url_lower.ends_with(".pls")
}

pub struct PlaylistSourceManager {
	http_client: reqwest::Client,
	client: Arc<Client>,
}

impl PlaylistSourceManager {
	pub fn new(client: Arc<Client>) -> Self {
		Self {
			http_client: reqwest::Client::builder()
				.user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
				.build()
				.unwrap(),
			client,
		}
	}

	fn can_handle_playlist_url(&self, url: &str) -> bool {
		let url_lower = url.to_ascii_lowercase();

		if has_m3u_suffix(&url_lower) || has_pls_suffix(&url_lower) {
			return true;
		}

		let playlist_patterns = [
			"playlist",
			"list=",
			"album",
			"set=",
			"tracks=",
			"soundcloud.com/",
			"bandcamp.com/",
			"youtube.com/",
		];

		playlist_patterns.iter().any(|p| url_lower.contains(p))
	}

	async fn parse_m3u(&self, content: &str) -> Result<Vec<String>> {
		let mut urls = Vec::new();

		for line in content.lines() {
			let line = line.trim();

			if line.is_empty() || line.starts_with('#') {
				continue;
			}

			urls.push(line.to_string());
		}

		if urls.is_empty() {
			return Err(ResolverError::ParseError("No URLs found in M3U playlist".to_string()));
		}

		Ok(urls)
	}

	async fn parse_m3u8(&self, content: &str) -> Result<Vec<String>> {
		self.parse_m3u(content).await
	}

	async fn parse_pls(&self, content: &str) -> Result<Vec<String>> {
		let mut urls = Vec::new();
		let mut in_track = false;

		for line in content.lines() {
			let line = line.trim().to_lowercase();

			if line.starts_with("[track]") {
				in_track = true;
			} else if line.starts_with("[") && line != "[track]" {
				in_track = false;
			}

			if in_track
				&& line.starts_with("file")
				&& let Some(pos) = line.find('=')
			{
				let url = line[pos + 1..].trim().to_string();

				if !url.is_empty() {
					urls.push(url);
				}
			}
		}

		if urls.is_empty() {
			return Err(ResolverError::ParseError("No URLs found in PLS playlist".to_string()));
		}

		Ok(urls)
	}

	async fn load_playlist_url(&self, url: &str) -> Result<LoadResult> {
		let url_lower = url.to_ascii_lowercase();

		if has_m3u_suffix(&url_lower) || url_lower.contains(".m3u") || url_lower.contains(".m3u8") {
			let response = self
				.http_client
				.get(url)
				.send()
				.await
				.map_err(|e| ResolverError::FetchError(e.to_string()))?;

			let content = response.text().await.map_err(|e| ResolverError::FetchError(e.to_string()))?;

			let urls = if url_lower.contains(".m3u8") {
				self.parse_m3u8(&content).await?
			} else {
				self.parse_m3u(&content).await?
			};

			let tracks: Vec<Track> = urls
				.into_iter()
				.enumerate()
				.map(|(i, url)| {
					let title = url
						.rsplit('/')
						.next()
						.unwrap_or("Track")
						.split('.')
						.next()
						.unwrap_or("Track")
						.to_string();

					new_track(TrackInfo {
						identifier: url.clone(),
						is_seekable: true,
						author: String::new(),
						length: 0,
						is_stream: false,
						position: i as u64,
						title,
						uri: Some(url),
						artwork_url: None,
						isrc: None,
						source_name: "http".to_string(),
					})
				})
				.collect();

			let name = url.rsplit('/').next().map(|s| s.to_string());

			return Ok(LoadResult {
				load_type: LoadType::Playlist,
				tracks,
				playlist_info: Some(PlaylistInfo {
					name,
					selected_track: None,
				}),
			});
		}

		if has_pls_suffix(&url_lower) {
			let response = self
				.http_client
				.get(url)
				.send()
				.await
				.map_err(|e| ResolverError::FetchError(e.to_string()))?;

			let content = response.text().await.map_err(|e| ResolverError::FetchError(e.to_string()))?;

			let urls = self.parse_pls(&content).await?;

			let tracks: Vec<Track> = urls
				.into_iter()
				.enumerate()
				.map(|(i, url)| {
					new_track(TrackInfo {
						identifier: url.clone(),
						is_seekable: true,
						author: String::new(),
						length: 0,
						is_stream: false,
						position: i as u64,
						title: "Track".to_string(),
						uri: Some(url),
						artwork_url: None,
						isrc: None,
						source_name: "http".to_string(),
					})
				})
				.collect();

			return Ok(LoadResult {
				load_type: LoadType::Playlist,
				tracks,
				playlist_info: Some(PlaylistInfo {
					name: None,
					selected_track: None,
				}),
			});
		}

		let is_youtube_playlist =
			url_lower.contains("youtube.com") && (url_lower.contains("playlist") || url_lower.contains("list="));

		if is_youtube_playlist {
			return self.load_via_ytdlp(url).await;
		}

		Err(ResolverError::UnsupportedSource("Unsupported playlist format".to_string()))
	}

	async fn load_via_ytdlp(&self, url: &str) -> Result<LoadResult> {
		let client = self.client.clone();
		let url_owned = url.to_string();

		let info = tokio::task::spawn_blocking(move || client.extract(&url_owned))
			.await
			.map_err(|e| ResolverError::FetchError(format!("extract task failed: {}", e)))?
			.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let tracks: Vec<Track> = info
			.entries()
			.iter()
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
				name: info.title().map(|title| title.to_string()),
				selected_track: None,
			}),
		})
	}
}

impl SourceManager for PlaylistSourceManager {
	fn name(&self) -> &'static str {
		"playlist"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		self.can_handle_playlist_url(identifier)
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		self.load_playlist_url(identifier).await
	}
}

use crate::PlaylistInfo;
