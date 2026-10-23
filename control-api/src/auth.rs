use actix_web::body::{EitherBody, MessageBody};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{Error, HttpResponse, web};

#[derive(Clone)]
pub struct Password(pub String);

fn is_exempt(path: &str) -> bool {
	matches!(
		path,
		"/metrics" | "/health" | "/stats" | "/info" | "/version" | "/v4/websocket"
	)
}

pub async fn require_auth(
	req: ServiceRequest,
	next: Next<impl MessageBody>,
) -> Result<ServiceResponse<EitherBody<impl MessageBody>>, Error> {
	// NOTE: REST auth exempts /metrics, which Prometheus scrapes without credentials,
	// and the non-API diagnostics (/health, /stats, /info, /version). The WebSocket
	// upgrade is exempt here because it performs its own Authorization check.
	if is_exempt(req.path()) {
		return next.call(req).await.map(|res| res.map_into_left_body());
	}

	let password = req
		.app_data::<web::Data<Password>>()
		.map(|data| data.get_ref().0.clone())
		.unwrap_or_default();

	let denial = match req.headers().get("Authorization").and_then(|value| value.to_str().ok()) {
		None => Some(HttpResponse::Unauthorized().finish()),
		Some(provided) if provided != password.as_str() => Some(HttpResponse::Forbidden().finish()),
		Some(_) => None,
	};

	if let Some(denied) = denial {
		return Ok(req.into_response(denied).map_into_right_body());
	}

	next.call(req).await.map(|res| res.map_into_left_body())
}
