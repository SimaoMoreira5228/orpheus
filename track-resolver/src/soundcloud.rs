use crate::{
	LoadResult, LoadType, PlaylistInfo, ResolverError, Result, RichResults, SourceManager, Track, TrackInfo, new_track,
};

fn is_soundcloud_url(identifier: &str) -> bool {
	let Ok(url) = reqwest::Url::parse(identifier) else {
		return false;
	};
	if !matches!(url.scheme(), "http" | "https") {
		return false;
	}

	let Some(host) = url.host_str() else {
		return false;
	};

	host == "snd.sc" || host == "soundcloud.com" || host.strip_suffix(".soundcloud.com").is_some()
}

fn is_soundcloud_search(identifier: &str) -> bool {
	identifier.to_ascii_lowercase().starts_with("scsearch:")
}

fn client_id_value(html: &str) -> Option<&str> {
	let mut rest = html;

	loop {
		let pos = rest.find("client_id=")?;
		let after = &rest[pos + "client_id=".len()..];
		let len = after
			.bytes()
			.take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'%')
			.count();

		if len > 0 {
			return Some(&after[..len]);
		}

		rest = &rest[pos + 1..];
	}
}

pub struct SoundCloudSourceManager {
	client_id: tokio::sync::Mutex<Option<String>>,
	http_client: reqwest::Client,
}

impl Default for SoundCloudSourceManager {
	fn default() -> Self {
		Self::new()
	}
}

impl SoundCloudSourceManager {
	pub fn new() -> Self {
		Self {
			client_id: tokio::sync::Mutex::new(None),
			http_client: reqwest::Client::builder()
				.user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
				.build()
				.unwrap(),
		}
	}

	async fn get_client_id(&self) -> Result<String> {
		let mut id = self.client_id.lock().await;
		if let Some(ref cid) = *id {
			return Ok(cid.clone());
		}

		let html = self
			.http_client
			.get("https://soundcloud.com/discover")
			.send()
			.await
			.map_err(|e| ResolverError::FetchError(e.to_string()))?
			.text()
			.await
			.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let cid = client_id_value(&html)
			.map(|m| {
				let mut decoded = String::new();
				let mut chars = m.chars().peekable();
				while let Some(c) = chars.next() {
					if c == '%' {
						let hex: String = chars.by_ref().take(2).collect();

						if hex.len() == 2
							&& let Ok(byte) = u8::from_str_radix(&hex, 16)
						{
							decoded.push(byte as char);
						}
					} else if c == '+' {
						decoded.push(' ');
					} else {
						decoded.push(c);
					}
				}
				decoded
			})
			.ok_or_else(|| ResolverError::FetchError("Failed to find client_id".to_string()))?;

		*id = Some(cid.clone());
		Ok(cid)
	}

	async fn resolve_url(&self, url: &str) -> Result<LoadResult> {
		let client_id = self.get_client_id().await?;

		let api_url = format!(
			"https://api.soundcloud.com/resolve?url={}&client_id={}",
			urlencoding::encode(url),
			client_id
		);

		let response = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
			.send()
			.await
			.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let json: serde_json::Value = response.json().await.map_err(|e| ResolverError::ParseError(e.to_string()))?;

		let kind = json.get("kind").and_then(|v| v.as_str()).unwrap_or("track");

		match kind {
			"playlist" | "set" => {
				let tracks = json
					.get("tracks")
					.and_then(|t| t.as_array())
					.map(|arr| arr.iter().filter_map(|t| self.track_from_json(t)).collect())
					.unwrap_or_default();

				let name = json.get("title").and_then(|t| t.as_str()).map(|s| s.to_string());

				Ok(LoadResult {
					load_type: LoadType::Playlist,
					tracks,
					playlist_info: Some(PlaylistInfo {
						name,
						selected_track: None,
					}),
				})
			}
			"track" => {
				let track = self
					.track_from_json(&json)
					.ok_or_else(|| ResolverError::ParseError("Failed to parse track".to_string()))?;

				Ok(LoadResult {
					load_type: LoadType::Track,
					tracks: vec![track],
					playlist_info: None,
				})
			}
			_ => Err(ResolverError::UnsupportedSource(format!("Unknown SoundCloud kind: {}", kind))),
		}
	}

	fn track_from_json(&self, json: &serde_json::Value) -> Option<Track> {
		let title = json.get("title")?.as_str()?.to_string();
		let uri = json.get("uri")?.as_str()?.to_string();
		let duration_ms = json.get("duration")?.as_i64()?;
		let artwork_url = json
			.get("artwork_url")
			.and_then(|v| v.as_str())
			.map(|s| s.replace("-large", "-t500x500"));

		let author = json
			.get("user")
			.and_then(|u| u.get("username"))
			.and_then(|n| n.as_str())
			.unwrap_or("Unknown Artist")
			.to_string();

		let stream_url = json.get("stream_url").and_then(|s| s.as_str()).map(|s| s.to_string());

		Some(new_track(TrackInfo {
			identifier: uri.clone(),
			is_seekable: true,
			author,
			length: duration_ms as u64,
			is_stream: false,
			position: 0,
			title,
			uri: Some(stream_url.unwrap_or(uri)),
			artwork_url,
			isrc: None,
			source_name: "soundcloud".to_string(),
		}))
	}

	async fn search(&self, query: &str) -> Result<LoadResult> {
		let search_query = query.trim_start_matches("scsearch:");
		let tracks = self.search_tracks(search_query).await?;

		Ok(LoadResult {
			load_type: if tracks.is_empty() {
				LoadType::Empty
			} else {
				LoadType::Search
			},
			tracks,
			playlist_info: None,
		})
	}

	pub(crate) async fn rich_search(&self, query: &str) -> Result<RichResults> {
		let search_query = query.trim_start_matches("scsearch:");
		let tracks = self.search_tracks(search_query).await?;

		Ok(RichResults {
			tracks,
			..RichResults::empty()
		})
	}

	async fn search_tracks(&self, search_query: &str) -> Result<Vec<Track>> {
		let client_id = self.get_client_id().await?;
		let encoded = urlencoding::encode(search_query);

		let api_url = format!("https://api.soundcloud.com/tracks?q={}&client_id={}", encoded, client_id);

		let response = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
			.send()
			.await
			.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let json: serde_json::Value = response.json().await.map_err(|e| ResolverError::ParseError(e.to_string()))?;

		Ok(json
			.as_array()
			.map(|arr| arr.iter().filter_map(|t| self.track_from_json(t)).collect())
			.unwrap_or_default())
	}
}

impl SourceManager for SoundCloudSourceManager {
	fn name(&self) -> &'static str {
		"soundcloud"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		is_soundcloud_url(identifier) || is_soundcloud_search(identifier)
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		if is_soundcloud_search(identifier) {
			return self.search(identifier).await;
		}

		self.resolve_url(identifier).await
	}
}

#[cfg(test)]
mod tests {
	use super::is_soundcloud_url;

	#[test]
	fn soundcloud_urls_require_a_soundcloud_hostname() {
		assert!(is_soundcloud_url("https://soundcloud.com/artist/track"));
		assert!(is_soundcloud_url("https://on.soundcloud.com/short"));
		assert!(is_soundcloud_url("https://snd.sc/short"));
		assert!(!is_soundcloud_url("https://not-soundcloud.com/artist/track"));
		assert!(!is_soundcloud_url("https://soundcloud.com.evil.example/artist/track"));
		assert!(!is_soundcloud_url("https://example.com/soundcloud.com/artist/track"));
	}
}

mod urlencoding {
	pub fn encode(input: &str) -> String {
		let mut encoded = String::new();
		for byte in input.bytes() {
			match byte {
				b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
					encoded.push(byte as char);
				}
				_ => {
					encoded.push_str(&format!("%{:02X}", byte));
				}
			}
		}
		encoded
	}
}
