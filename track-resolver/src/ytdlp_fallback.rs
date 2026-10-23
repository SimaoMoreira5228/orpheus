use std::sync::Arc;

use libytdlp::Client;

use crate::{LoadResult, LoadType, ResolverError, Result, SourceManager, TrackInfo, new_track};

const AUDIO_EXTENSIONS: [&str; 11] = [
	"mp3", "flac", "wav", "ogg", "m4a", "webm", "aac", "opus", "wma", "aiff", "caf",
];

fn is_http_url(identifier: &str) -> bool {
	identifier.starts_with("http://") || identifier.starts_with("https://")
}

fn has_audio_extension(identifier: &str) -> bool {
	let path = identifier.split(['?', '#']).next().unwrap_or(identifier).to_ascii_lowercase();

	AUDIO_EXTENSIONS.iter().any(|ext| path.ends_with(&format!(".{ext}")))
}

fn is_vimeo_url(identifier: &str) -> bool {
	let Some(host) = crate::url_host(identifier) else {
		return false;
	};

	host == "vimeo.com" || host.ends_with(".vimeo.com")
}

fn source_label(info: &libytdlp::MediaInfo) -> String {
	info.extractor()
		.map(|extractor| extractor.to_ascii_lowercase())
		.filter(|extractor| !extractor.is_empty())
		.unwrap_or_else(|| "ytdlp".to_string())
}

pub struct YtDlpFallbackSourceManager {
	client: Arc<Client>,
}

impl YtDlpFallbackSourceManager {
	pub fn new(client: Arc<Client>) -> Self {
		Self { client }
	}

	async fn resolve_url(&self, url: &str) -> Result<LoadResult> {
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

		let is_stream = info.is_live().unwrap_or(false);
		let label = source_label(&info);

		Ok(LoadResult {
			load_type: LoadType::Track,
			tracks: vec![new_track(TrackInfo {
				identifier: url.to_string(),
				is_seekable: !is_stream,
				author: info.uploader().or_else(|| info.channel()).unwrap_or(&label).to_string(),
				length: info.duration_secs().map(|d| d as u64 * 1000).unwrap_or(0),
				is_stream,
				position: 0,
				title: info.title().unwrap_or(url).to_string(),
				uri: Some(stream_url),
				artwork_url: info.thumbnail().map(|url| url.to_string()),
				isrc: None,
				source_name: label,
			})],
			playlist_info: None,
		})
	}
}

impl SourceManager for YtDlpFallbackSourceManager {
	fn name(&self) -> &'static str {
		"ytdlp"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		// NOTE: the generic fallback excludes Vimeo because its yt-dlp extractor requires login.
		is_http_url(identifier) && !has_audio_extension(identifier) && !is_vimeo_url(identifier)
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		self.resolve_url(identifier).await
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn manager() -> YtDlpFallbackSourceManager {
		let client = Arc::new(Client::new("{}").expect("test client"));

		YtDlpFallbackSourceManager::new(client)
	}

	#[test]
	fn claims_long_tail_page_urls() {
		let manager = manager();

		assert!(manager.can_handle("https://www.tiktok.com/@user/video/123456789"));
		assert!(manager.can_handle("https://x.com/user/status/123456789"));
		assert!(manager.can_handle("https://www.reddit.com/r/music/comments/abc/song/"));
		assert!(manager.can_handle("https://example.com/some/page"));
	}

	#[test]
	fn yields_audio_files_to_http() {
		let manager = manager();

		assert!(!manager.can_handle("https://example.com/song.mp3"));
		assert!(!manager.can_handle("https://example.com/song.MP3?token=abc"));
		assert!(!manager.can_handle("https://example.com/episode.opus#t=10"));
	}

	#[test]
	fn does_not_claim_disabled_vimeo_urls() {
		let manager = manager();

		assert!(!manager.can_handle("https://vimeo.com/501958307"));
		assert!(!manager.can_handle("https://player.vimeo.com/video/501958307"));
		assert!(manager.can_handle("https://vimeo.com.evil.example/video/501958307"));
	}

	#[test]
	fn rejects_non_http_identifiers() {
		let manager = manager();

		assert!(!manager.can_handle("never gonna give you up"));
		assert!(!manager.can_handle("ytsearch:never gonna give you up"));
		assert!(!manager.can_handle("/music/song.mp3"));
		assert!(!manager.can_handle(""));
	}
}
