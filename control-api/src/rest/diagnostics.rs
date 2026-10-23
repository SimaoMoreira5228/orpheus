use std::sync::Arc;

use actix_web::HttpRequest;
use actix_web::web::{Data, Json, Path};
use serde::Serialize;

use super::sessions::parse_guild_id;
use crate::errors::RestError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct VoiceDiagnosticsResponse {
	#[serde(rename = "guildId")]
	pub guild_id: String,
	pub events: Vec<crate::voice::VoiceDiagnostic>,
}

pub async fn get_voice_diagnostics(
	req: HttpRequest,
	path: Path<String>,
	state: Data<Arc<AppState>>,
) -> Result<Json<VoiceDiagnosticsResponse>, RestError> {
	let guild_raw = path.into_inner();
	let guild_id = parse_guild_id(&guild_raw, &req)?;

	Ok(Json(VoiceDiagnosticsResponse {
		guild_id: guild_id.to_string(),
		events: state.voice_connection_manager.voice_diagnostics(guild_id),
	}))
}
