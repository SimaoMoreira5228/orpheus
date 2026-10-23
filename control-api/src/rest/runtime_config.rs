use std::sync::Arc;

use actix_web::HttpRequest;
use actix_web::web::{Data, Json};

use crate::errors::{ErrorCode, RestError};
use crate::runtime_config::{RuntimeConfig, RuntimeConfigUpdateError};
use crate::state::AppState;

pub async fn get_runtime_config(state: Data<Arc<AppState>>) -> Json<RuntimeConfig> {
	Json(state.runtime_config.current())
}

pub async fn replace_runtime_config(
	req: HttpRequest,
	state: Data<Arc<AppState>>,
	request: Json<RuntimeConfig>,
) -> Result<Json<RuntimeConfig>, RestError> {
	let config = match state.runtime_config.replace(request.into_inner()).await {
		Ok(config) => config,
		Err(RuntimeConfigUpdateError::Invalid(message)) => {
			return Err(RestError::bad_request(ErrorCode::InvalidRuntimeConfig, message, &req));
		}
		Err(RuntimeConfigUpdateError::Persistence(error)) => {
			return Err(RestError::internal(
				ErrorCode::RuntimeConfigWriteFailed,
				format!("failed to persist runtime config: {error}"),
				&req,
			));
		}
	};

	state
		.track_resolver
		.configure_runtime(
			config.source_order.clone(),
			config.default_source_timeout_ms,
			config.source_timeouts_ms.clone(),
		)
		.map_err(|message| RestError::internal(ErrorCode::InternalError, message, &req))?;
	state
		.voice_connection_manager
		.set_diagnostic_capacity(config.diagnostics_capacity);

	Ok(Json(config))
}
