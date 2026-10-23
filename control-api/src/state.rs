use std::sync::Arc;
use std::time::Instant;

use orpheus_core::PlayerManager;
use orpheus_plugins::PluginManager;
use orpheus_track_resolver::{LoadResult, TrackResolver};

use crate::errors::code_for_resolver;
use crate::filter_presets::FilterPresetStore;
use crate::metrics::MetricsState;
use crate::prefetch::PrefetchCoordinator;
use crate::routeplanner::RoutePlannerManager;
use crate::runtime_config::RuntimeConfigStore;
use crate::voice::VoiceConnectionManager;

pub struct AppState {
	pub player_manager: Arc<PlayerManager>,
	pub start_time: Instant,
	pub metrics: Arc<MetricsState>,
	pub route_planner: Arc<RoutePlannerManager>,
	pub track_resolver: Arc<TrackResolver>,
	pub plugin_manager: Arc<tokio::sync::RwLock<PluginManager>>,
	pub voice_connection_manager: Arc<VoiceConnectionManager>,
	pub filter_presets: FilterPresetStore,
	pub runtime_config: Arc<RuntimeConfigStore>,
	pub prefetch: Arc<PrefetchCoordinator>,
}

impl AppState {
	pub fn new(
		track_resolver: Arc<TrackResolver>,
		plugin_manager: Arc<tokio::sync::RwLock<PluginManager>>,
		runtime_config: Arc<RuntimeConfigStore>,
	) -> Self {
		let voice_connection_manager = Arc::new(VoiceConnectionManager::new());
		voice_connection_manager.set_diagnostic_capacity(runtime_config.current().diagnostics_capacity);
		let player_manager = Arc::new(PlayerManager::new());
		let prefetch = Arc::new(PrefetchCoordinator::new(
			player_manager.clone(),
			track_resolver.clone(),
			runtime_config.clone(),
		));
		let vcm = voice_connection_manager.clone();
		let tr = track_resolver.clone();
		tokio::spawn(async move {
			let path = tr.ffmpeg_path().await;
			vcm.set_ffmpeg_path(path).await;
		});

		crate::autoplay::spawn_autoplay(
			player_manager.clone(),
			track_resolver.clone(),
			voice_connection_manager.clone(),
			prefetch.clone(),
		);
		prefetch.clone().spawn(voice_connection_manager.clone());

		Self {
			player_manager,
			start_time: Instant::now(),
			metrics: Arc::new(MetricsState::new()),
			route_planner: Arc::new(RoutePlannerManager::new()),
			track_resolver,
			plugin_manager,
			voice_connection_manager,
			filter_presets: FilterPresetStore::new(),
			runtime_config,
			prefetch,
		}
	}

	pub fn uptime(&self) -> u64 {
		self.start_time.elapsed().as_secs()
	}

	pub async fn load_tracks(
		&self,
		identifier: &str,
	) -> Result<(LoadResult, &'static str), orpheus_track_resolver::ResolverError> {
		let started = std::time::Instant::now();
		let outcome = self.track_resolver.load(identifier).await;
		let elapsed_ms = started.elapsed().as_millis() as u64;

		match &outcome {
			Ok((_, source)) => self.metrics.metrics.record_resolve(source, "OK", elapsed_ms),
			Err(error) => {
				let code = match serde_json::to_value(code_for_resolver(error)) {
					Ok(serde_json::Value::String(code)) => code,
					_ => "UNKNOWN".to_string(),
				};

				self.metrics.metrics.record_resolve("unknown", &code, elapsed_ms);
			}
		}

		outcome
	}
}
#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn load_tracks_records_source_and_outcome() {
		let track_resolver = Arc::new(orpheus_track_resolver::TrackResolver::new().await);
		let plugins = Arc::new(tokio::sync::RwLock::new(
			orpheus_plugins::PluginManager::new()
				.await
				.expect("manager builds without wasm"),
		));
		let runtime_config = Arc::new(
			RuntimeConfigStore::in_memory(
				crate::runtime_config::RuntimeConfig::default(),
				track_resolver.configurable_source_names(),
			)
			.unwrap(),
		);
		let state = AppState::new(track_resolver, plugins, runtime_config);

		let (result, source) = state
			.load_tracks("file:///nonexistent-bbb.mp3")
			.await
			.expect("local file resolves offline");

		assert_eq!(source, "local");
		assert!(!result.tracks.is_empty());

		state
			.load_tracks("file:///nonexistent-aaa.txt")
			.await
			.expect_err("unsupported file fails offline");

		let counts = state.metrics.metrics.resolve_counts.lock();

		assert_eq!(counts.get(&("local".to_string(), "OK".to_string())), Some(&1));

		let unknown: u64 = counts
			.iter()
			.filter(|((source, _), _)| source == "unknown")
			.map(|(_, count)| count)
			.sum();

		assert_eq!(unknown, 1);
	}
}
