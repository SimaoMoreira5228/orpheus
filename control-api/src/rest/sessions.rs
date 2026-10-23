use std::sync::Arc;

use actix_web::web::{self, Data, Json, Path};
use actix_web::{HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, RestError};
use crate::sessions::SessionStore;
use crate::state::AppState;

pub(crate) fn require_session(sessions: &SessionStore, session_id: &str, req: &HttpRequest) -> Result<(), RestError> {
	match sessions.get(session_id) {
		Some(_) => Ok(()),
		None => Err(RestError::not_found(ErrorCode::SessionNotFound, "Session not found", req)),
	}
}

pub(crate) fn parse_guild_id(raw: &str, req: &HttpRequest) -> Result<u64, RestError> {
	match raw.parse::<u64>() {
		Ok(guild_id) => Ok(guild_id),
		Err(_) => Err(RestError::bad_request(
			ErrorCode::InvalidGuildId,
			format!("Invalid guild id: {raw}"),
			req,
		)),
	}
}

#[derive(Debug, Deserialize)]
pub struct VoiceChannelRequest {
	#[serde(deserialize_with = "orpheus_core::string_or_number", rename = "channelId")]
	pub channel_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionUpdateRequest {
	#[serde(default)]
	pub resuming: Option<bool>,
	#[serde(default, rename = "timeout", alias = "timeoutSeconds")]
	pub timeout_secs: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SessionUpdateResponse {
	pub resuming: bool,
	#[serde(rename = "timeout")]
	pub timeout_secs: u64,
}

pub async fn set_voice_channel(
	req: HttpRequest,
	path: Path<(String, String)>,
	body: web::Json<VoiceChannelRequest>,
	state: Data<Arc<AppState>>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
) -> Result<HttpResponse, RestError> {
	let (session_id, guild_raw) = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	require_session(&ws_state.sessions, &session_id, &req)?;
	ws_state.sessions.touch(&session_id, guild_id);

	match body.channel_id.parse::<u64>() {
		Ok(channel_id) => {
			tracing::info!("Voice channel update for guild {}: channel_id={}", guild_id, channel_id);
			state.voice_connection_manager.set_voice_channel(guild_id, channel_id).await;
			Ok(HttpResponse::NoContent().finish())
		}
		Err(_) => Err(RestError::bad_request(ErrorCode::InvalidChannelId, "invalid channelId", &req)),
	}
}

pub async fn update_session(
	req: HttpRequest,
	path: Path<String>,
	ws_state: Data<Arc<crate::websocket::WebSocketState>>,
	request: Json<SessionUpdateRequest>,
) -> Result<Json<SessionUpdateResponse>, RestError> {
	let session_id = path.into_inner();
	let body = request.into_inner();

	match ws_state.sessions.update(&session_id, body.resuming, body.timeout_secs) {
		Some((resuming, timeout_secs)) => Ok(Json(SessionUpdateResponse { resuming, timeout_secs })),
		None => Err(RestError::not_found(ErrorCode::SessionNotFound, "Session not found", &req)),
	}
}

#[cfg(test)]
mod tests {
	use actix_web::ResponseError;
	use actix_web::http::StatusCode;
	use actix_web::test::TestRequest;

	use super::*;

	fn get_request(path: &str) -> HttpRequest {
		TestRequest::get().uri(path).to_http_request()
	}

	#[test]
	fn guild_id_parsing_accepts_numeric_strings() {
		let req = get_request("/v4/sessions/abc/players/123");

		assert_eq!(parse_guild_id("123", &req).unwrap(), 123);

		let error = parse_guild_id("not-a-snowflake", &req).unwrap_err();

		assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
	}

	#[test]
	fn session_guard_rejects_unknown_sessions() {
		let store = SessionStore::new();
		let req = get_request("/v4/sessions/missing/players");

		let error = require_session(&store, "missing", &req).unwrap_err();

		assert_eq!(error.status_code(), StatusCode::NOT_FOUND);

		let created = store.create(7);

		assert!(require_session(&store, &created.id, &req).is_ok());
	}

	#[test]
	fn channel_id_accepts_string_or_number() {
		let from_string: VoiceChannelRequest = serde_json::from_str(r#"{"channelId": "123"}"#).unwrap();

		assert_eq!(from_string.channel_id, "123");

		let from_number: VoiceChannelRequest = serde_json::from_str(r#"{"channelId": 123}"#).unwrap();

		assert_eq!(from_number.channel_id, "123");
	}

	#[test]
	fn empty_session_patch_preserves_existing_settings() {
		let store = SessionStore::new();
		let created = store.create(7);
		store.update(&created.id, Some(true), Some(120));

		let request: SessionUpdateRequest = serde_json::from_str("{}").unwrap();

		let settings = store.update(&created.id, request.resuming, request.timeout_secs).unwrap();
		assert_eq!(settings, (true, 120));

		let response = SessionUpdateResponse {
			resuming: settings.0,
			timeout_secs: settings.1,
		};

		assert_eq!(
			serde_json::to_value(response).unwrap(),
			serde_json::json!({ "resuming": true, "timeout": 120 })
		);
	}
}
