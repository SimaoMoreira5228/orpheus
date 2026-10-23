use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, ResponseError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct RestErrorBody {
	pub timestamp: i64,
	pub status: u16,
	pub error: String,
	pub code: ErrorCode,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub message: Option<String>,
	pub path: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub trace: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
	InvalidGuildId,
	InvalidChannelId,
	InvalidEndTime,
	InvalidVoiceState,
	ConflictingTrackFields,
	MissingTrack,
	InvalidAddress,
	EmptyTrackList,
	UndecodableTrack,
	InvalidQuery,
	InvalidJsonBody,
	InvalidPlugin,
	InvalidPluginBinary,
	InvalidFilterValue,
	InvalidRuntimeConfig,
	InvalidIdentifier,
	UnsupportedSource,
	PlaylistNotPlayable,
	SessionNotFound,
	PlayerNotFound,
	PluginNotFound,
	UnsupportedPluginEndpoint,
	FilterPresetNotFound,
	UnsupportedMigrationVersion,
	EmptyBatch,
	BatchTooLarge,
	NoMatches,
	NoAudioStream,
	RateLimited,
	SourceAuthRequired,
	SourceAccessForbidden,
	SourceUnavailable,
	SourceTimeout,
	RuntimeConfigWriteFailed,
	FetchFailed,
	ResponseParseFailed,
	RoutePlannerDisabled,
	PluginRegistryInconsistent,
	PluginGuestFailed,
	PluginRouteFailed,
	InternalError,
}

impl ErrorCode {
	pub fn status(&self) -> StatusCode {
		match self {
			ErrorCode::SessionNotFound
			| ErrorCode::PlayerNotFound
			| ErrorCode::PluginNotFound
			| ErrorCode::UnsupportedPluginEndpoint
			| ErrorCode::FilterPresetNotFound
			| ErrorCode::NoMatches
			| ErrorCode::NoAudioStream => StatusCode::NOT_FOUND,

			ErrorCode::RoutePlannerDisabled
			| ErrorCode::PluginRegistryInconsistent
			| ErrorCode::PluginGuestFailed
			| ErrorCode::PluginRouteFailed
			| ErrorCode::InternalError
			| ErrorCode::FetchFailed
			| ErrorCode::SourceUnavailable
			| ErrorCode::ResponseParseFailed
			| ErrorCode::RuntimeConfigWriteFailed => StatusCode::INTERNAL_SERVER_ERROR,
			ErrorCode::SourceTimeout => StatusCode::GATEWAY_TIMEOUT,

			ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
			ErrorCode::SourceAuthRequired => StatusCode::UNAUTHORIZED,
			ErrorCode::SourceAccessForbidden => StatusCode::FORBIDDEN,

			_ => StatusCode::BAD_REQUEST,
		}
	}
}

#[derive(Debug)]
pub struct RestError {
	status: StatusCode,
	code: ErrorCode,
	message: Option<String>,
	path: String,
	trace: Option<String>,
}

fn trace_for(req: &HttpRequest, caller: &std::panic::Location<'_>) -> Option<String> {
	let wanted = req.query_string().split('&').any(|pair| {
		let mut parts = pair.splitn(2, '=');

		match (parts.next(), parts.next()) {
			(Some(key), Some(value)) => key == "trace" && value == "true",
			_ => false,
		}
	});

	if !wanted {
		return None;
	}

	Some(format!("{}:{}:{}", caller.file(), caller.line(), caller.column()))
}

impl RestError {
	#[track_caller]
	pub fn new(code: ErrorCode, message: Option<String>, path: &str) -> Self {
		Self {
			status: code.status(),
			code,
			message,
			path: path.to_string(),
			trace: None,
		}
	}

	#[track_caller]
	pub fn bad_request(code: ErrorCode, message: impl Into<String>, req: &HttpRequest) -> Self {
		let trace = trace_for(req, std::panic::Location::caller());

		Self::with_trace(code, Some(message.into()), req.path(), trace)
	}

	#[track_caller]
	pub fn not_found(code: ErrorCode, message: impl Into<String>, req: &HttpRequest) -> Self {
		let trace = trace_for(req, std::panic::Location::caller());

		Self::with_trace(code, Some(message.into()), req.path(), trace)
	}

	#[track_caller]
	pub fn internal(code: ErrorCode, message: impl Into<String>, req: &HttpRequest) -> Self {
		let trace = trace_for(req, std::panic::Location::caller());

		Self::with_trace(code, Some(message.into()), req.path(), trace)
	}

	fn with_trace(code: ErrorCode, message: Option<String>, path: &str, trace: Option<String>) -> Self {
		Self {
			status: code.status(),
			code,
			message,
			path: path.to_string(),
			trace,
		}
	}
}

impl std::fmt::Display for RestError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(formatter, "{}: {}", self.status, self.message.as_deref().unwrap_or(""))
	}
}

fn error_timestamp_ms() -> i64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_millis() as i64
}

impl ResponseError for RestError {
	fn status_code(&self) -> StatusCode {
		self.status
	}

	fn error_response(&self) -> HttpResponse {
		let body = RestErrorBody {
			timestamp: error_timestamp_ms(),
			status: self.status.as_u16(),
			error: self.status.canonical_reason().unwrap_or("Unknown").to_string(),
			code: self.code,
			message: self.message.clone(),
			path: self.path.clone(),
			trace: self.trace.clone(),
		};

		HttpResponse::build(self.status).json(body)
	}
}

pub fn code_for_resolver(error: &orpheus_track_resolver::ResolverError) -> ErrorCode {
	use orpheus_track_resolver::ResolverError as Resolver;

	match error {
		Resolver::NotFound(_) => ErrorCode::NoMatches,
		Resolver::InvalidIdentifier(_) => ErrorCode::InvalidIdentifier,
		Resolver::UnsupportedSource(_) => ErrorCode::UnsupportedSource,
		Resolver::NoAudioStream => ErrorCode::NoAudioStream,
		Resolver::Timeout(_) => ErrorCode::SourceTimeout,
		Resolver::ParseError(_) => ErrorCode::ResponseParseFailed,
		Resolver::IoError(_) => ErrorCode::FetchFailed,
		Resolver::Codec(_) => ErrorCode::UndecodableTrack,
		Resolver::FetchError(detail) => fetch_failure_code(detail),
	}
}

fn fetch_failure_code(detail: &str) -> ErrorCode {
	let lower = detail.to_ascii_lowercase();

	if lower.contains("429") || lower.contains("rate limit") || lower.contains("ratelimit") {
		return ErrorCode::RateLimited;
	}

	if lower.contains("401") || lower.contains("unauthorized") {
		return ErrorCode::SourceAuthRequired;
	}

	if lower.contains("403") || lower.contains("forbidden") {
		return ErrorCode::SourceAccessForbidden;
	}

	if lower.contains("404") || lower.contains("not found") {
		return ErrorCode::NoMatches;
	}

	ErrorCode::SourceUnavailable
}

pub fn query_error_handler(error: actix_web::error::QueryPayloadError, req: &HttpRequest) -> actix_web::Error {
	RestError::new(ErrorCode::InvalidQuery, Some(error.to_string()), req.path()).into()
}

pub fn json_error_handler(error: actix_web::error::JsonPayloadError, req: &HttpRequest) -> actix_web::Error {
	RestError::new(ErrorCode::InvalidJsonBody, Some(error.to_string()), req.path()).into()
}

#[cfg(test)]
mod tests {
	use actix_web::test::TestRequest;

	use super::*;

	#[actix_web::test]
	async fn error_envelope_matches_lavalink_shape() {
		let req = TestRequest::get().uri("/v4/sessions/abc/players/7").to_http_request();
		let error = RestError::bad_request(ErrorCode::InvalidEndTime, "End time must be greater than 0", &req);
		let response = error.error_response();

		assert_eq!(response.status(), StatusCode::BAD_REQUEST);

		let body = actix_web::body::to_bytes(response.into_body()).await.unwrap();
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert!(json["timestamp"].is_number());
		assert!(json["timestamp"].as_i64().unwrap() > 0);
		assert_eq!(json["status"], 400);
		assert_eq!(json["error"], "Bad Request");
		assert_eq!(json["code"], "INVALID_END_TIME");
		assert_eq!(json["message"], "End time must be greater than 0");
		assert_eq!(json["path"], "/v4/sessions/abc/players/7");
		assert!(json.get("trace").is_none());
	}

	#[actix_web::test]
	async fn not_found_envelope_carries_status_and_path() {
		let req = TestRequest::get().uri("/v4/sessions/missing/players").to_http_request();
		let error = RestError::not_found(ErrorCode::SessionNotFound, "Session not found", &req);
		let response = error.error_response();

		assert_eq!(response.status(), StatusCode::NOT_FOUND);

		let body = actix_web::body::to_bytes(response.into_body()).await.unwrap();
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

		assert_eq!(json["status"], 404);
		assert_eq!(json["error"], "Not Found");
		assert_eq!(json["code"], "SESSION_NOT_FOUND");
		assert_eq!(json["message"], "Session not found");
		assert_eq!(json["path"], "/v4/sessions/missing/players");
		assert!(json.get("trace").is_none());
	}

	#[test]
	fn error_codes_derive_their_status() {
		assert_eq!(ErrorCode::InvalidEndTime.status(), StatusCode::BAD_REQUEST);
		assert_eq!(ErrorCode::MissingTrack.status(), StatusCode::BAD_REQUEST);
		assert_eq!(ErrorCode::SessionNotFound.status(), StatusCode::NOT_FOUND);
		assert_eq!(ErrorCode::FilterPresetNotFound.status(), StatusCode::NOT_FOUND);
		assert_eq!(ErrorCode::NoMatches.status(), StatusCode::NOT_FOUND);
		assert_eq!(ErrorCode::RateLimited.status(), StatusCode::TOO_MANY_REQUESTS);
		assert_eq!(ErrorCode::SourceAuthRequired.status(), StatusCode::UNAUTHORIZED);
		assert_eq!(ErrorCode::SourceAccessForbidden.status(), StatusCode::FORBIDDEN);
		assert_eq!(ErrorCode::SourceTimeout.status(), StatusCode::GATEWAY_TIMEOUT);
		assert_eq!(ErrorCode::InternalError.status(), StatusCode::INTERNAL_SERVER_ERROR);
		assert_eq!(ErrorCode::SourceUnavailable.status(), StatusCode::INTERNAL_SERVER_ERROR);
	}

	#[actix_web::test]
	async fn trace_param_adds_caller_location() {
		let plain = TestRequest::get().uri("/v4/sessions/abc/players/7").to_http_request();
		let plain_error = RestError::bad_request(ErrorCode::InternalError, "Bad", &plain);
		let plain_body = actix_web::body::to_bytes(plain_error.error_response().into_body())
			.await
			.unwrap();
		let plain_json: serde_json::Value = serde_json::from_slice(&plain_body).unwrap();

		assert!(plain_json.get("trace").is_none());

		let traced = TestRequest::get()
			.uri("/v4/sessions/abc/players/7?trace=true")
			.to_http_request();
		let traced_error = RestError::bad_request(ErrorCode::InternalError, "Bad", &traced);
		let traced_body = actix_web::body::to_bytes(traced_error.error_response().into_body())
			.await
			.unwrap();
		let traced_json: serde_json::Value = serde_json::from_slice(&traced_body).unwrap();

		let trace = traced_json["trace"].as_str().expect("trace present");

		assert!(trace.contains("errors.rs"), "trace was: {trace}");

		let other = TestRequest::get().uri("/v4/info?trace=false").to_http_request();
		let other_error = RestError::not_found(ErrorCode::SessionNotFound, "Missing", &other);
		let other_body = actix_web::body::to_bytes(other_error.error_response().into_body())
			.await
			.unwrap();
		let other_json: serde_json::Value = serde_json::from_slice(&other_body).unwrap();

		assert!(other_json.get("trace").is_none());
	}
}
