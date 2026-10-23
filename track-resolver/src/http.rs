use crate::{LoadResult, LoadType, ResolverError, Result, SourceManager, TrackInfo, new_track};

const AUDIO_EXTENSIONS: [&str; 11] = [
	"mp3", "flac", "wav", "ogg", "m4a", "webm", "aac", "opus", "wma", "aiff", "caf",
];

fn is_http_url(identifier: &str) -> bool {
	identifier.starts_with("http://") || identifier.starts_with("https://")
}

fn url_path(url: &str) -> Option<&str> {
	let authority_and_path = url.split_once("://")?.1;
	let end = authority_and_path.find(['?', '#']).unwrap_or(authority_and_path.len());
	let authority_and_path = &authority_and_path[..end];

	match authority_and_path.find('/') {
		Some(pos) => Some(&authority_and_path[pos..]),
		None => Some(""),
	}
}

pub struct HttpSourceManager {
	http_client: reqwest::Client,
}

impl Default for HttpSourceManager {
	fn default() -> Self {
		Self::new()
	}
}

impl HttpSourceManager {
	pub fn new() -> Self {
		Self {
			http_client: reqwest::Client::builder()
				.user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
				.build()
				.unwrap(),
		}
	}

	fn is_audio_url(&self, url: &str) -> bool {
		match url_path(url) {
			Some(path) => {
				let path_lower = path.to_ascii_lowercase();

				AUDIO_EXTENSIONS.iter().any(|ext| path_lower.ends_with(ext))
			}
			None => false,
		}
	}

	async fn load_url(&self, url: &str) -> Result<LoadResult> {
		let response = self
			.http_client
			.head(url)
			.send()
			.await
			.map_err(|e| ResolverError::FetchError(e.to_string()))?;

		let content_type = response
			.headers()
			.get(reqwest::header::CONTENT_TYPE)
			.and_then(|v| v.to_str().ok())
			.unwrap_or("")
			.to_lowercase();

		let is_audio = content_type.starts_with("audio/") || self.is_audio_url(url);

		if !is_audio {
			return Err(ResolverError::UnsupportedSource(
				"URL does not point to an audio file".to_string(),
			));
		}

		let content_length = response
			.headers()
			.get(reqwest::header::CONTENT_LENGTH)
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.parse::<u64>().ok());

		let file_name = url_path(url)
			.map(|path| path.rsplit('/').next().unwrap_or("").to_string())
			.unwrap_or_else(|| "Unknown".to_string());

		let (title, author) = self.extract_metadata_from_filename(&file_name);

		let is_stream = content_length.is_none() || content_length == Some(0);

		Ok(LoadResult {
			load_type: LoadType::Track,
			tracks: vec![new_track(TrackInfo {
				identifier: url.to_string(),
				is_seekable: !is_stream,
				author,
				length: 0,
				is_stream,
				position: 0,
				title,
				uri: Some(url.to_string()),
				artwork_url: None,
				isrc: None,
				source_name: "http".to_string(),
			})],
			playlist_info: None,
		})
	}

	fn extract_metadata_from_filename(&self, filename: &str) -> (String, String) {
		let name = Path::new(filename)
			.file_stem()
			.and_then(|s| s.to_str())
			.unwrap_or("Unknown")
			.to_string();

		if name.contains(" - ") {
			let parts: Vec<&str> = name.splitn(2, " - ").collect();
			if parts.len() == 2 {
				return (parts[1].to_string(), parts[0].to_string());
			}
		}

		(name, "Unknown Artist".to_string())
	}
}

impl SourceManager for HttpSourceManager {
	fn name(&self) -> &'static str {
		"http"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		is_http_url(identifier)
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		self.load_url(identifier).await
	}
}

use std::path::Path;
