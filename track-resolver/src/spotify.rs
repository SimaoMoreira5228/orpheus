use std::time::{Duration, Instant};

use tokio::sync::Mutex;

mod identifier;
mod mapping;

use identifier::{is_spotify_search, parse_spotify_id, url_encode};
pub(crate) use identifier::{is_spotify_url, mirror_query};
use mapping::track_info_from_json;

use crate::{LoadResult, LoadType, PlaylistInfo, ResolverError, Result, RichResults, SourceManager, Track, new_track};

const SPOTIFY_TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const SPOTIFY_API_BASE: &str = "https://api.spotify.com/v1";
const USER_AGENT: &str =
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.6998.178 Safari/537.36";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const TOKEN_REFRESH_MARGIN_SECS: u64 = 60;
const PAGE_LIMIT: u32 = 100;
const MAX_TRACKS: usize = 200;

pub struct SpotifySourceManager {
	client_id: Option<String>,
	client_secret: Option<String>,
	http_client: reqwest::Client,
	token: Mutex<Option<(String, Instant)>>,
}

impl Default for SpotifySourceManager {
	fn default() -> Self {
		Self::new()
	}
}

impl SpotifySourceManager {
	pub fn new() -> Self {
		Self::with_credentials(
			std::env::var("SPOTIFY_CLIENT_ID").ok(),
			std::env::var("SPOTIFY_CLIENT_SECRET").ok(),
		)
	}

	pub fn with_credentials(client_id: Option<String>, client_secret: Option<String>) -> Self {
		Self {
			client_id,
			client_secret,
			http_client: reqwest::Client::builder()
				.user_agent(USER_AGENT)
				.timeout(REQUEST_TIMEOUT)
				.build()
				.expect("Spotify HTTP client builds"),
			token: Mutex::new(None),
		}
	}

	fn credentials(&self) -> Result<(String, String)> {
		match (&self.client_id, &self.client_secret) {
			(Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => Ok((id.clone(), secret.clone())),
			_ => Err(ResolverError::FetchError(
				"Spotify credentials not configured (set SPOTIFY_CLIENT_ID and SPOTIFY_CLIENT_SECRET)".to_string(),
			)),
		}
	}

	async fn get_token(&self) -> Result<String> {
		{
			let cached = self.token.lock().await;

			if let Some((token, expires_at)) = cached.as_ref()
				&& Instant::now() + Duration::from_secs(TOKEN_REFRESH_MARGIN_SECS) < *expires_at
			{
				return Ok(token.clone());
			}
		}

		self.refresh_token().await
	}

	async fn refresh_token(&self) -> Result<String> {
		let (client_id, client_secret) = self.credentials()?;

		let credentials = base64::Engine::encode(
			&base64::engine::general_purpose::STANDARD,
			format!("{client_id}:{client_secret}"),
		);

		let response = self
			.http_client
			.post(SPOTIFY_TOKEN_URL)
			.header("Authorization", format!("Basic {credentials}"))
			.header("Content-Type", "application/x-www-form-urlencoded")
			.body("grant_type=client_credentials".to_string())
			.send()
			.await
			.map_err(|e| ResolverError::FetchError(format!("Spotify token request failed: {e}")))?;

		if !response.status().is_success() {
			return Err(ResolverError::FetchError(format!(
				"Spotify token request failed: HTTP {}",
				response.status()
			)));
		}

		let json: serde_json::Value = response
			.json()
			.await
			.map_err(|e| ResolverError::FetchError(format!("Failed to parse Spotify token response: {e}")))?;

		let token = json
			.get("access_token")
			.and_then(|v| v.as_str())
			.ok_or_else(|| ResolverError::FetchError("Spotify token response missing access_token".to_string()))?
			.to_string();

		let expires_in = json.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600);
		let expires_at = Instant::now() + Duration::from_secs(expires_in);

		*self.token.lock().await = Some((token.clone(), expires_at));

		Ok(token)
	}

	async fn api_get(&self, path: &str, query: &[(&str, &str)]) -> Result<serde_json::Value> {
		let token = self.get_token().await?;
		let mut url = format!("{SPOTIFY_API_BASE}{path}");

		if !query.is_empty() {
			url.push('?');
			url.push_str(
				&query
					.iter()
					.map(|(key, value)| format!("{key}={}", url_encode(value)))
					.collect::<Vec<_>>()
					.join("&"),
			);
		}

		let send = |token: &str| self.http_client.get(&url).bearer_auth(token).send();

		let response = send(&token)
			.await
			.map_err(|e| ResolverError::FetchError(format!("Spotify API request failed: {e}")))?;

		let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
			*self.token.lock().await = None;

			send(&self.refresh_token().await?)
				.await
				.map_err(|e| ResolverError::FetchError(format!("Spotify API request failed: {e}")))?
		} else {
			response
		};

		if !response.status().is_success() {
			return Err(ResolverError::FetchError(format!(
				"Spotify API request failed: HTTP {}",
				response.status()
			)));
		}

		response
			.json()
			.await
			.map_err(|e| ResolverError::FetchError(format!("Failed to parse Spotify API response: {e}")))
	}

	fn new_track(&self, kind: &str, id: &str, value: &serde_json::Value, position: usize) -> Option<Track> {
		let mut info = track_info_from_json(kind, id, value)?;
		info.position = position as u64;

		Some(new_track(info))
	}

	async fn resolve_track(&self, id: &str) -> Result<LoadResult> {
		self.credentials()?;
		let json = self.api_get(&format!("/tracks/{id}"), &[]).await?;

		let track = self
			.new_track("track", id, &json, 0)
			.ok_or_else(|| ResolverError::ParseError("Failed to parse Spotify track".to_string()))?;

		Ok(LoadResult {
			load_type: LoadType::Track,
			tracks: vec![track],
			playlist_info: None,
		})
	}

	async fn collect_paged_tracks(&self, url: &str, tracks: &mut Vec<Track>) -> Result<()> {
		let mut next: Option<String> = Some(url.to_string());

		while let Some(page_url) = next.take() {
			if tracks.len() >= MAX_TRACKS {
				break;
			}

			let token = self.get_token().await?;

			let response = self
				.http_client
				.get(&page_url)
				.bearer_auth(&token)
				.send()
				.await
				.map_err(|e| ResolverError::FetchError(format!("Spotify API request failed: {e}")))?;

			if !response.status().is_success() {
				return Err(ResolverError::FetchError(format!(
					"Spotify API request failed: HTTP {}",
					response.status()
				)));
			}

			let json: serde_json::Value = response
				.json()
				.await
				.map_err(|e| ResolverError::FetchError(format!("Failed to parse Spotify API response: {e}")))?;

			let items = json
				.get("items")
				.and_then(|items| items.as_array())
				.cloned()
				.unwrap_or_default();

			for item in items {
				if tracks.len() >= MAX_TRACKS {
					break;
				}

				let value = item.get("track").unwrap_or(&item);

				if value.get("type").and_then(|v| v.as_str()) == Some("episode") {
					continue;
				}

				let Some(id) = value.get("id").and_then(|v| v.as_str()) else {
					continue;
				};

				if let Some(track) = self.new_track("track", id, value, tracks.len()) {
					tracks.push(track);
				}
			}

			next = json
				.get("next")
				.and_then(|v| v.as_str())
				.filter(|next| !next.is_empty())
				.map(|next| next.to_string());
		}

		Ok(())
	}

	async fn resolve_album(&self, id: &str) -> Result<LoadResult> {
		self.credentials()?;
		let json = self.api_get(&format!("/albums/{id}"), &[]).await?;

		let name = json.get("name").and_then(|v| v.as_str()).map(|v| v.to_string());
		let mut tracks = Vec::new();

		self.collect_paged_tracks(
			&format!("{SPOTIFY_API_BASE}/albums/{id}/tracks?limit={PAGE_LIMIT}"),
			&mut tracks,
		)
		.await?;

		if tracks.is_empty() {
			return Err(ResolverError::NotFound("No tracks found in Spotify album".to_string()));
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

	async fn resolve_playlist(&self, id: &str) -> Result<LoadResult> {
		self.credentials()?;
		let json = self.api_get(&format!("/playlists/{id}"), &[]).await?;

		let name = json.get("name").and_then(|v| v.as_str()).map(|v| v.to_string());
		let mut tracks = Vec::new();

		self.collect_paged_tracks(
			&format!("{SPOTIFY_API_BASE}/playlists/{id}/tracks?limit={PAGE_LIMIT}"),
			&mut tracks,
		)
		.await?;

		if tracks.is_empty() {
			return Err(ResolverError::NotFound("No tracks found in Spotify playlist".to_string()));
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

	async fn resolve_artist(&self, id: &str) -> Result<LoadResult> {
		self.credentials()?;
		let json = self
			.api_get(&format!("/artists/{id}/top-tracks"), &[("market", "US")])
			.await?;

		let tracks: Vec<Track> = json
			.get("tracks")
			.and_then(|v| v.as_array())
			.cloned()
			.unwrap_or_default()
			.into_iter()
			.enumerate()
			.filter_map(|(i, value)| {
				let track_id = value.get("id")?.as_str()?;
				self.new_track("track", track_id, &value, i)
			})
			.collect();

		if tracks.is_empty() {
			return Err(ResolverError::NotFound("No top tracks found for Spotify artist".to_string()));
		}

		Ok(LoadResult {
			load_type: LoadType::Search,
			tracks,
			playlist_info: None,
		})
	}

	async fn search(&self, query: &str) -> Result<LoadResult> {
		self.credentials()?;
		let search_query = query.trim_start_matches("spsearch:");

		let json = self
			.api_get("/search", &[("q", search_query), ("type", "track"), ("limit", "10")])
			.await?;

		let tracks = self.search_track_list(&json);

		if tracks.is_empty() {
			return Err(ResolverError::NotFound("No Spotify tracks found".to_string()));
		}

		Ok(LoadResult {
			load_type: LoadType::Search,
			tracks,
			playlist_info: None,
		})
	}

	fn search_track_list(&self, json: &serde_json::Value) -> Vec<Track> {
		json.get("tracks")
			.and_then(|v| v.get("items"))
			.and_then(|v| v.as_array())
			.cloned()
			.unwrap_or_default()
			.into_iter()
			.enumerate()
			.filter_map(|(i, value)| {
				let track_id = value.get("id")?.as_str()?;
				self.new_track("track", track_id, &value, i)
			})
			.collect()
	}

	pub(crate) async fn rich_search(&self, query: &str) -> Result<RichResults> {
		self.credentials()?;

		let search_query = query.trim_start_matches("spsearch:");

		let json = self
			.api_get(
				"/search",
				&[("q", search_query), ("type", "album,artist,playlist,track"), ("limit", "10")],
			)
			.await?;

		Ok(mapping::rich_search_results(&json, self.search_track_list(&json)))
	}
}

impl SourceManager for SpotifySourceManager {
	fn name(&self) -> &'static str {
		"spotify"
	}

	fn can_handle(&self, identifier: &str) -> bool {
		is_spotify_url(identifier) || is_spotify_search(identifier)
	}

	async fn load(&self, identifier: &str) -> Result<LoadResult> {
		if is_spotify_search(identifier) {
			return self.search(identifier).await;
		}

		let (kind, id) = parse_spotify_id(identifier)
			.ok_or_else(|| ResolverError::InvalidIdentifier(format!("Not a Spotify URL: {identifier}")))?;

		match kind.as_str() {
			"track" => self.resolve_track(&id).await,
			"album" => self.resolve_album(&id).await,
			"playlist" => self.resolve_playlist(&id).await,
			"artist" => self.resolve_artist(&id).await,
			_ => Err(ResolverError::UnsupportedSource(format!(
				"Unsupported Spotify URL type: {kind}"
			))),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::TrackSource;

	fn manager_without_creds() -> SpotifySourceManager {
		SpotifySourceManager::with_credentials(None, None)
	}

	fn track_json() -> serde_json::Value {
		serde_json::json!({
			"id": "4cOdK2wGLETKBW3PvgPWqTx",
			"name": "Never Gonna Give You Up",
			"artists": [{"name": "Rick Astley"}],
			"album": {"name": "Whenever You Need Somebody", "images": [{"url": "https://example.com/cover.jpg"}]},
			"duration_ms": 213000,
			"external_urls": {"spotify": "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqTx"},
			"external_ids": {"isrc": "GBARL0600789"}
		})
	}

	#[test]
	fn recognizes_spotify_urls() {
		let manager = manager_without_creds();

		assert!(manager.can_handle("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqTx"));
		assert!(manager.can_handle("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqTx?si=abc"));
		assert!(manager.can_handle("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M"));
		assert!(manager.can_handle("https://open.spotify.com/album/1uL4q2p8rW5NJ4vYt0WeuS"));
		assert!(manager.can_handle("https://open.spotify.com/artist/0gxyHSt3pucJWEgTPpzkwD"));
		assert!(manager.can_handle("spotify:track:4cOdK2wGLETKBW3PvgPWqTx"));
		assert!(manager.can_handle("spsearch:never gonna give you up"));
	}

	#[test]
	fn rejects_non_spotify_identifiers() {
		let manager = manager_without_creds();

		assert!(!manager.can_handle("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
		assert!(!manager.can_handle("https://open.spotify.com.evil.example/track/abc"));
		assert!(!manager.can_handle("https://example.com/open.spotify.com/track/abc"));
		assert!(!manager.can_handle("never gonna give you up"));
		assert!(!manager.can_handle("https://open.spotify.com/"));
		assert!(!manager.can_handle(""));
	}

	#[test]
	fn parses_track_album_playlist_and_artist_ids() {
		assert_eq!(
			parse_spotify_id("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqTx"),
			Some(("track".to_string(), "4cOdK2wGLETKBW3PvgPWqTx".to_string()))
		);
		assert_eq!(
			parse_spotify_id("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc"),
			Some(("playlist".to_string(), "37i9dQZF1DXcBWIGoYBM5M".to_string()))
		);
		assert_eq!(
			parse_spotify_id("spotify:album:6JWc4iAiJ9ISuwN9pJ9o0v"),
			Some(("album".to_string(), "6JWc4iAiJ9ISuwN9pJ9o0v".to_string()))
		);
		assert_eq!(
			parse_spotify_id("https://open.spotify.com/intl-de/track/4cOdK2wGLETKBW3PvgPWqTx"),
			Some(("track".to_string(), "4cOdK2wGLETKBW3PvgPWqTx".to_string()))
		);
	}

	#[test]
	fn rejects_malformed_spotify_identifiers() {
		assert_eq!(parse_spotify_id("https://open.spotify.com/track/"), None);
		assert_eq!(parse_spotify_id("https://open.spotify.com/track"), None);
		assert_eq!(parse_spotify_id("https://open.spotify.com/"), None);
		assert_eq!(parse_spotify_id("https://example.com/track/abc"), None);
		assert_eq!(parse_spotify_id(""), None);
	}

	#[test]
	fn mirror_query_combines_artist_and_title() {
		assert_eq!(
			mirror_query("Never Gonna Give You Up", "Rick Astley"),
			"Rick Astley - Never Gonna Give You Up"
		);
		assert_eq!(mirror_query("Unknown", ""), "Unknown");
		assert_eq!(mirror_query("", "Someone"), "Someone");
		assert_eq!(mirror_query("", ""), "");
	}

	#[test]
	fn maps_track_json_to_track_info() {
		let info = track_info_from_json("track", "4cOdK2wGLETKBW3PvgPWqTx", &track_json()).unwrap();

		assert_eq!(info.identifier, "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqTx");
		assert_eq!(info.title, "Never Gonna Give You Up");
		assert_eq!(info.author, "Rick Astley");
		assert_eq!(info.length, 213000);
		assert_eq!(
			info.uri,
			Some("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqTx".to_string())
		);
		assert_eq!(info.artwork_url, Some("https://example.com/cover.jpg".to_string()));
		assert_eq!(info.isrc, Some("GBARL0600789".to_string()));
		assert_eq!(info.source_name, "spotify");
		assert!(info.is_seekable);
		assert!(!info.is_stream);
	}

	#[tokio::test]
	async fn missing_credentials_fail_with_actionable_error() {
		let manager = manager_without_creds();
		let err = manager
			.load("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqTx")
			.await
			.unwrap_err();

		assert!(err.to_string().contains("SPOTIFY_CLIENT_ID"));
	}

	fn multi_type_search_json() -> serde_json::Value {
		serde_json::json!({
			"tracks": {"items": [track_json()]},
			"albums": {"items": [{
				"id": "6JWc4iAiJ9ISuwN9pJ9o0v",
				"name": "Whenever You Need Somebody",
				"artists": [{"name": "Rick Astley"}],
				"images": [{"url": "https://example.com/album.jpg"}]
			}]},
			"artists": {"items": [{
				"id": "0gxyHSt3pucJWEgTPpzkwD",
				"name": "Rick Astley",
				"images": [{"url": "https://example.com/artist.jpg"}]
			}]},
			"playlists": {"items": [{
				"id": "37i9dQZF1DXcBWIGoYBM5M",
				"name": "80s Hits",
				"owner": {"display_name": "Spotify"},
				"images": [{"url": "https://example.com/playlist.jpg"}],
				"tracks": {"total": 50}
			}]}
		})
	}

	#[test]
	fn maps_multi_type_search_to_rich_results() {
		let manager = manager_without_creds();
		let json = multi_type_search_json();
		let results = mapping::rich_search_results(&json, manager.search_track_list(&json));

		assert_eq!(results.tracks.len(), 1);
		assert_eq!(results.tracks[0].info.title, "Never Gonna Give You Up");

		assert_eq!(results.albums.len(), 1);
		assert_eq!(results.albums[0].name, "Whenever You Need Somebody");
		assert_eq!(results.albums[0].artist, "Rick Astley");
		assert_eq!(results.albums[0].uri, "https://open.spotify.com/album/6JWc4iAiJ9ISuwN9pJ9o0v");

		assert_eq!(results.artists.len(), 1);
		assert_eq!(
			results.artists[0].uri,
			"https://open.spotify.com/artist/0gxyHSt3pucJWEgTPpzkwD"
		);

		assert_eq!(results.playlists.len(), 1);
		assert_eq!(results.playlists[0].track_count, 50);
		assert_eq!(
			results.playlists[0].uri,
			"https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M"
		);

		let empty_json = serde_json::json!({});
		let empty = mapping::rich_search_results(&empty_json, manager.search_track_list(&empty_json));

		assert!(empty.tracks.is_empty());
		assert!(empty.albums.is_empty());
		assert!(empty.artists.is_empty());
		assert!(empty.playlists.is_empty());
	}

	#[tokio::test]
	async fn rich_search_without_credentials_fails() {
		let manager = manager_without_creds();
		let direct = manager.rich_search("never gonna give you up").await;

		assert!(direct.is_err());

		let via_dispatch = TrackSource::Spotify(manager_without_creds())
			.rich_search("never gonna give you up")
			.await;

		assert!(via_dispatch.is_err());
	}

	#[tokio::test]
	#[ignore = "hits the live Spotify API; needs SPOTIFY_CLIENT_ID/SECRET in env"]
	async fn live_search_track_and_album() {
		for var in ["SPOTIFY_CLIENT_ID", "SPOTIFY_CLIENT_SECRET"] {
			assert!(std::env::var(var).is_ok(), "live test needs {var} in env");
		}

		let manager = SpotifySourceManager::new();
		let query = "Never Gonna Give You Up Rick Astley";

		let searched = manager.load(&format!("spsearch:{query}")).await.expect("live search works");

		assert!(!searched.tracks.is_empty());
		assert!(!searched.tracks[0].info.title.is_empty());

		let track_id = searched.tracks[0]
			.info
			.identifier
			.rsplit('/')
			.next()
			.expect("canonical track url carries the id");

		let resolved = manager
			.load(&format!("https://open.spotify.com/track/{track_id}"))
			.await
			.expect("live track resolves");

		assert_eq!(resolved.tracks.len(), 1);
		assert_eq!(resolved.tracks[0].info.title, searched.tracks[0].info.title);

		let raw = manager
			.api_get("/search", &[("q", query), ("type", "track"), ("limit", "1")])
			.await
			.expect("raw search works");

		let album_id = raw
			.pointer("/tracks/items/0/album/id")
			.and_then(|id| id.as_str())
			.expect("search carries the album id");

		let album = manager.resolve_album(album_id).await.expect("live album resolves");

		assert!(!album.tracks.is_empty());
	}
}
