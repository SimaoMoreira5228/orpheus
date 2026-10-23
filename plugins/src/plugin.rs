use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
	#[error("Plugin error: {0}")]
	Custom(String),
	#[error("Plugin not initialized")]
	NotInitialized,
	#[error("Plugin initialization failed: {0}")]
	InitializationFailed(String),
	#[error("Plugin execution failed: {0}")]
	ExecutionFailed(String),
	#[error("Invalid configuration: {0}")]
	InvalidConfig(String),
}

pub type PluginResult<T> = Result<T, PluginError>;

pub trait Plugin: Send + Sync {
	fn name(&self) -> &str;
	fn version(&self) -> &str;
	fn on_load(&self) -> PluginResult<()>;
	fn on_unload(&self);
}

pub trait TrackResolverPlugin: Plugin {
	fn resolve(&self, query: &str) -> PluginResult<TrackInfo>;
}

pub trait AudioFilterPlugin: Plugin {
	fn filter(&self, frame: &mut [i16]) -> PluginResult<()>;
	fn sample_rate(&self) -> u32;
	fn channels(&self) -> u16;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
	pub id: String,
	pub title: String,
	pub artist: String,
	pub album: Option<String>,
	pub duration_ms: u64,
	pub url: String,
}

impl TrackInfo {
	pub fn new(id: String, title: String, artist: String, url: String) -> Self {
		Self {
			id,
			title,
			artist,
			album: None,
			duration_ms: 0,
			url,
		}
	}

	pub fn with_album(mut self, album: String) -> Self {
		self.album = Some(album);
		self
	}

	pub fn with_duration(mut self, duration_ms: u64) -> Self {
		self.duration_ms = duration_ms;
		self
	}

	pub fn to_json(&self) -> String {
		serde_json::to_string(self).unwrap_or_default()
	}

	pub fn from_json(json: &str) -> PluginResult<Self> {
		serde_json::from_str(json).map_err(|e| PluginError::Custom(format!("Failed to parse track info: {}", e)))
	}
}
