pub mod bandcamp;
pub mod codec;
pub mod http;
pub mod local;
pub mod playlist;
pub mod plugin_source;
pub mod soundcloud;
pub mod spotify;
pub mod twitch;
pub mod ytdlp;
pub mod ytdlp_fallback;

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub use bandcamp::BandcampSourceManager;
pub use codec::{CodecError, DecodedTrack, decode_track, encode_track};
pub use http::HttpSourceManager;
pub use local::LocalFileSourceManager;
use parking_lot::RwLock;
pub use playlist::PlaylistSourceManager;
pub use plugin_source::PluginSourceManager;
pub use soundcloud::SoundCloudSourceManager;
pub use spotify::SpotifySourceManager;
use thiserror::Error;
pub use twitch::TwitchSourceManager;
pub use ytdlp::YtDlpSourceManager;
pub use ytdlp_fallback::YtDlpFallbackSourceManager;

#[allow(async_fn_in_trait)]
pub trait SourceManager: Send + Sync {
	fn name(&self) -> &'static str;
	fn can_handle(&self, identifier: &str) -> bool;
	async fn load(&self, identifier: &str) -> Result<LoadResult>;

	async fn rich_search(&self, query: &str) -> Result<RichResults> {
		let _ = query;

		Ok(RichResults::empty())
	}
}

#[derive(Error, Debug)]
pub enum ResolverError {
	#[error("Failed to fetch: {0}")]
	FetchError(String),
	#[error("No audio stream available")]
	NoAudioStream,
	#[error("Invalid identifier: {0}")]
	InvalidIdentifier(String),
	#[error("Unsupported source: {0}")]
	UnsupportedSource(String),
	#[error("Parse error: {0}")]
	ParseError(String),
	#[error("IO error: {0}")]
	IoError(String),
	#[error("Not found: {0}")]
	NotFound(String),
	#[error("Source timed out: {0}")]
	Timeout(String),
	#[error("Track codec error: {0}")]
	Codec(#[from] CodecError),
}

pub type Result<T> = std::result::Result<T, ResolverError>;

pub use orpheus_core::{
	AlbumRef, ArtistRef, LoadResult, LoadResultType as LoadType, LoadResultType, PlaylistInfo, PlaylistRef, RichResults,
	Track, TrackInfo,
};

use crate::spotify::{is_spotify_url, mirror_query};

pub fn new_track(info: TrackInfo) -> Track {
	Track {
		encoded: None,
		info,
		start_time: None,
		end_time: None,
		plugin_info: serde_json::json!({}),
		user_data: serde_json::json!({}),
	}
}

pub enum TrackSource {
	YtDlp(YtDlpSourceManager),
	SoundCloud(SoundCloudSourceManager),
	Spotify(SpotifySourceManager),
	Bandcamp(BandcampSourceManager),
	Playlist(PlaylistSourceManager),
	Local(LocalFileSourceManager),
	Http(HttpSourceManager),
	Twitch(TwitchSourceManager),
	YtDlpFallback(YtDlpFallbackSourceManager),
	Plugin(PluginSourceManager),
}

#[derive(Clone)]
struct ResolverSettings {
	source_order: Vec<String>,
	default_timeout_ms: u64,
	source_timeouts_ms: BTreeMap<String, u64>,
}

impl TrackSource {
	pub fn name(&self) -> &'static str {
		match self {
			TrackSource::YtDlp(m) => m.name(),
			TrackSource::SoundCloud(m) => m.name(),
			TrackSource::Spotify(m) => m.name(),
			TrackSource::Bandcamp(m) => m.name(),
			TrackSource::Playlist(m) => m.name(),
			TrackSource::Local(m) => m.name(),
			TrackSource::Http(m) => m.name(),
			TrackSource::Twitch(m) => m.name(),
			TrackSource::YtDlpFallback(m) => m.name(),
			TrackSource::Plugin(m) => m.name(),
		}
	}

	pub fn can_handle(&self, identifier: &str) -> bool {
		match self {
			TrackSource::YtDlp(m) => m.can_handle(identifier),
			TrackSource::SoundCloud(m) => m.can_handle(identifier),
			TrackSource::Spotify(m) => m.can_handle(identifier),
			TrackSource::Bandcamp(m) => m.can_handle(identifier),
			TrackSource::Playlist(m) => m.can_handle(identifier),
			TrackSource::Local(m) => m.can_handle(identifier),
			TrackSource::Http(m) => m.can_handle(identifier),
			TrackSource::Twitch(m) => m.can_handle(identifier),
			TrackSource::YtDlpFallback(m) => m.can_handle(identifier),
			TrackSource::Plugin(m) => m.can_handle(identifier),
		}
	}

	pub async fn load(&self, identifier: &str) -> Result<LoadResult> {
		match self {
			TrackSource::YtDlp(m) => m.load(identifier).await,
			TrackSource::SoundCloud(m) => m.load(identifier).await,
			TrackSource::Spotify(m) => m.load(identifier).await,
			TrackSource::Bandcamp(m) => m.load(identifier).await,
			TrackSource::Playlist(m) => m.load(identifier).await,
			TrackSource::Local(m) => m.load(identifier).await,
			TrackSource::Http(m) => m.load(identifier).await,
			TrackSource::Twitch(m) => m.load(identifier).await,
			TrackSource::YtDlpFallback(m) => m.load(identifier).await,
			TrackSource::Plugin(m) => m.load(identifier).await,
		}
	}

	pub async fn rich_search(&self, query: &str) -> Result<RichResults> {
		match self {
			TrackSource::YtDlp(m) => m.rich_search(query).await,
			TrackSource::SoundCloud(m) => m.rich_search(query).await,
			TrackSource::Spotify(m) => m.rich_search(query).await,
			TrackSource::Bandcamp(m) => m.rich_search(query).await,
			TrackSource::Playlist(m) => m.rich_search(query).await,
			TrackSource::Local(m) => m.rich_search(query).await,
			TrackSource::Http(m) => m.rich_search(query).await,
			TrackSource::Twitch(m) => m.rich_search(query).await,
			TrackSource::YtDlpFallback(m) => m.rich_search(query).await,
			TrackSource::Plugin(m) => m.rich_search(query).await,
		}
	}
}

fn url_host(uri: &str) -> Option<String> {
	let authority = uri.split_once("://")?.1;
	let end = authority.find(['/', '?', '#']).unwrap_or(authority.len());
	let mut host = &authority[..end];

	if let Some(pos) = host.rfind('@') {
		host = &host[pos + 1..];
	}

	if host.starts_with('[') {
		host = host.split(']').next().unwrap_or(host).trim_start_matches('[');
	} else if let Some(pos) = host.find(':') {
		host = &host[..pos];
	}

	if host.is_empty() {
		return None;
	}

	Some(host.to_ascii_lowercase())
}

pub fn needs_stream_resolve(uri: &str) -> bool {
	if uri.is_empty() {
		return false;
	}

	let host = url_host(uri);

	match host {
		Some(host) => {
			if host.contains("googlevideo.") {
				return false;
			}

			host == "youtu.be" || host.ends_with(".youtu.be") || host == "youtube.com" || host.ends_with(".youtube.com")
		}
		None => {
			let lower = uri.to_ascii_lowercase();

			if lower.contains("googlevideo.") {
				return false;
			}

			lower.contains("youtube.com") || lower.contains("youtu.be")
		}
	}
}

const STREAM_EXPIRY_MARGIN_SECS: u64 = 300;

fn stream_expiry_unix(uri: &str) -> Option<u64> {
	let after_query = uri.split_once('?')?.1;
	let query = after_query.split('#').next().unwrap_or(after_query);

	for pair in query.split('&') {
		let (key, value) = pair.split_once('=')?;

		if (key.eq_ignore_ascii_case("expire") || key.eq_ignore_ascii_case("expires"))
			&& let Ok(expires) = value.parse::<u64>()
		{
			return Some(expires);
		}
	}

	None
}

pub fn baked_stream_is_expired(uri: &str, now_unix: u64) -> bool {
	match stream_expiry_unix(uri) {
		Some(expires) => now_unix + STREAM_EXPIRY_MARGIN_SECS >= expires,
		None => false,
	}
}

fn now_unix() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

fn select_playback_uri(baked: &str, loaded: &Result<LoadResult>) -> String {
	match loaded {
		Ok(result) => {
			let fresh = result
				.tracks
				.first()
				.and_then(|track| track.info.uri.clone())
				.filter(|uri| !uri.is_empty());

			fresh.unwrap_or_else(|| baked.to_string())
		}
		Err(_) => baked.to_string(),
	}
}

pub struct TrackResolver {
	sources: Vec<TrackSource>,
	plugin_source: Option<TrackSource>,
	settings: RwLock<ResolverSettings>,
}

impl TrackResolver {
	pub async fn new() -> Self {
		let client = libytdlp::Client::new("{}").expect("Failed to initialize libytdlp client");

		Self::from_client(Arc::new(client))
	}

	pub fn from_client(client: Arc<libytdlp::Client>) -> Self {
		let sources = vec![
			TrackSource::Twitch(TwitchSourceManager::new(client.clone())),
			TrackSource::Spotify(SpotifySourceManager::new()),
			TrackSource::YtDlp(YtDlpSourceManager::new(client.clone())),
			TrackSource::SoundCloud(SoundCloudSourceManager::new()),
			TrackSource::Bandcamp(BandcampSourceManager::new()),
			TrackSource::Playlist(PlaylistSourceManager::new(client.clone())),
			TrackSource::Local(LocalFileSourceManager::new()),
			TrackSource::YtDlpFallback(YtDlpFallbackSourceManager::new(client.clone())),
			TrackSource::Http(HttpSourceManager::new()),
		];

		let source_order = sources.iter().map(|source| source.name().to_string()).collect();

		Self {
			sources,
			plugin_source: None,
			settings: RwLock::new(ResolverSettings {
				source_order,
				default_timeout_ms: 30_000,
				source_timeouts_ms: BTreeMap::new(),
			}),
		}
	}

	pub fn configurable_source_names(&self) -> Vec<String> {
		self.sources.iter().map(|source| source.name().to_string()).collect()
	}

	pub fn configure_runtime(
		&self,
		source_order: Vec<String>,
		default_timeout_ms: u64,
		source_timeouts_ms: BTreeMap<String, u64>,
	) -> std::result::Result<(), String> {
		if source_order.is_empty() {
			return Err("at least one resolver source must remain enabled".to_string());
		}
		if default_timeout_ms == 0 {
			return Err("default resolver timeout must be positive".to_string());
		}

		let available: HashSet<_> = self.sources.iter().map(|source| source.name()).collect();
		let mut configured = HashSet::new();
		for source in &source_order {
			if !available.contains(source.as_str()) {
				return Err(format!("unknown resolver source: {source}"));
			}
			if !configured.insert(source.as_str()) {
				return Err(format!("duplicate resolver source: {source}"));
			}
		}
		for (source, timeout_ms) in &source_timeouts_ms {
			if !available.contains(source.as_str()) {
				return Err(format!("unknown resolver source timeout: {source}"));
			}
			if *timeout_ms == 0 {
				return Err(format!("resolver timeout for {source} must be positive"));
			}
		}

		*self.settings.write() = ResolverSettings {
			source_order,
			default_timeout_ms,
			source_timeouts_ms,
		};

		Ok(())
	}

	pub fn register_plugin_source(&mut self, manager: Arc<tokio::sync::RwLock<orpheus_plugins::PluginManager>>) {
		self.plugin_source = Some(TrackSource::Plugin(PluginSourceManager::new(manager)));
	}

	pub async fn ffmpeg_path(&self) -> String {
		libytdlp::runtime_info()
			.ok()
			.and_then(|info| info.ffmpeg().map(|path| path.to_string()))
			.filter(|path| !path.is_empty())
			.unwrap_or_else(|| "ffmpeg".to_string())
	}

	pub async fn load(&self, identifier: &str) -> Result<(LoadResult, &'static str)> {
		// NOTE: plugin prefixes are checked before built-ins because YtDlp claims every
		// bare query; a postfix position would shadow all plugin prefixes. The check is
		// strictly prefix-gated, so it never diverts identifiers built-ins would handle.
		if let Some(plugin) = &self.plugin_source
			&& plugin.can_handle(identifier)
		{
			let mut result = plugin.load(identifier).await?;
			for track in &mut result.tracks {
				track.encoded = Some(encode_track(track, &[])?);
			}
			return Ok((result, plugin.name()));
		}

		let settings = self.settings.read().clone();

		for source_name in &settings.source_order {
			let Some(source) = self.sources.iter().find(|source| source.name() == source_name) else {
				continue;
			};
			if source.can_handle(identifier) {
				let source_name = source.name();
				let timeout_ms = settings
					.source_timeouts_ms
					.get(source_name)
					.copied()
					.unwrap_or(settings.default_timeout_ms);
				let mut result = tokio::time::timeout(Duration::from_millis(timeout_ms), source.load(identifier))
					.await
					.map_err(|_| ResolverError::Timeout(source_name.to_string()))??;
				for track in &mut result.tracks {
					track.encoded = Some(encode_track(track, &[])?);
				}
				return Ok((result, source_name));
			}
		}

		Err(ResolverError::UnsupportedSource(format!(
			"No enabled source claimed {identifier:?}"
		)))
	}

	pub async fn rich_search(&self, query: &str) -> RichResults {
		let query = query.trim();

		if query.is_empty() {
			return RichResults::empty();
		}

		let mut searches = Vec::new();

		// NOTE: the boxed future erases the per-source future types so the fan-in
		// below stays a single join over a homogeneous list.
		type BoxedSearch<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = RichResults> + Send + 'a>>;

		let settings = self.settings.read().clone();

		for source_name in &settings.source_order {
			let Some(source) = self.sources.iter().find(|source| source.name() == source_name) else {
				continue;
			};
			let query = query.to_string();
			let source_name = source.name();
			let timeout_ms = settings
				.source_timeouts_ms
				.get(source_name)
				.copied()
				.unwrap_or(settings.default_timeout_ms);

			let search: BoxedSearch<'_> = Box::pin(async move {
				match tokio::time::timeout(Duration::from_millis(timeout_ms), source.rich_search(&query)).await {
					Ok(Ok(results)) => results,
					Ok(Err(error)) => {
						tracing::warn!("Rich search on {source_name} failed: {error}");

						RichResults::empty()
					}
					Err(_) => {
						tracing::warn!("Rich search on {source_name} timed out");

						RichResults::empty()
					}
				}
			});

			searches.push(search);
		}

		let mut merged = RichResults::merge(futures_util::future::join_all(searches).await);

		for (position, track) in merged.tracks.iter_mut().enumerate() {
			track.info.position = position as u64;

			if let Ok(encoded) = encode_track(track, &[]) {
				track.encoded = Some(encoded);
			}
		}

		merged
	}

	pub async fn playback_url(&self, track: &Track) -> String {
		let baked = track.info.uri.clone().unwrap_or_default();

		if is_spotify_url(&track.info.identifier) {
			return self.playback_url_spotify(track, &baked).await;
		}

		if track.info.source_name != "youtube" {
			return baked;
		}

		// NOTE: watch-page URLs always re-resolve, and baked stream URLs re-resolve once
		// their `expire` timestamp passes (with margin); anything else plays as baked.
		if !needs_stream_resolve(&baked) && !baked_stream_is_expired(&baked, now_unix()) {
			return baked;
		}

		let loaded = self.load(&track.info.identifier).await.map(|(result, _)| result);

		let playback = select_playback_uri(&baked, &loaded);

		if playback == baked {
			if loaded.is_err() {
				tracing::warn!(
					"Playback re-resolve failed for {:?}, falling back to baked uri",
					track.info.identifier
				);
			} else {
				tracing::warn!(
					"Playback re-resolve for {:?} returned no stream URL, falling back to baked uri",
					track.info.identifier
				);
			}
		}

		playback
	}

	async fn playback_url_spotify(&self, track: &Track, baked: &str) -> String {
		let query = mirror_query(&track.info.title, &track.info.author);

		let mirrored = if query.trim().is_empty() {
			Err(ResolverError::InvalidIdentifier("empty Spotify track metadata".to_string()))
		} else {
			self.load(&query).await.map(|(result, _)| result)
		};

		let playback = select_playback_uri(baked, &mirrored);

		if playback == baked {
			tracing::warn!(
				"Spotify mirror failed for {:?}, falling back to baked uri",
				track.info.identifier
			);
		}

		playback
	}

	pub fn source_names(&self) -> Vec<String> {
		let mut names = self.settings.read().source_order.clone();

		if let Some(plugin) = &self.plugin_source {
			names.push(plugin.name().to_string());
		}

		names
	}
}

#[cfg(test)]
mod tests;
