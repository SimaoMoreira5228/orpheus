use std::sync::Arc;
use std::time::Duration;

use crate::sessions::SessionStore;

pub async fn destroy_all_players(
	player_manager: &orpheus_core::PlayerManager,
	voice_connection_manager: &crate::voice::VoiceConnectionManager,
) {
	// NOTE: players are global, not scoped per session, so session teardown clears
	// every player and voice connection, mirroring Lavalink's context shutdown.
	for guild_id in player_manager.list().await {
		voice_connection_manager.destroy(guild_id).await;
		player_manager.remove(guild_id).await;
	}
}

pub fn spawn_resume_reaper(
	sessions: Arc<SessionStore>,
	player_manager: Arc<orpheus_core::PlayerManager>,
	voice_connection_manager: Arc<crate::voice::VoiceConnectionManager>,
) {
	tokio::spawn(async move {
		let mut interval = tokio::time::interval(Duration::from_secs(1));

		loop {
			interval.tick().await;

			if sessions.reap().is_empty() {
				continue;
			}

			destroy_all_players(&player_manager, &voice_connection_manager).await;
		}
	});
}

pub fn spawn_event_parker(
	sessions: Arc<SessionStore>,
	player_manager: Arc<orpheus_core::PlayerManager>,
	voice_connection_manager: Arc<crate::voice::VoiceConnectionManager>,
) {
	tokio::spawn(async move {
		let mut events = voice_connection_manager.subscribe();
		let mut updates = voice_connection_manager.subscribe_player_updates();

		loop {
			tokio::select! {
				Ok(event) = events.recv() => {
					let guild_id = event.guild_id();
					let outgoing = crate::voice_events::voice_event_to_outgoing(&event);

					if let Ok(json) = serde_json::to_string(&outgoing) {
						sessions.park_owned(guild_id, json);
					}
				}
				Ok(guild_id) = updates.recv() => {
					let players = player_manager.get_all_players().await;

					let parked = crate::websocket::player_update_json(&voice_connection_manager, guild_id, &players);

					if let Some(json) = parked {
						sessions.park_owned(guild_id, json);
					}
				}
				else => break,
			}
		}
	});
}
