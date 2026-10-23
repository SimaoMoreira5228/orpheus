use crate::{LoadResult, LoadType, ResolverError, Result, SourceManager, TrackInfo, new_track};

fn is_bandcamp_url(identifier: &str) -> bool {
	let Ok(url) = reqwest::Url::parse(identifier) else {
		return false;
	};
	if !matches!(url.scheme(), "http" | "https") {
		return false;
	}

	url.host_str()
		.is_some_and(|host| host == "bandcamp.com" || host.strip_suffix(".bandcamp.com").is_some())
}

fn quoted_attr<'a>(html: &'a str, marker: &str) -> Option<&'a str> {
	let pos = html.find(marker)?;
	let after = &html[pos + marker.len()..];
	let end = after.find('"')?;

	if end == 0 {
		return None;
	}

	Some(&after[..end])
}

pub struct BandcampSourceManager {
	http_client: reqwest::Client,
}

impl Default for BandcampSourceManager {
	fn default() -> Self {
		Self::new()
	}
}

impl BandcampSourceManager {
	pub fn new() -> Self {
		Self {
			http_client: reqwest::Client::builder()
				.user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
				.build()
				.unwrap(),
		}
	}

	async fn resolve_url(&self, url: &str) -> Result<LoadResult> {
		let response = self
			.http_client
			.get(url)
			.header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
			.send()
			.await
			.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let html = response.text().await.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let embed_data = self.parse_embed_data(&html)?;

		if embed_data.get("track").is_some() {
			self.parse_single_track(&embed_data, url)
		} else if embed_data.get("album").is_some() {
			self.parse_album(&embed_data, url)
		} else {
			Err(ResolverError::NotFound("No track or album found".to_string()))
		}
	}

	fn parse_embed_data(&self, html: &str) -> Result<serde_json::Value> {
		let json_str = quoted_attr(html, "data-tralbum=\"")
			.or_else(|| quoted_attr(html, "data-embed=\""))
			.ok_or_else(|| ResolverError::ParseError("No embed data found".to_string()))?;

		let decoded = html_escape::decode_html_entities(json_str);

		serde_json::from_str(&decoded).map_err(|e| ResolverError::ParseError(e.to_string()))
	}

	fn parse_single_track(&self, data: &serde_json::Value, url: &str) -> Result<LoadResult> {
		let track_data = data
			.get("track")
			.and_then(|t| t.as_object())
			.ok_or_else(|| ResolverError::ParseError("No track data".to_string()))?;

		let title = track_data
			.get("title")
			.and_then(|t| t.as_str())
			.unwrap_or("Unknown")
			.to_string();

		let duration_ms = track_data.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0) * 1000.0;

		let file_url = track_data
			.get("file")
			.and_then(|f| f.get("mp3-128"))
			.and_then(|u| u.as_str())
			.map(|s| s.to_string())
			.ok_or(ResolverError::NoAudioStream)?;

		let artwork_url = data
			.get("album")
			.and_then(|a| a.get("art_id"))
			.and_then(|id| id.as_i64())
			.map(|id| {
				format!(
					"https://bandcamp.com/EmbeddedPlayer/album={}/size=large/artwork=large/transparent.png",
					id
				)
			});

		let artist = data
			.get("album")
			.and_then(|a| a.get("artist"))
			.and_then(|a| a.as_str())
			.unwrap_or("Unknown Artist")
			.to_string();

		Ok(LoadResult {
			load_type: LoadType::Track,
			tracks: vec![new_track(TrackInfo {
				identifier: url.to_string(),
				is_seekable: true,
				author: artist,
				length: duration_ms as u64,
				is_stream: false,
				position: 0,
				title,
				uri: Some(file_url),
				artwork_url,
				isrc: None,
				source_name: "bandcamp".to_string(),
			})],
			playlist_info: None,
		})
	}

	fn parse_album(&self, data: &serde_json::Value, url: &str) -> Result<LoadResult> {
		let album_data = data
			.get("album")
			.and_then(|a| a.as_object())
			.ok_or_else(|| ResolverError::ParseError("No album data".to_string()))?;

		let tracks_data = data
			.get("trackinfo")
			.and_then(|t| t.as_array())
			.ok_or_else(|| ResolverError::ParseError("No track info".to_string()))?;

		let album_title = album_data
			.get("title")
			.and_then(|t| t.as_str())
			.unwrap_or("Unknown Album")
			.to_string();

		let artist = album_data
			.get("artist")
			.and_then(|a| a.as_str())
			.unwrap_or("Unknown Artist")
			.to_string();

		let artwork_url = album_data.get("art_id").and_then(|id| id.as_i64()).map(|id| {
			format!(
				"https://bandcamp.com/EmbeddedPlayer/album={}/size=large/artwork=large/transparent.png",
				id
			)
		});

		let mut tracks = Vec::new();

		for track in tracks_data {
			let track_obj = track
				.as_object()
				.ok_or_else(|| ResolverError::ParseError("Invalid track".to_string()))?;

			let title = track_obj
				.get("title")
				.and_then(|t| t.as_str())
				.unwrap_or("Unknown Track")
				.to_string();

			let duration_ms = track_obj.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0) * 1000.0;

			let file_url = track_obj
				.get("file")
				.and_then(|f| f.get("mp3-128"))
				.and_then(|u| u.as_str())
				.map(|s| s.to_string());

			if let Some(uri) = file_url {
				tracks.push(new_track(TrackInfo {
					identifier: format!("{}#{}", url, title),
					is_seekable: true,
					author: artist.clone(),
					length: duration_ms as u64,
					is_stream: false,
					position: 0,
					title,
					uri: Some(uri),
					artwork_url: artwork_url.clone(),
					isrc: None,
					source_name: "bandcamp".to_string(),
				}));
			}
		}

		if tracks.is_empty() {
			return Err(ResolverError::NoAudioStream);
		}

		Ok(LoadResult {
			load_type: LoadType::Playlist,
			tracks,
			playlist_info: Some(PlaylistInfo {
				name: Some(album_title),
				selected_track: None,
			}),
		})
	}
}

impl SourceManager for BandcampSourceManager {
	fn name(&self) -> &'static str {
		"bandcamp"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		is_bandcamp_url(identifier)
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		self.resolve_url(identifier).await
	}
}

#[cfg(test)]
mod tests {
	use super::is_bandcamp_url;

	#[test]
	fn bandcamp_urls_require_a_bandcamp_hostname() {
		assert!(is_bandcamp_url("https://bandcamp.com/album/example"));
		assert!(is_bandcamp_url("https://artist.bandcamp.com/track/example"));
		assert!(!is_bandcamp_url("https://not-bandcamp.com/track/example"));
		assert!(!is_bandcamp_url("https://bandcamp.com.evil.example/track/example"));
		assert!(!is_bandcamp_url("https://example.com/bandcamp.com/track/example"));
	}
}

mod html_escape {
	pub fn decode_html_entities(s: &str) -> String {
		let mut result = String::new();
		let mut chars = s.chars().peekable();

		while let Some(c) = chars.next() {
			if c == '&' {
				let mut entity = String::new();
				while let Some(&nc) = chars.peek() {
					if nc == ';' {
						chars.next();
						break;
					}
					entity.push(chars.next().unwrap());
				}

				let decoded = match entity.as_str() {
					"amp" => "&",
					"lt" => "<",
					"gt" => ">",
					"quot" => "\"",
					"apos" => "'",
					_ => {
						result.push('&');
						result.push_str(&entity);
						result.push(';');
						continue;
					}
				};
				result.push_str(decoded);
			} else {
				result.push(c);
			}
		}

		result
	}
}

use crate::PlaylistInfo;
