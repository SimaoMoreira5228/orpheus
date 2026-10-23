use std::sync::Arc;
use std::time::Duration;

use actix::prelude::*;
use actix_web_actors::ws;
use orpheus_core::{Player, PlayerState as CorePlayerState};

use super::session::WebSocketSession;
use super::ws_message;
use crate::sessions::SessionStore;

const PLAYER_UPDATE_INTERVAL: Duration = Duration::from_secs(5);
const STATS_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) fn spawn_resumed_replay(ctx: &mut ws::WebsocketContext<WebSocketSession>, session: &WebSocketSession) {
	let state = session.state.clone();
	let sessions = session.state.sessions.clone();
	let session_id = session.session_id.clone();

	let fut = async move {
		let players = state.player_manager.get_all_players().await;

		owned_player_updates(&sessions, &session_id, &state.voice_connection_manager, &players)
	};

	let fut = fut.into_actor(session).map(|msgs, _actor, ctx| {
		for m in msgs {
			ctx.text(m);
		}
	});

	ctx.spawn(fut);
}

fn owned_player_updates(
	sessions: &Arc<SessionStore>,
	session_id: &str,
	voice_connection_manager: &crate::voice::VoiceConnectionManager,
	players: &[(u64, Player)],
) -> Vec<String> {
	let mut msgs = Vec::new();

	for (guild_id, player) in players {
		if player.current.is_none() || !sessions.delivers_to(session_id, *guild_id) {
			continue;
		}

		if let Some(json) = player_update_json(voice_connection_manager, *guild_id, players) {
			msgs.push(json);
		}
	}

	msgs
}

pub(crate) fn spawn_player_update_tick(ctx: &mut ws::WebsocketContext<WebSocketSession>, session: &WebSocketSession) {
	let tick_state = session.state.clone();
	let tick_sessions = session.state.sessions.clone();
	let tick_session_id = session.session_id.clone();

	ctx.run_interval(PLAYER_UPDATE_INTERVAL, move |actor, ctx| {
		let state = tick_state.clone();
		let sessions = tick_sessions.clone();
		let session_id = tick_session_id.clone();
		let fut = async move {
			let players = state.player_manager.get_all_players().await;

			owned_player_updates(&sessions, &session_id, &state.voice_connection_manager, &players)
		};

		let fut = fut.into_actor(actor).map(|msgs, _actor, ctx| {
			for m in msgs {
				ctx.text(m);
			}
		});

		ctx.spawn(fut);
	});
}

pub(crate) fn spawn_stats_tick(ctx: &mut ws::WebsocketContext<WebSocketSession>, session: &WebSocketSession) {
	let stats_state = session.state.clone();

	ctx.run_interval(STATS_INTERVAL, move |actor, ctx| {
		let state = stats_state.clone();
		let fut = async move {
			let players = state.player_manager.get_all_players().await;
			let playing = players
				.iter()
				.filter(|(_, player)| player.state == CorePlayerState::Playing)
				.count();

			state.metrics.update_player_metrics(players.len(), playing);
			let snapshot = state.metrics.snapshot();

			let stats = crate::websocket_messages::Stats {
				players: players.len(),
				playing_players: playing,
				uptime: snapshot.uptime_ms,
				memory: crate::websocket_messages::Memory {
					free: snapshot.memory_free,
					used: snapshot.memory_used,
					allocated: snapshot.memory_total,
					reservable: snapshot.memory_total,
				},
				cpu: crate::websocket_messages::Cpu {
					cores: std::thread::available_parallelism()
						.map(|cores| cores.get() as i32)
						.unwrap_or(1),
					system_load: snapshot.system_load,
					lavalink_load: snapshot.process_load,
				},
				frame_stats: None,
			};

			let msg = crate::websocket_messages::OutgoingMessage::Stats(stats);
			serde_json::to_string(&msg).ok()
		};

		let fut = fut.into_actor(actor).map(|maybe, _actor, ctx| {
			if let Some(json) = maybe {
				tracing::debug!(">>> SENDING STATS: {}", json);
				ctx.text(json);
			}
		});

		ctx.spawn(fut);
	});
}

pub(crate) fn spawn_event_fanout(ctx: &mut ws::WebsocketContext<WebSocketSession>, session: &WebSocketSession) {
	let mut event_rx = session.state.voice_connection_manager.subscribe();
	let event_sessions = session.state.sessions.clone();
	let event_session_id = session.session_id.clone();
	let event_state = session.state.clone();
	let event_addr = ctx.address();

	ctx.spawn(
		async move {
			loop {
				match event_rx.recv().await {
					Ok(event) => {
						if !event_sessions.delivers_to(&event_session_id, event.guild_id()) {
							continue;
						}

						let outgoing = crate::voice_events::voice_event_to_outgoing(&event);

						if let Ok(json) = serde_json::to_string(&outgoing) {
							event_addr.do_send(ws_message::Text(json));
						}

						if matches!(
							event,
							crate::voice_events::VoiceEvent::Started { .. } | crate::voice_events::VoiceEvent::Stuck { .. }
						) {
							let players = event_state.player_manager.get_all_players().await;

							if let Some(json) =
								player_update_json(&event_state.voice_connection_manager, event.guild_id(), &players)
							{
								event_addr.do_send(ws_message::Text(json));
							}
						}
					}
					Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
					Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
				}
			}
		}
		.into_actor(session)
		.map(|_, _, _| {}),
	);
}

pub(crate) fn spawn_player_update_fanout(ctx: &mut ws::WebsocketContext<WebSocketSession>, session: &WebSocketSession) {
	let mut update_rx = session.state.voice_connection_manager.subscribe_player_updates();
	let update_sessions = session.state.sessions.clone();
	let update_session_id = session.session_id.clone();
	let update_state = session.state.clone();
	let update_addr = ctx.address();

	let update_task = async move {
		loop {
			match update_rx.recv().await {
				Ok(guild_id) => {
					if !update_sessions.delivers_to(&update_session_id, guild_id) {
						continue;
					}

					let players = update_state.player_manager.get_all_players().await;

					if let Some(json) = player_update_json(&update_state.voice_connection_manager, guild_id, &players) {
						update_addr.do_send(ws_message::Text(json));
					}
				}
				Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
				Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
			}
		}
	};

	ctx.spawn(update_task.into_actor(session).map(|_, _, _| {}));
}

pub(crate) fn player_update_json(
	voice_connection_manager: &crate::voice::VoiceConnectionManager,
	guild_id: u64,
	players: &[(u64, Player)],
) -> Option<String> {
	let (_, player) = players.iter().find(|(guild, _)| *guild == guild_id)?;

	let position = voice_connection_manager.current_position(guild_id).unwrap_or(player.position);

	let update = crate::websocket_messages::OutgoingMessage::PlayerUpdate {
		guild_id: guild_id.to_string(),
		state: crate::websocket_messages::PlayerState::snapshot(
			position as i64,
			voice_connection_manager.is_connected(guild_id),
		),
	};

	serde_json::to_string(&update).ok()
}
