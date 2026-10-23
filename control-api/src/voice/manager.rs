use std::sync::Arc;

use dashmap::DashMap;
use orpheus_core::{Filters, Track, VoiceState};
use tokio::sync::{RwLock as AsyncRwLock, broadcast, mpsc, oneshot};
use tracing::info;

use super::connection::run_voice_connection;
use super::diagnostics::VoiceDiagnostics;
use crate::voice_events::VoiceEvent;

pub struct VoiceConnectionManager {
	connections: DashMap<u64, Arc<VoiceConnectionHandle>>,
	ffmpeg_path: AsyncRwLock<String>,
	channels: Arc<DashMap<u64, u64>>,
	event_tx: broadcast::Sender<VoiceEvent>,
	progress_tx: broadcast::Sender<VoiceProgress>,
	live_voice: Arc<DashMap<u64, ()>>,
	live_positions: Arc<DashMap<u64, u64>>,
	player_update_tx: broadcast::Sender<u64>,
	diagnostics: Arc<VoiceDiagnostics>,
}

pub struct VoiceConnectionHandle {
	pub command_tx: mpsc::Sender<VoiceCommand>,
}

#[derive(Debug, Clone, Copy)]
pub struct VoiceProgress {
	pub guild_id: u64,
	pub generation: u64,
	pub position_ms: u64,
}

#[derive(Debug)]
pub enum VoiceCommand {
	Connect(u64, VoiceState),
	Reconnect,
	Disconnect(oneshot::Sender<()>),
	Play {
		track: Box<Track>,
		seek_position: u64,
		volume: u32,
		filters: Filters,
		end_time: Option<u64>,
	},
	Stop,
	Pause(bool),
	Seek(u64),
	SetVolume(u32),
	SetFilters(Filters),
	StreamEnded {
		generation: u64,
	},
	Progress {
		generation: u64,
		position_ms: u64,
	},
}

impl VoiceConnectionManager {
	pub fn new() -> Self {
		let (event_tx, _) = broadcast::channel(256);
		let (progress_tx, _) = broadcast::channel(256);
		let (player_update_tx, _) = broadcast::channel(256);

		Self {
			connections: DashMap::new(),
			ffmpeg_path: AsyncRwLock::new("ffmpeg".to_string()),
			channels: Arc::new(DashMap::new()),
			event_tx,
			progress_tx,
			live_voice: Arc::new(DashMap::new()),
			live_positions: Arc::new(DashMap::new()),
			player_update_tx,
			diagnostics: Arc::new(VoiceDiagnostics::new(64)),
		}
	}

	pub fn subscribe(&self) -> broadcast::Receiver<VoiceEvent> {
		self.event_tx.subscribe()
	}

	pub fn subscribe_progress(&self) -> broadcast::Receiver<VoiceProgress> {
		self.progress_tx.subscribe()
	}

	pub fn is_connected(&self, guild_id: u64) -> bool {
		self.live_voice.contains_key(&guild_id)
	}

	pub fn current_position(&self, guild_id: u64) -> Option<u64> {
		if !self.live_voice.contains_key(&guild_id) {
			return None;
		}

		self.live_positions.get(&guild_id).map(|entry| *entry.value())
	}

	pub fn voice_channel_id(&self, guild_id: u64) -> Option<u64> {
		self.channels.get(&guild_id).map(|channel| *channel)
	}

	pub fn subscribe_player_updates(&self) -> broadcast::Receiver<u64> {
		self.player_update_tx.subscribe()
	}

	pub fn voice_diagnostics(&self, guild_id: u64) -> Vec<super::VoiceDiagnostic> {
		self.diagnostics.for_guild(guild_id)
	}

	pub fn set_diagnostic_capacity(&self, capacity: usize) {
		self.diagnostics.set_capacity(capacity);
	}

	pub fn request_player_update(&self, guild_id: u64) {
		let _ = self.player_update_tx.send(guild_id);
	}

	pub async fn set_ffmpeg_path(&self, path: String) {
		*self.ffmpeg_path.write().await = path;
	}

	pub async fn set_voice_channel(&self, guild_id: u64, channel_id: u64) {
		self.channels.insert(guild_id, channel_id);

		let handle = self.connections.get(&guild_id).map(|entry| entry.value().clone());

		if let Some(handle) = handle {
			let _ = handle.command_tx.send(VoiceCommand::Reconnect).await;
		}
	}

	async fn ensure_handle(&self, guild_id: u64) -> Arc<VoiceConnectionHandle> {
		if let Some(handle) = self.connections.get(&guild_id).map(|entry| entry.value().clone()) {
			return handle;
		}

		let (tx, rx) = mpsc::channel(32);
		let handle = Arc::new(VoiceConnectionHandle { command_tx: tx.clone() });

		let ours = {
			let entry = self.connections.entry(guild_id).or_insert(handle.clone());

			Arc::ptr_eq(entry.value(), &handle)
		};

		if !ours {
			return self
				.connections
				.get(&guild_id)
				.map(|entry| entry.value().clone())
				.expect("racer inserted a handle");
		}

		let ffmpeg_path = self.ffmpeg_path.read().await.clone();
		let channels = self.channels.clone();
		let event_tx = self.event_tx.clone();
		let progress_tx = self.progress_tx.clone();
		let live_voice = self.live_voice.clone();
		let live_positions = self.live_positions.clone();
		let player_update_tx = self.player_update_tx.clone();
		let diagnostics = self.diagnostics.clone();

		tokio::spawn(async move {
			run_voice_connection(
				guild_id,
				rx,
				tx,
				ffmpeg_path,
				channels,
				event_tx,
				progress_tx,
				live_voice,
				live_positions,
				player_update_tx,
				diagnostics,
			)
			.await;
		});

		handle
	}

	pub async fn connect(&self, guild_id: u64, user_id: u64, voice_state: VoiceState) {
		info!(
			"VoiceConnectionManager::connect guild={} endpoint={}",
			guild_id, voice_state.endpoint
		);
		let handle = self.ensure_handle(guild_id).await;
		let _ = handle.command_tx.send(VoiceCommand::Connect(user_id, voice_state)).await;
	}

	pub async fn play(
		&self,
		guild_id: u64,
		track: Track,
		seek_position: u64,
		volume: u32,
		filters: Filters,
		end_time: Option<u64>,
	) {
		let handle = self.ensure_handle(guild_id).await;

		let _ = handle
			.command_tx
			.send(VoiceCommand::Play {
				track: Box::new(track),
				seek_position,
				volume,
				filters,
				end_time,
			})
			.await;
	}

	pub async fn pause(&self, guild_id: u64, paused: bool) {
		let handle = self.connections.get(&guild_id).map(|entry| entry.value().clone());

		if let Some(handle) = handle {
			let _ = handle.command_tx.send(VoiceCommand::Pause(paused)).await;
		}
	}

	pub async fn seek(&self, guild_id: u64, position_ms: u64) {
		let handle = self.connections.get(&guild_id).map(|entry| entry.value().clone());

		if let Some(handle) = handle {
			let _ = handle.command_tx.send(VoiceCommand::Seek(position_ms)).await;
		}
	}

	pub async fn set_volume(&self, guild_id: u64, volume: u32) {
		let handle = self.connections.get(&guild_id).map(|entry| entry.value().clone());

		if let Some(handle) = handle {
			let _ = handle.command_tx.send(VoiceCommand::SetVolume(volume)).await;
		}
	}

	pub async fn set_filters(&self, guild_id: u64, filters: Filters) {
		let handle = self.connections.get(&guild_id).map(|entry| entry.value().clone());

		if let Some(handle) = handle {
			let _ = handle.command_tx.send(VoiceCommand::SetFilters(filters)).await;
		}
	}

	pub async fn stop(&self, guild_id: u64) {
		let handle = self.connections.get(&guild_id).map(|entry| entry.value().clone());

		if let Some(handle) = handle {
			let _ = handle.command_tx.send(VoiceCommand::Stop).await;
		}
	}

	pub async fn destroy(&self, guild_id: u64) {
		self.live_voice.remove(&guild_id);
		self.live_positions.remove(&guild_id);

		let handle = self.connections.get(&guild_id).map(|entry| entry.value().clone());

		let Some(handle) = handle else {
			return;
		};
		let _ = handle.command_tx.send(VoiceCommand::Stop).await;

		if self.disconnect_actor(guild_id, &handle).await {
			self.connections.remove(&guild_id);
		}
	}

	pub async fn disconnect(&self, guild_id: u64) {
		self.live_voice.remove(&guild_id);
		self.live_positions.remove(&guild_id);

		let Some(handle) = self.connections.get(&guild_id).map(|entry| entry.value().clone()) else {
			return;
		};

		if self.disconnect_actor(guild_id, &handle).await {
			self.connections.remove(&guild_id);
		}
	}

	async fn disconnect_actor(&self, guild_id: u64, handle: &VoiceConnectionHandle) -> bool {
		let (disconnected, confirmation) = oneshot::channel();
		if handle.command_tx.send(VoiceCommand::Disconnect(disconnected)).await.is_err() {
			return true;
		}

		if tokio::time::timeout(std::time::Duration::from_secs(10), confirmation)
			.await
			.is_ok_and(|result| result.is_ok())
		{
			return true;
		}

		tracing::warn!("Timed out closing voice runtime for guild {}", guild_id);
		false
	}

	pub async fn shutdown_all(&self) {
		let connections: Vec<_> = self
			.connections
			.iter()
			.map(|entry| (*entry.key(), entry.value().clone()))
			.collect();
		let mut pending = Vec::new();

		for (guild_id, handle) in connections {
			self.live_voice.remove(&guild_id);
			self.live_positions.remove(&guild_id);
			let _ = handle.command_tx.send(VoiceCommand::Stop).await;
			let (disconnected, confirmation) = oneshot::channel();

			if handle.command_tx.send(VoiceCommand::Disconnect(disconnected)).await.is_ok() {
				pending.push((guild_id, confirmation));
			} else {
				self.connections.remove(&guild_id);
			}
		}

		let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
		for (guild_id, confirmation) in pending {
			let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
			let closed = tokio::time::timeout(remaining, confirmation)
				.await
				.is_ok_and(|result| result.is_ok());

			if closed {
				self.connections.remove(&guild_id);
			} else {
				tracing::warn!("Timed out closing voice runtime for guild {}", guild_id);
			}
		}
	}
}

impl Default for VoiceConnectionManager {
	fn default() -> Self {
		Self::new()
	}
}
