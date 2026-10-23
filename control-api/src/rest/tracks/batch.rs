use std::sync::Arc;

use actix_web::HttpRequest;
use actix_web::web::{Data, Json};
use serde::{Deserialize, Serialize};

use super::{
	LoadResultType, LoadTracksResponse, LoadTracksResponseData, error_response_for, load_failure_for, response_for_result,
};
use crate::errors::{ErrorCode, RestError};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct BatchTracksRequest {
	pub identifiers: Vec<String>,
}

// NOTE: one request fans out to N full resolves (each an extract + network),
// so the batch is capped and in-flight resolves are bounded — without this a
// single request could pin the blocking pool and the network.
pub const MAX_BATCH_IDENTIFIERS: usize = 25;

const BATCH_CONCURRENCY: usize = 8;

pub async fn load_tracks_batch(
	req: HttpRequest,
	state: Data<Arc<AppState>>,
	request: Json<BatchTracksRequest>,
) -> Result<Json<Vec<LoadTracksResponse>>, RestError> {
	let identifiers = request.into_inner().identifiers;

	check_batch_request(&identifiers, &req)?;
	Ok(Json(resolve_batch(&state, &identifiers).await))
}

fn check_batch_request(identifiers: &[String], req: &HttpRequest) -> Result<(), RestError> {
	if identifiers.is_empty() {
		return Err(RestError::bad_request(
			ErrorCode::EmptyBatch,
			"No identifiers to resolve provided",
			req,
		));
	}

	if identifiers.len() > MAX_BATCH_IDENTIFIERS {
		return Err(RestError::bad_request(
			ErrorCode::BatchTooLarge,
			format!("Too many identifiers: maximum is {MAX_BATCH_IDENTIFIERS}"),
			req,
		));
	}

	Ok(())
}

async fn resolve_batch(state: &Arc<AppState>, identifiers: &[String]) -> Vec<LoadTracksResponse> {
	use tokio::task::JoinSet;

	let mut ordered: std::collections::BTreeMap<usize, LoadTracksResponse> = std::collections::BTreeMap::new();
	let mut next = 0;
	let mut in_flight = JoinSet::new();

	while ordered.len() < identifiers.len() {
		while in_flight.len() < BATCH_CONCURRENCY && next < identifiers.len() {
			let state = state.clone();
			let identifier = identifiers[next].clone();
			let position = next;

			next += 1;

			in_flight.spawn(async move {
				let response = match state.load_tracks(&identifier).await {
					Ok((result, _)) => response_for_result(result),
					Err(error) => error_response_for(&error),
				};

				(position, response)
			});
		}

		match in_flight.join_next().await {
			Some(Ok((position, response))) => {
				ordered.insert(position, response);
			}
			Some(Err(_)) => break,
			None => break,
		}
	}

	(0..identifiers.len())
		.map(|position| {
			ordered.remove(&position).unwrap_or_else(|| {
				error_response_for(&orpheus_track_resolver::ResolverError::FetchError(
					"batch resolve task failed".to_string(),
				))
			})
		})
		.collect()
}

#[derive(Debug, Serialize)]
pub struct DryRunItem {
	#[serde(rename = "loadType")]
	pub load_type: LoadResultType,
	pub data: LoadTracksResponseData,
	#[serde(rename = "playbackUrl")]
	pub playback_url: Option<String>,
}

pub async fn load_tracks_dry_run(
	req: HttpRequest,
	state: Data<Arc<AppState>>,
	request: Json<BatchTracksRequest>,
) -> Result<Json<Vec<DryRunItem>>, RestError> {
	let identifiers = request.into_inner().identifiers;

	check_batch_request(&identifiers, &req)?;
	Ok(Json(resolve_dry_run(&state, &identifiers).await))
}

async fn resolve_dry_run(state: &Arc<AppState>, identifiers: &[String]) -> Vec<DryRunItem> {
	use tokio::task::JoinSet;

	let mut ordered: std::collections::BTreeMap<usize, DryRunItem> = std::collections::BTreeMap::new();
	let mut next = 0;
	let mut in_flight = JoinSet::new();

	while ordered.len() < identifiers.len() {
		while in_flight.len() < BATCH_CONCURRENCY && next < identifiers.len() {
			let state = state.clone();
			let identifier = identifiers[next].clone();
			let position = next;

			next += 1;

			in_flight.spawn(async move {
				let item = match state.load_tracks(&identifier).await {
					Ok((result, _)) => {
						let playback_url = if let Some(track) = result.tracks.first() {
							Some(state.track_resolver.playback_url(track).await)
						} else {
							None
						};

						let response = response_for_result(result);

						DryRunItem {
							load_type: response.load_type,
							data: response.data,
							playback_url,
						}
					}
					Err(error) => DryRunItem {
						load_type: LoadResultType::Error,
						data: LoadTracksResponseData::Error(load_failure_for(&error)),
						playback_url: None,
					},
				};

				(position, item)
			});
		}

		match in_flight.join_next().await {
			Some(Ok((position, item))) => {
				ordered.insert(position, item);
			}
			Some(Err(_)) => break,
			None => break,
		}
	}

	(0..identifiers.len())
		.map(|position| {
			ordered.remove(&position).unwrap_or(DryRunItem {
				load_type: LoadResultType::Error,
				data: LoadTracksResponseData::Error(load_failure_for(&orpheus_track_resolver::ResolverError::FetchError(
					"dry-run resolve task failed".to_string(),
				))),
				playback_url: None,
			})
		})
		.collect()
}
