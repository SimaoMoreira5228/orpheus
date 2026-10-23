use std::path::{Path, PathBuf};

use symphonia::core::formats::probe::Hint;
use symphonia::core::meta::MetadataOptions;

use crate::{LoadResult, LoadType, ResolverError, Result, SourceManager, TrackInfo, new_track};

pub struct LocalFileSourceManager {
	supported_extensions: Vec<&'static str>,
}

impl Default for LocalFileSourceManager {
	fn default() -> Self {
		Self::new()
	}
}

impl LocalFileSourceManager {
	pub fn new() -> Self {
		Self {
			supported_extensions: vec!["mp3", "flac", "wav", "ogg", "m4a", "webm", "aac", "opus", "wma", "aiff"],
		}
	}

	fn can_handle_path(&self, path: &Path) -> bool {
		path.extension()
			.and_then(|e| e.to_str())
			.map(|e| self.supported_extensions.contains(&e.to_lowercase().as_str()))
			.unwrap_or(false)
	}

	fn extract_metadata(&self, path: &Path) -> TrackInfo {
		let file_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string();

		let title = file_name.clone();
		let author = String::new();
		let mut duration_ms: Option<u64> = None;
		let artwork_url: Option<String> = None;

		if let Ok(file) = std::fs::File::open(path) {
			let mss = symphonia::core::io::MediaSourceStream::new(Box::new(file), Default::default());
			let mut hint = Hint::new();
			if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
				hint.with_extension(ext);
			}

			if let Ok(format) = symphonia::default::get_probe().probe(
				&hint,
				mss,
				symphonia::core::formats::FormatOptions::default(),
				MetadataOptions::default(),
			) && let Some(track) = format.tracks().first()
			{
				let sample_rate = track.codec_params.as_ref().and_then(|params| params.audio()?.sample_rate);

				if let (Some(n_frames), Some(sample_rate)) = (track.num_frames, sample_rate) {
					duration_ms = Some((n_frames * 1000) / sample_rate as u64);
				}
			}
		}

		TrackInfo {
			identifier: path.to_string_lossy().to_string(),
			is_seekable: true,
			author: if author.is_empty() {
				"Unknown Artist".to_string()
			} else {
				author
			},
			length: duration_ms.unwrap_or(0),
			is_stream: false,
			position: 0,
			title,
			uri: Some(format!("file://{}", path.to_string_lossy())),
			artwork_url,
			isrc: None,
			source_name: "local".to_string(),
		}
	}

	fn load_directory(&self, path: &Path) -> Result<LoadResult> {
		let mut tracks = Vec::new();

		let entries = std::fs::read_dir(path).map_err(|e| ResolverError::IoError(e.to_string()))?;

		let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();

		entries.sort_by_key(|entry| entry.path());

		for entry in entries {
			let entry_path = entry.path();
			if entry_path.is_file() && self.can_handle_path(&entry_path) {
				let info = self.extract_metadata(&entry_path);
				tracks.push(new_track(info));
			}
		}

		if tracks.is_empty() {
			return Err(ResolverError::NotFound("No audio files found".to_string()));
		}

		Ok(LoadResult {
			load_type: LoadType::Playlist,
			tracks,
			playlist_info: Some(PlaylistInfo {
				name: path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()),
				selected_track: None,
			}),
		})
	}
}

impl SourceManager for LocalFileSourceManager {
	fn name(&self) -> &'static str {
		"local"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		let path = Path::new(identifier);

		if let Some(rest) = identifier.strip_prefix("file://") {
			return self.can_handle_path(Path::new(rest));
		}

		path.exists() && (path.is_file() || path.is_dir())
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		let path = if let Some(rest) = identifier.strip_prefix("file://") {
			PathBuf::from(rest)
		} else {
			PathBuf::from(identifier)
		};

		if path.is_dir() {
			return self.load_directory(&path);
		}

		if !self.can_handle_path(&path) {
			return Err(ResolverError::InvalidIdentifier("Unsupported file format".to_string()));
		}

		let info = self.extract_metadata(&path);

		Ok(LoadResult {
			load_type: LoadType::Track,
			tracks: vec![new_track(info)],
			playlist_info: None,
		})
	}
}

use crate::PlaylistInfo;
