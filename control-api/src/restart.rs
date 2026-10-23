use std::io::Write;
use std::path::{Path, PathBuf};

use orpheus_core::{PlayerManager, PlayerSnapshot, PlayerState};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use crate::voice::VoiceConnectionManager;

const RESTART_SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestartSnapshot {
	version: u32,
	players: Vec<RestartPlayer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestartPlayer {
	guild_id: u64,
	was_playing: bool,
	voice_channel_id: Option<u64>,
	player: PlayerSnapshot,
}

pub struct RestartSnapshotStore {
	path: PathBuf,
}

impl RestartSnapshotStore {
	pub fn new(path: PathBuf) -> Self {
		Self { path }
	}

	pub async fn restore(&self, state: &AppState, ws_state: &crate::websocket::WebSocketState) -> anyhow::Result<usize> {
		let snapshot = match tokio::fs::read(&self.path).await {
			Ok(bytes) => serde_json::from_slice::<RestartSnapshot>(&bytes)?,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
			Err(error) => return Err(error.into()),
		};

		if snapshot.version != RESTART_SNAPSHOT_VERSION {
			anyhow::bail!("unsupported restart snapshot version {}", snapshot.version);
		}
		let player_count = snapshot.players.len();

		for saved in &snapshot.players {
			if saved.player.version != orpheus_core::SNAPSHOT_VERSION {
				anyhow::bail!(
					"unsupported player snapshot version {} for guild {}",
					saved.player.version,
					saved.guild_id
				);
			}
		}

		for saved in snapshot.players {
			let resume_position = saved.player.position;
			let has_current = saved.player.current.is_some();
			let player_lock = state.player_manager.get_or_create(saved.guild_id).await;
			player_lock.write().await.restore(saved.player);

			if let Some(channel_id) = saved.voice_channel_id {
				state
					.voice_connection_manager
					.set_voice_channel(saved.guild_id, channel_id)
					.await;
			}

			if saved.was_playing && has_current {
				ws_state.arm_restart_resume(saved.guild_id, resume_position);
			}
		}

		self.consume().await?;
		Ok(player_count)
	}

	pub async fn save(&self, players: &PlayerManager, voice: &VoiceConnectionManager) -> anyhow::Result<usize> {
		let active_players = players.get_all_players().await;
		let mut saved_players = Vec::with_capacity(active_players.len());

		for (guild_id, player) in active_players {
			let mut snapshot = player.snapshot();
			if let Some(position) = voice.current_position(guild_id) {
				snapshot.position = position;
			}

			saved_players.push(RestartPlayer {
				guild_id,
				was_playing: player.state == PlayerState::Playing,
				voice_channel_id: voice.voice_channel_id(guild_id),
				player: snapshot,
			});
		}

		let snapshot = RestartSnapshot {
			version: RESTART_SNAPSHOT_VERSION,
			players: saved_players,
		};
		let bytes = serde_json::to_vec(&snapshot)?;
		write_private_atomically(&self.path, bytes).await?;

		Ok(snapshot.players.len())
	}

	async fn consume(&self) -> std::io::Result<()> {
		match tokio::fs::remove_file(&self.path).await {
			Ok(()) => Ok(()),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(error) => Err(error),
		}
	}
}

async fn write_private_atomically(path: &Path, bytes: Vec<u8>) -> anyhow::Result<()> {
	if let Some(parent) = path.parent()
		&& !parent.as_os_str().is_empty()
	{
		tokio::fs::create_dir_all(parent).await?;
	}

	let mut temporary_name = path.file_name().unwrap_or_default().to_os_string();
	temporary_name.push(format!(".{}.tmp", std::process::id()));
	let temporary_path = path.with_file_name(temporary_name);
	let path = path.to_path_buf();

	tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
		let mut options = std::fs::OpenOptions::new();
		options.write(true).create(true).truncate(true);

		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}

		let mut file = options.open(&temporary_path)?;
		file.write_all(&bytes)?;
		file.sync_all()?;
		std::fs::rename(&temporary_path, &path)?;

		#[cfg(unix)]
		if let Some(parent) = path.parent()
			&& !parent.as_os_str().is_empty()
		{
			std::fs::File::open(parent)?.sync_all()?;
		}

		Ok(())
	})
	.await??;

	Ok(())
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use orpheus_core::{Filters, RepeatMode, Track, TrackInfo};
	use orpheus_plugins::PluginManager;
	use orpheus_track_resolver::TrackResolver;

	use super::*;
	use crate::runtime_config::{RuntimeConfig, RuntimeConfigStore};
	use crate::websocket::WebSocketState;

	fn track() -> Track {
		Track {
			encoded: None,
			info: TrackInfo {
				identifier: "track-id".to_string(),
				is_seekable: true,
				author: "artist".to_string(),
				length: 100_000,
				is_stream: false,
				position: 0,
				title: "track".to_string(),
				uri: Some("https://example.invalid/audio.mp3".to_string()),
				artwork_url: None,
				isrc: None,
				source_name: "http".to_string(),
			},
			start_time: None,
			end_time: None,
			plugin_info: serde_json::Value::Null,
			user_data: serde_json::Value::Null,
		}
	}

	async fn app_state() -> (Arc<AppState>, Arc<WebSocketState>) {
		let resolver = Arc::new(TrackResolver::new().await);
		let plugins = Arc::new(tokio::sync::RwLock::new(
			PluginManager::new().await.expect("manager builds without wasm"),
		));
		let config =
			Arc::new(RuntimeConfigStore::in_memory(RuntimeConfig::default(), resolver.configurable_source_names()).unwrap());
		let state = Arc::new(AppState::new(resolver.clone(), plugins, config));
		let websocket = Arc::new(WebSocketState::new(
			state.player_manager.clone(),
			"password".to_string(),
			resolver,
			state.voice_connection_manager.clone(),
			state.metrics.clone(),
			state.prefetch.clone(),
		));

		(state, websocket)
	}

	#[tokio::test]
	async fn snapshot_round_trip_preserves_live_position_and_voice_channel() {
		let path = std::env::temp_dir().join(format!("orpheus-restart-{}.json", std::process::id()));
		let store = RestartSnapshotStore::new(path.clone());
		let (state, _) = app_state().await;
		let player_lock = state.player_manager.get_or_create(7).await;
		{
			let mut player = player_lock.write().await;
			player.current = Some(track());
			player.position = 41_000;
			player.state = PlayerState::Playing;
			player.repeat = RepeatMode::Track;
			player.filters = Filters::default();
		}
		state.voice_connection_manager.set_voice_channel(7, 42).await;

		store
			.save(&state.player_manager, &state.voice_connection_manager)
			.await
			.unwrap();
		let bytes = tokio::fs::read(&path).await.unwrap();
		let snapshot: RestartSnapshot = serde_json::from_slice(&bytes).unwrap();

		assert_eq!(snapshot.players.len(), 1);
		assert_eq!(snapshot.players[0].player.position, 41_000);
		assert_eq!(snapshot.players[0].voice_channel_id, Some(42));
		assert!(snapshot.players[0].was_playing);

		let (restored, websocket) = app_state().await;
		assert_eq!(store.restore(&restored, &websocket).await.unwrap(), 1);
		let restored_player = restored.player_manager.get(7).await.unwrap();
		let restored_player = restored_player.read().await;
		assert_eq!(restored_player.current.as_ref().unwrap().info.identifier, "track-id");
		assert_eq!(restored_player.position, 41_000);
		assert_eq!(restored_player.state, PlayerState::Paused);
		assert_eq!(restored.voice_connection_manager.voice_channel_id(7), Some(42));
		assert_eq!(websocket.take_restart_resume(7), Some(41_000));
		assert!(tokio::fs::metadata(path).await.is_err());
	}
}
