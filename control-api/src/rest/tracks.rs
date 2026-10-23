use std::sync::Arc;

use actix_web::HttpRequest;
use actix_web::web::{Data, Json, Query};
use orpheus_core::{AlbumRef, ArtistRef, PlaylistRef, Track, TrackInfo};
use serde::{Deserialize, Deserializer, Serialize};

use crate::errors::{ErrorCode, RestError};
use crate::state::AppState;

mod batch;
pub use batch::{BatchTracksRequest, DryRunItem, MAX_BATCH_IDENTIFIERS, load_tracks_batch, load_tracks_dry_run};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
	pub name: Option<String>,
	pub selected_track: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoadResultType {
	Track,
	Playlist,
	Search,
	Empty,
	Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadFailure {
	pub message: String,
	pub severity: crate::websocket_messages::ExceptionSeverity,
	pub code: ErrorCode,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum LoadTracksResponseData {
	Track(Box<Track>),
	Playlist(PlaylistTracksData),
	Search(Vec<Track>),
	Error(LoadFailure),
}

#[derive(Debug, Serialize)]
pub struct PlaylistTracksData {
	pub info: PlaylistInfo,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub plugin_info: Option<serde_json::Value>,
	pub tracks: Vec<Track>,
}

#[derive(Debug, Serialize)]
pub struct LoadTracksResponse {
	#[serde(rename = "loadType")]
	pub load_type: LoadResultType,
	pub data: LoadTracksResponseData,
}

pub(crate) fn nullable_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
	D: Deserializer<'de>,
	T: Deserialize<'de>,
{
	Ok(Some(Option::<T>::deserialize(deserializer)?))
}

#[derive(Debug, Deserialize)]
pub struct LoadTracksQuery {
	identifier: String,
}

#[derive(Debug, Deserialize)]
pub struct RichSearchQuery {
	query: String,
}

#[derive(Debug, Serialize)]
pub struct RichSearchResponse {
	pub tracks: Vec<Track>,
	pub albums: Vec<AlbumRef>,
	pub artists: Vec<ArtistRef>,
	pub playlists: Vec<PlaylistRef>,
}

#[derive(Debug, Deserialize)]
pub struct DecodeTrackQuery {
	#[serde(default, rename = "encodedTrack")]
	encoded_track: Option<String>,
	#[serde(default, rename = "track")]
	legacy_track: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DecodeTracksRequest {
	pub tracks: Vec<String>,
}

pub async fn load_tracks(query: Query<LoadTracksQuery>, state: Data<Arc<AppState>>) -> Json<LoadTracksResponse> {
	let identifier = &query.identifier;

	let result = match state.load_tracks(identifier).await {
		Ok((result, _)) => result,
		Err(error) => {
			tracing::warn!("Failed to load tracks for {:?}: {}", identifier, error);

			return Json(error_response_for(&error));
		}
	};

	Json(response_for_result(result))
}

fn map_track(t: orpheus_track_resolver::Track) -> Track {
	Track {
		encoded: t.encoded,
		info: TrackInfo {
			identifier: t.info.identifier,
			is_seekable: t.info.is_seekable,
			author: t.info.author,
			length: t.info.length,
			is_stream: t.info.is_stream,
			position: t.info.position,
			title: t.info.title,
			uri: t.info.uri,
			artwork_url: t.info.artwork_url,
			isrc: t.info.isrc,
			source_name: t.info.source_name,
		},
		start_time: t.start_time,
		end_time: t.end_time,
		plugin_info: serde_json::json!({}),
		user_data: serde_json::json!({}),
	}
}

pub(super) fn error_response_for(error: &orpheus_track_resolver::ResolverError) -> LoadTracksResponse {
	LoadTracksResponse {
		load_type: LoadResultType::Error,
		data: LoadTracksResponseData::Error(load_failure_for(error)),
	}
}

pub(super) fn response_for_result(result: orpheus_track_resolver::LoadResult) -> LoadTracksResponse {
	let load_type = match result.load_type {
		orpheus_track_resolver::LoadType::Track => LoadResultType::Track,
		orpheus_track_resolver::LoadType::Playlist => LoadResultType::Playlist,
		orpheus_track_resolver::LoadType::Search => LoadResultType::Search,
		orpheus_track_resolver::LoadType::Empty => LoadResultType::Empty,
		orpheus_track_resolver::LoadType::Error => LoadResultType::Error,
	};

	let tracks: Vec<Track> = result.tracks.into_iter().map(map_track).collect();

	let data = match load_type {
		LoadResultType::Track => {
			if let Some(track) = tracks.into_iter().next() {
				LoadTracksResponseData::Track(Box::new(track))
			} else {
				LoadTracksResponseData::Search(vec![])
			}
		}
		LoadResultType::Playlist => {
			let playlist_info = result
				.playlist_info
				.map(|p| PlaylistInfo {
					name: p.name,
					selected_track: p.selected_track,
				})
				.unwrap_or(PlaylistInfo {
					name: None,
					selected_track: None,
				});
			LoadTracksResponseData::Playlist(PlaylistTracksData {
				info: playlist_info,
				plugin_info: Some(serde_json::json!({})),
				tracks,
			})
		}
		LoadResultType::Search => LoadTracksResponseData::Search(tracks),
		LoadResultType::Empty => LoadTracksResponseData::Search(vec![]),
		LoadResultType::Error => LoadTracksResponseData::Search(vec![]),
	};

	LoadTracksResponse { load_type, data }
}

pub async fn load_search(query: Query<RichSearchQuery>, state: Data<Arc<AppState>>) -> Json<RichSearchResponse> {
	let results = state.track_resolver.rich_search(&query.query).await;

	Json(RichSearchResponse {
		tracks: results.tracks.into_iter().map(map_track).collect(),
		albums: results.albums,
		artists: results.artists,
		playlists: results.playlists,
	})
}

pub async fn decode_track(req: HttpRequest, query: Query<DecodeTrackQuery>) -> Result<Json<Track>, RestError> {
	let target = resolve_decode_target(&query).map_err(|(code, message)| RestError::new(code, Some(message), req.path()))?;

	let decoded = orpheus_track_resolver::decode_track(target)
		.map_err(|e| RestError::bad_request(ErrorCode::UndecodableTrack, e.to_string(), &req))?;

	Ok(Json(decoded.track))
}

pub async fn decode_tracks(req: HttpRequest, request: Json<DecodeTracksRequest>) -> Result<Json<Vec<Track>>, RestError> {
	let request = request.into_inner();

	if request.tracks.is_empty() {
		return Err(RestError::bad_request(
			ErrorCode::EmptyTrackList,
			"No tracks to decode provided",
			&req,
		));
	}

	let mut tracks = Vec::with_capacity(request.tracks.len());

	for encoded in &request.tracks {
		let decoded = orpheus_track_resolver::decode_track(encoded)
			.map_err(|e| RestError::bad_request(ErrorCode::UndecodableTrack, e.to_string(), &req))?;

		tracks.push(decoded.track);
	}

	Ok(Json(tracks))
}

pub(crate) fn merge_no_replace(query: Option<bool>, body: Option<bool>) -> bool {
	query.or(body).unwrap_or(false)
}

pub(crate) fn resolve_decode_target(query: &DecodeTrackQuery) -> Result<&str, (ErrorCode, String)> {
	match query.encoded_track.as_deref().or(query.legacy_track.as_deref()) {
		Some(target) => Ok(target),
		None => Err((ErrorCode::MissingTrack, "No track to decode provided".to_string())),
	}
}

pub(crate) fn ensure_playable(load_type: &orpheus_track_resolver::LoadType) -> Result<(), (ErrorCode, String)> {
	match load_type {
		orpheus_track_resolver::LoadType::Playlist => Err((
			ErrorCode::PlaylistNotPlayable,
			"Cannot play a playlist or search result".to_string(),
		)),
		_ => Ok(()),
	}
}

pub(super) fn load_failure_for(error: &orpheus_track_resolver::ResolverError) -> LoadFailure {
	use crate::errors::code_for_resolver;
	use crate::websocket_messages::ExceptionSeverity as Severity;

	let code = code_for_resolver(error);
	let message = error.to_string();

	let severity = match code {
		ErrorCode::NoMatches | ErrorCode::InvalidIdentifier | ErrorCode::UnsupportedSource | ErrorCode::NoAudioStream => {
			Severity::Common
		}
		_ => Severity::Fault,
	};

	LoadFailure { message, severity, code }
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use actix_web::http::StatusCode;
	use actix_web::test::{TestRequest, call_service, init_service, read_body};
	use actix_web::web::Data;
	use actix_web::{ResponseError, web};

	use super::*;
	use crate::errors::{json_error_handler, query_error_handler};
	use crate::rest::players::NoReplaceQuery;
	use crate::state::AppState;

	async fn batch_state() -> Data<Arc<AppState>> {
		let track_resolver = Arc::new(orpheus_track_resolver::TrackResolver::new().await);
		let plugin_manager = Arc::new(tokio::sync::RwLock::new(
			orpheus_plugins::PluginManager::new()
				.await
				.expect("manager builds without wasm"),
		));
		let runtime_config = Arc::new(
			crate::runtime_config::RuntimeConfigStore::in_memory(
				crate::runtime_config::RuntimeConfig::default(),
				track_resolver.configurable_source_names(),
			)
			.unwrap(),
		);

		Data::new(Arc::new(AppState::new(track_resolver, plugin_manager, runtime_config)))
	}

	fn get_request(path: &str) -> HttpRequest {
		TestRequest::get().uri(path).to_http_request()
	}

	#[test]
	fn decode_target_prefers_encoded_track_and_requires_one() {
		let missing = DecodeTrackQuery {
			encoded_track: None,
			legacy_track: None,
		};

		assert!(resolve_decode_target(&missing).is_err());

		let legacy = DecodeTrackQuery {
			encoded_track: None,
			legacy_track: Some("abc".to_string()),
		};

		assert_eq!(resolve_decode_target(&legacy).unwrap(), "abc");

		let both = DecodeTrackQuery {
			encoded_track: Some("primary".to_string()),
			legacy_track: Some("abc".to_string()),
		};

		assert_eq!(resolve_decode_target(&both).unwrap(), "primary");
	}

	#[test]
	fn query_structs_ignore_unknown_params() {
		let load: LoadTracksQuery =
			serde_json::from_value(serde_json::json!({"identifier": "x", "trace": true, "extra": 1})).unwrap();

		assert_eq!(load.identifier, "x");

		let no_replace: NoReplaceQuery =
			serde_json::from_value(serde_json::json!({"noReplace": true, "trace": true})).unwrap();

		assert_eq!(no_replace.no_replace, Some(true));
	}

	#[test]
	fn resolver_errors_map_to_stable_codes_and_severities() {
		use orpheus_track_resolver::ResolverError;

		use crate::websocket_messages::ExceptionSeverity;

		let cases: Vec<(ResolverError, ErrorCode, ExceptionSeverity)> = vec![
			(
				ResolverError::NotFound("gone".to_string()),
				ErrorCode::NoMatches,
				ExceptionSeverity::Common,
			),
			(
				ResolverError::InvalidIdentifier("bogus".to_string()),
				ErrorCode::InvalidIdentifier,
				ExceptionSeverity::Common,
			),
			(
				ResolverError::UnsupportedSource("n/a".to_string()),
				ErrorCode::UnsupportedSource,
				ExceptionSeverity::Common,
			),
			(
				ResolverError::NoAudioStream,
				ErrorCode::NoAudioStream,
				ExceptionSeverity::Common,
			),
			(
				ResolverError::ParseError("truncated".to_string()),
				ErrorCode::ResponseParseFailed,
				ExceptionSeverity::Fault,
			),
			(
				ResolverError::IoError("denied".to_string()),
				ErrorCode::FetchFailed,
				ExceptionSeverity::Fault,
			),
			(
				ResolverError::Codec(orpheus_track_resolver::CodecError::EmptyMessage),
				ErrorCode::UndecodableTrack,
				ExceptionSeverity::Fault,
			),
			(
				ResolverError::FetchError("YouTube resolve failed: HTTP 429 Too Many Requests".to_string()),
				ErrorCode::RateLimited,
				ExceptionSeverity::Fault,
			),
			(
				ResolverError::FetchError("Spotify token request failed: HTTP 401 Unauthorized".to_string()),
				ErrorCode::SourceAuthRequired,
				ExceptionSeverity::Fault,
			),
			(
				ResolverError::FetchError("Spotify API request failed: HTTP 403 Forbidden".to_string()),
				ErrorCode::SourceAccessForbidden,
				ExceptionSeverity::Fault,
			),
			(
				ResolverError::FetchError("SoundCloud resolve failed: HTTP 404 Not Found".to_string()),
				ErrorCode::NoMatches,
				ExceptionSeverity::Common,
			),
			(
				ResolverError::FetchError("extract task failed: connection reset".to_string()),
				ErrorCode::SourceUnavailable,
				ExceptionSeverity::Fault,
			),
		];

		for (error, code, severity) in cases {
			let failure = load_failure_for(&error);

			assert_eq!(failure.code, code, "code for {error}");
			assert_eq!(failure.severity, severity, "severity for {error}");
			assert!(!failure.message.is_empty());
		}
	}

	#[test]
	fn playlist_identifier_is_rejected_as_track() {
		let result = ensure_playable(&orpheus_track_resolver::LoadType::Playlist);

		assert_eq!(
			result,
			Err((
				ErrorCode::PlaylistNotPlayable,
				"Cannot play a playlist or search result".to_string()
			))
		);

		for load_type in [
			orpheus_track_resolver::LoadType::Track,
			orpheus_track_resolver::LoadType::Search,
			orpheus_track_resolver::LoadType::Empty,
			orpheus_track_resolver::LoadType::Error,
		] {
			assert!(ensure_playable(&load_type).is_ok());
		}
	}

	#[actix_web::test]
	async fn batch_resolves_mixed_identifiers_in_order() {
		let state = batch_state().await;
		let app = init_service(
			actix_web::App::new()
				.app_data(state)
				.service(web::resource("/v4/loadtracks").route(web::post().to(load_tracks_batch))),
		)
		.await;

		let response = call_service(
			&app,
			TestRequest::post()
				.uri("/v4/loadtracks")
				.set_json(serde_json::json!({"identifiers": ["file:///nonexistent-aaa.txt", "file:///nonexistent-bbb.mp3"]}))
				.to_request(),
		)
		.await;

		assert_eq!(response.status(), StatusCode::OK);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert!(json.is_array());
		assert_eq!(json.as_array().unwrap().len(), 2);
		assert_eq!(json[0]["loadType"], "error");
		assert!(json[0]["data"]["message"].as_str().unwrap().contains("aaa"));
		assert_eq!(json[1]["loadType"], "track");
		assert_eq!(json[1]["data"]["info"]["title"], "nonexistent-bbb");
	}

	#[actix_web::test]
	async fn batch_rejects_empty_and_oversize() {
		let state = batch_state().await;
		let app = init_service(
			actix_web::App::new()
				.app_data(state)
				.service(web::resource("/v4/loadtracks").route(web::post().to(load_tracks_batch))),
		)
		.await;

		let empty = call_service(
			&app,
			TestRequest::post()
				.uri("/v4/loadtracks")
				.set_json(serde_json::json!({"identifiers": []}))
				.to_request(),
		)
		.await;

		assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

		let body = read_body(empty).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["code"], "EMPTY_BATCH");

		let identifiers: Vec<String> = (0..=MAX_BATCH_IDENTIFIERS)
			.map(|i| format!("file:///nonexistent-{i}.mp3"))
			.collect();
		let oversize = call_service(
			&app,
			TestRequest::post()
				.uri("/v4/loadtracks")
				.set_json(serde_json::json!({"identifiers": identifiers}))
				.to_request(),
		)
		.await;

		assert_eq!(oversize.status(), StatusCode::BAD_REQUEST);

		let body = read_body(oversize).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["code"], "BATCH_TOO_LARGE");
	}

	#[actix_web::test]
	async fn dry_run_previews_playback_without_touching_state() {
		let state = batch_state().await;
		let app = init_service(
			actix_web::App::new()
				.app_data(state)
				.service(web::resource("/v4/loadtracks/dry-run").route(web::post().to(load_tracks_dry_run))),
		)
		.await;

		let response = call_service(
			&app,
			TestRequest::post()
				.uri("/v4/loadtracks/dry-run")
				.set_json(serde_json::json!({"identifiers": ["file:///nonexistent-aaa.txt", "file:///nonexistent-bbb.mp3"]}))
				.to_request(),
		)
		.await;

		assert_eq!(response.status(), StatusCode::OK);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert!(json.is_array());
		assert_eq!(json.as_array().unwrap().len(), 2);
		assert_eq!(json[0]["loadType"], "error");
		assert!(json[0]["data"]["message"].as_str().unwrap().contains("aaa"));
		assert!(json[0].get("playbackUrl").unwrap().is_null());
		assert_eq!(json[1]["loadType"], "track");
		assert_eq!(json[1]["data"]["info"]["title"], "nonexistent-bbb");
		assert_eq!(json[1]["playbackUrl"], "file:///nonexistent-bbb.mp3");
	}

	#[actix_web::test]
	async fn dry_run_rejects_oversize_batches() {
		let state = batch_state().await;
		let app = init_service(
			actix_web::App::new()
				.app_data(state)
				.service(web::resource("/v4/loadtracks/dry-run").route(web::post().to(load_tracks_dry_run))),
		)
		.await;

		let identifiers: Vec<String> = (0..=MAX_BATCH_IDENTIFIERS)
			.map(|i| format!("file:///nonexistent-{i}.mp3"))
			.collect();
		let oversize = call_service(
			&app,
			TestRequest::post()
				.uri("/v4/loadtracks/dry-run")
				.set_json(serde_json::json!({"identifiers": identifiers}))
				.to_request(),
		)
		.await;

		assert_eq!(oversize.status(), StatusCode::BAD_REQUEST);

		let body = read_body(oversize).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["code"], "BATCH_TOO_LARGE");
	}

	#[actix_web::test]
	async fn decode_track_without_param_returns_envelope_400() {
		let req = get_request("/v4/decodetrack");
		let query = Query(DecodeTrackQuery {
			encoded_track: None,
			legacy_track: None,
		});

		let error = decode_track(req, query).await.unwrap_err();
		let response = error.error_response();

		assert_eq!(response.status(), StatusCode::BAD_REQUEST);

		let body = actix_web::body::to_bytes(response.into_body()).await.unwrap();
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["status"], 400);
		assert_eq!(json["message"], "No track to decode provided");
		assert_eq!(json["path"], "/v4/decodetrack");
	}

	#[actix_web::test]
	async fn decode_tracks_with_empty_list_returns_envelope_400() {
		let http_req = TestRequest::post().uri("/v4/decodetracks").to_http_request();
		let request = Json(DecodeTracksRequest { tracks: vec![] });

		let error = decode_tracks(http_req, request).await.unwrap_err();

		assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
	}

	#[actix_web::test]
	async fn decode_track_ignores_unknown_params_and_supports_legacy_alias() {
		let app = init_service(
			actix_web::App::new()
				.app_data(web::QueryConfig::default().error_handler(query_error_handler))
				.route("/v4/decodetrack", web::get().to(decode_track)),
		)
		.await;

		let request = TestRequest::get().uri("/v4/decodetrack?trace=true").to_request();
		let response = call_service(&app, request).await;

		assert_eq!(response.status(), StatusCode::BAD_REQUEST);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["status"], 400);
		assert_eq!(json["path"], "/v4/decodetrack");

		let request = TestRequest::get().uri("/v4/decodetrack?track=!!!&trace=true").to_request();
		let response = call_service(&app, request).await;

		assert_eq!(response.status(), StatusCode::BAD_REQUEST);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["status"], 400);
		assert_eq!(json["path"], "/v4/decodetrack");
		assert!(json["message"].is_string());
	}

	#[actix_web::test]
	async fn malformed_json_body_returns_envelope_400() {
		let app = init_service(
			actix_web::App::new()
				.app_data(web::JsonConfig::default().error_handler(json_error_handler))
				.route("/v4/decodetracks", web::post().to(decode_tracks)),
		)
		.await;

		let request = TestRequest::post()
			.uri("/v4/decodetracks")
			.insert_header(("content-type", "application/json"))
			.set_payload(r#"{"tracks": "not-a-list"}"#)
			.to_request();
		let response = call_service(&app, request).await;

		assert_eq!(response.status(), StatusCode::BAD_REQUEST);

		let body = read_body(response).await;
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["status"], 400);
		assert_eq!(json["error"], "Bad Request");
		assert_eq!(json["path"], "/v4/decodetracks");
	}
}
