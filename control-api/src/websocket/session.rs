use std::sync::Arc;

use actix::prelude::*;
use actix_web::{HttpRequest, HttpResponse, web};
use actix_web_actors::ws;
use dashmap::DashMap;
use orpheus_core::PlayerManager;
use tracing::{debug, error};

use super::ws_message;
use crate::sessions::{CloseOutcome, Reconnect, SessionStore};

#[derive(Clone)]
pub struct WebSocketState {
	pub player_manager: Arc<PlayerManager>,
	pub sessions: Arc<SessionStore>,
	pub metrics: Arc<crate::metrics::MetricsState>,
	pub password: String,
	pub track_resolver: Arc<orpheus_track_resolver::TrackResolver>,
	pub voice_connection_manager: Arc<crate::voice::VoiceConnectionManager>,
	pub prefetch: Arc<crate::prefetch::PrefetchCoordinator>,
	restart_resume_positions: Arc<DashMap<u64, u64>>,
}

impl WebSocketState {
	pub fn new(
		player_manager: Arc<PlayerManager>,
		password: String,
		track_resolver: Arc<orpheus_track_resolver::TrackResolver>,
		voice_connection_manager: Arc<crate::voice::VoiceConnectionManager>,
		metrics: Arc<crate::metrics::MetricsState>,
		prefetch: Arc<crate::prefetch::PrefetchCoordinator>,
	) -> Self {
		let sessions = Arc::new(SessionStore::new());

		crate::session_tasks::spawn_resume_reaper(
			sessions.clone(),
			player_manager.clone(),
			voice_connection_manager.clone(),
		);
		crate::session_tasks::spawn_event_parker(sessions.clone(), player_manager.clone(), voice_connection_manager.clone());

		Self {
			player_manager,
			sessions,
			metrics,
			password,
			track_resolver,
			voice_connection_manager,
			prefetch,
			restart_resume_positions: Arc::new(DashMap::new()),
		}
	}

	pub fn arm_restart_resume(&self, guild_id: u64, position_ms: u64) {
		self.restart_resume_positions.insert(guild_id, position_ms);
	}

	pub fn take_restart_resume(&self, guild_id: u64) -> Option<u64> {
		self.restart_resume_positions.remove(&guild_id).map(|(_, position)| position)
	}
}

pub fn configure(cfg: &mut web::ServiceConfig) {
	cfg.service(web::resource("/v4/websocket").route(web::get().to(ws_index)));
}

pub async fn ws_index(
	req: HttpRequest,
	stream: web::Payload,
	data: web::Data<Arc<WebSocketState>>,
) -> Result<HttpResponse, actix_web::Error> {
	debug!("WebSocket request: method={}, path={}", req.method(), req.path());
	let headers = req.headers();

	let auth_header = headers
		.get("Authorization")
		.and_then(|v| v.to_str().ok())
		.map(|s| s.to_string())
		.unwrap_or_default();

	debug!("User-Id header: {:?}", headers.get("User-Id"));
	debug!("Client-Name header: {:?}", headers.get("Client-Name"));
	debug!("Session-Id header: {:?}", headers.get("Session-Id"));

	if auth_header != data.password {
		return Ok(HttpResponse::Unauthorized().body("Invalid password"));
	}

	let user_id = match headers
		.get("User-Id")
		.and_then(|v| v.to_str().ok())
		.and_then(|s| s.parse().ok())
	{
		Some(id) => id,
		None => return Ok(HttpResponse::BadRequest().body("Missing User-Id header")),
	};

	let client_name = headers
		.get("Client-Name")
		.and_then(|v| v.to_str().ok())
		.map(|s| s.to_string())
		.unwrap_or_else(|| "orpheus/0.1.0".to_string());

	let session_id = headers.get("Session-Id").and_then(|v| v.to_str().ok()).map(|s| s.to_string());

	let (session_id, resumed, replay) = match session_id {
		Some(ref sid) => match data.sessions.reconnect(sid, user_id) {
			Reconnect::Resumed { session, parked } => {
				tracing::info!("Resuming paused session: {}", session.id);

				(session.id.clone(), true, parked)
			}
			Reconnect::Live(session) => (session.id, false, Vec::new()),
			Reconnect::Gone => {
				let session = data.sessions.create(user_id);

				(session.id, false, Vec::new())
			}
		},
		None => {
			let session = data.sessions.create(user_id);

			(session.id, false, Vec::new())
		}
	};

	let ws = WebSocketSession {
		state: data.get_ref().clone(),
		session_id: session_id.clone(),
		resumed,
		replay,
		client_name,
	};

	debug!("Starting WebSocket session: session_id={}", ws.session_id);

	match ws::start(ws, &req, stream) {
		Ok(response) => {
			debug!("WebSocket started successfully");
			Ok(response)
		}
		Err(e) => {
			error!("WebSocket start error: {:?}", e);
			Ok(HttpResponse::InternalServerError().body(format!("WebSocket error: {}", e)))
		}
	}
}

#[allow(dead_code)]
pub struct WebSocketSession {
	pub(crate) state: Arc<WebSocketState>,
	pub(crate) session_id: String,
	pub(crate) resumed: bool,
	pub(crate) replay: Vec<String>,
	pub(crate) client_name: String,
}

impl actix::Handler<ws_message::Text> for WebSocketSession {
	type Result = ();

	fn handle(&mut self, msg: ws_message::Text, ctx: &mut Self::Context) {
		ctx.text(msg.0);
	}
}

impl Actor for WebSocketSession {
	type Context = ws::WebsocketContext<Self>;

	fn stopped(&mut self, _ctx: &mut Self::Context) {
		self.park_or_destroy();
	}

	fn started(&mut self, ctx: &mut Self::Context) {
		let source_managers = self.state.track_resolver.source_names();
		let resumed = self.resumed;
		let ready = crate::websocket_messages::OutgoingMessage::Ready {
			session_id: self.session_id.clone(),
			resumed,
			source_managers,
		};

		if let Ok(json) = serde_json::to_string(&ready) {
			tracing::info!(">>> SENDING READY: {}", json);
			ctx.text(json);
		}

		if self.resumed {
			for parked in self.replay.clone() {
				ctx.text(parked);
			}

			super::outgoing::spawn_resumed_replay(ctx, self);
		}

		super::outgoing::spawn_player_update_tick(ctx, self);
		super::outgoing::spawn_stats_tick(ctx, self);
		super::outgoing::spawn_event_fanout(ctx, self);
		super::outgoing::spawn_player_update_fanout(ctx, self);
	}
}

impl WebSocketSession {
	pub(crate) fn park_or_destroy(&self) {
		match self.state.sessions.on_close(&self.session_id) {
			CloseOutcome::Parked => {
				tracing::info!("Session {} is resumable, pausing", self.session_id);
			}
			CloseOutcome::Destroyed | CloseOutcome::Unknown => {
				let state = self.state.clone();

				actix::spawn(async move {
					crate::session_tasks::destroy_all_players(&state.player_manager, &state.voice_connection_manager).await;
				});
			}
		}
	}
}
