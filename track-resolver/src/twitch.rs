use std::sync::Arc;

use libytdlp::Client;

use crate::{LoadResult, LoadType, ResolverError, Result, SourceManager, TrackInfo, new_track};

fn is_channel_char(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn is_twitch_url(identifier: &str) -> bool {
	let Ok(url) = reqwest::Url::parse(identifier) else {
		return false;
	};
	if !matches!(url.scheme(), "http" | "https") {
		return false;
	}

	let Some(host) = url.host_str() else {
		return false;
	};
	if host != "twitch.tv" && !host.ends_with(".twitch.tv") {
		return false;
	}

	url.path_segments()
		.and_then(|mut segments| segments.next())
		.and_then(|segment| segment.as_bytes().first())
		.is_some_and(|byte| is_channel_char(*byte))
}

pub struct TwitchSourceManager {
	client: Arc<Client>,
}

impl TwitchSourceManager {
	pub fn new(client: Arc<Client>) -> Self {
		Self { client }
	}

	async fn resolve_stream(&self, url: &str) -> Result<LoadResult> {
		let client = self.client.clone();
		let url_owned = url.to_string();

		let info = tokio::task::spawn_blocking(move || client.extract(&url_owned))
			.await
			.map_err(|e| ResolverError::FetchError(format!("extract task failed: {}", e)))?
			.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let stream_url = info
			.best_audio_url()
			.map(|url| url.to_string())
			.or_else(|| {
				info.value()
					.get("url")
					.and_then(|url| url.as_str())
					.map(|url| url.to_string())
			})
			.ok_or(ResolverError::NoAudioStream)?;

		let title = info.title().unwrap_or("Twitch Stream").to_string();
		let author = info.uploader().or_else(|| info.channel()).unwrap_or("Twitch").to_string();
		let thumbnail = info.thumbnail().map(|url| url.to_string());

		Ok(LoadResult {
			load_type: LoadType::Track,
			tracks: vec![new_track(TrackInfo {
				identifier: url.to_string(),
				is_seekable: false,
				author,
				length: 0,
				is_stream: true,
				position: 0,
				title,
				uri: Some(stream_url),
				artwork_url: thumbnail,
				isrc: None,
				source_name: "twitch".to_string(),
			})],
			playlist_info: None,
		})
	}
}

impl SourceManager for TwitchSourceManager {
	fn name(&self) -> &'static str {
		"twitch"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		is_twitch_url(identifier)
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		self.resolve_stream(identifier).await
	}
}

#[cfg(test)]
mod tests {
	use super::is_twitch_url;

	#[test]
	fn twitch_urls_require_a_twitch_hostname_and_channel_path() {
		assert!(is_twitch_url("https://www.twitch.tv/channel_name"));
		assert!(is_twitch_url("https://clips.twitch.tv/clipSlug"));
		assert!(!is_twitch_url("https://not-twitch.tv/channel"));
		assert!(!is_twitch_url("https://twitch.tv.evil.example/channel"));
		assert!(!is_twitch_url("https://example.com/twitch.tv/channel"));
		assert!(!is_twitch_url("https://twitch.tv/"));
	}
}
