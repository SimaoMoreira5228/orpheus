use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, watch};

const DEFAULT_SOURCE_TIMEOUT_MS: u64 = 30_000;
const MIN_SOURCE_TIMEOUT_MS: u64 = 100;
const MAX_SOURCE_TIMEOUT_MS: u64 = 120_000;
const MAX_DIAGNOSTIC_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
	pub source_order: Vec<String>,
	#[serde(default = "default_source_timeout_ms")]
	pub default_source_timeout_ms: u64,
	#[serde(default)]
	pub source_timeouts_ms: BTreeMap<String, u64>,
	#[serde(default)]
	pub prefetch: PrefetchConfig,
	#[serde(default = "default_diagnostic_capacity")]
	pub diagnostics_capacity: usize,
	#[serde(default = "default_log_level")]
	pub log_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrefetchConfig {
	#[serde(default = "default_prefetch_enabled")]
	pub enabled: bool,
	#[serde(default = "default_prefetch_window_ms")]
	pub lookahead_ms: u64,
	#[serde(default = "default_prefetch_max_age_ms")]
	pub max_url_age_ms: u64,
	#[serde(default = "default_prefetch_concurrency")]
	pub max_concurrent: usize,
}

impl Default for RuntimeConfig {
	fn default() -> Self {
		Self {
			source_order: Vec::new(),
			default_source_timeout_ms: default_source_timeout_ms(),
			source_timeouts_ms: BTreeMap::new(),
			prefetch: PrefetchConfig::default(),
			diagnostics_capacity: default_diagnostic_capacity(),
			log_level: default_log_level(),
		}
	}
}

impl Default for PrefetchConfig {
	fn default() -> Self {
		Self {
			enabled: default_prefetch_enabled(),
			lookahead_ms: default_prefetch_window_ms(),
			max_url_age_ms: default_prefetch_max_age_ms(),
			max_concurrent: default_prefetch_concurrency(),
		}
	}
}

impl RuntimeConfig {
	pub fn validate(&self, available_sources: &[String]) -> Result<(), String> {
		if self.source_order.is_empty() {
			return Err("source_order must contain at least one source".to_string());
		}

		let mut seen = HashSet::new();
		for source in &self.source_order {
			if !available_sources.iter().any(|available| available == source) {
				return Err(format!("unknown source in source_order: {source}"));
			}
			if !seen.insert(source) {
				return Err(format!("duplicate source in source_order: {source}"));
			}
		}

		validate_timeout("default_source_timeout_ms", self.default_source_timeout_ms)?;
		for (source, timeout_ms) in &self.source_timeouts_ms {
			if !available_sources.iter().any(|available| available == source) {
				return Err(format!("unknown source timeout: {source}"));
			}
			validate_timeout(source, *timeout_ms)?;
		}

		if !(1_000..=120_000).contains(&self.prefetch.lookahead_ms) {
			return Err("prefetch.lookahead_ms must be between 1000 and 120000".to_string());
		}
		if !(1_000..=300_000).contains(&self.prefetch.max_url_age_ms) {
			return Err("prefetch.max_url_age_ms must be between 1000 and 300000".to_string());
		}
		if !(1..=16).contains(&self.prefetch.max_concurrent) {
			return Err("prefetch.max_concurrent must be between 1 and 16".to_string());
		}
		if self.diagnostics_capacity > MAX_DIAGNOSTIC_CAPACITY {
			return Err(format!("diagnostics_capacity must not exceed {MAX_DIAGNOSTIC_CAPACITY}"));
		}
		if !matches!(self.log_level.as_str(), "off" | "error" | "warn" | "info" | "debug" | "trace") {
			return Err("log_level must be off, error, warn, info, debug or trace".to_string());
		}

		Ok(())
	}
}

fn validate_timeout(name: &str, timeout_ms: u64) -> Result<(), String> {
	if !(MIN_SOURCE_TIMEOUT_MS..=MAX_SOURCE_TIMEOUT_MS).contains(&timeout_ms) {
		return Err(format!(
			"{name} must be between {MIN_SOURCE_TIMEOUT_MS} and {MAX_SOURCE_TIMEOUT_MS}"
		));
	}

	Ok(())
}

fn default_source_timeout_ms() -> u64 {
	DEFAULT_SOURCE_TIMEOUT_MS
}

fn default_prefetch_enabled() -> bool {
	true
}

fn default_prefetch_window_ms() -> u64 {
	15_000
}

fn default_prefetch_max_age_ms() -> u64 {
	30_000
}

fn default_prefetch_concurrency() -> usize {
	4
}

fn default_diagnostic_capacity() -> usize {
	64
}

fn default_log_level() -> String {
	"info".to_string()
}

pub struct RuntimeConfigStore {
	path: Option<PathBuf>,
	available_sources: Vec<String>,
	current: watch::Sender<RuntimeConfig>,
	update_lock: Mutex<()>,
}

impl RuntimeConfigStore {
	pub async fn load(path: PathBuf, mut initial: RuntimeConfig, available_sources: Vec<String>) -> Result<Self, String> {
		let stored = match tokio::fs::read_to_string(&path).await {
			Ok(contents) => Some(toml::from_str(&contents).map_err(|error| format!("invalid runtime config: {error}"))?),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
			Err(error) => return Err(format!("failed to read runtime config: {error}")),
		};
		if let Some(stored) = stored {
			initial = stored;
		}

		if initial.source_order.is_empty() {
			initial.source_order = available_sources.clone();
		}
		initial.validate(&available_sources)?;

		Ok(Self::from_config(initial, Some(path), available_sources))
	}

	pub fn in_memory(mut initial: RuntimeConfig, available_sources: Vec<String>) -> Result<Self, String> {
		if initial.source_order.is_empty() {
			initial.source_order = available_sources.clone();
		}
		initial.validate(&available_sources)?;

		Ok(Self::from_config(initial, None, available_sources))
	}

	fn from_config(initial: RuntimeConfig, path: Option<PathBuf>, available_sources: Vec<String>) -> Self {
		let (current, _) = watch::channel(initial);

		Self {
			path,
			available_sources,
			current,
			update_lock: Mutex::new(()),
		}
	}

	pub fn current(&self) -> RuntimeConfig {
		self.current.borrow().clone()
	}

	pub fn subscribe(&self) -> watch::Receiver<RuntimeConfig> {
		self.current.subscribe()
	}

	pub async fn replace(&self, config: RuntimeConfig) -> Result<RuntimeConfig, RuntimeConfigUpdateError> {
		config
			.validate(&self.available_sources)
			.map_err(RuntimeConfigUpdateError::Invalid)?;

		let _guard = self.update_lock.lock().await;
		if let Some(path) = &self.path {
			persist_config(path, &config)
				.await
				.map_err(RuntimeConfigUpdateError::Persistence)?;
		}
		self.current.send_replace(config.clone());

		Ok(config)
	}
}

#[derive(Debug)]
pub enum RuntimeConfigUpdateError {
	Invalid(String),
	Persistence(std::io::Error),
}

async fn persist_config(path: &Path, config: &RuntimeConfig) -> std::io::Result<()> {
	if let Some(parent) = path.parent()
		&& !parent.as_os_str().is_empty()
	{
		tokio::fs::create_dir_all(parent).await?;
	}

	let serialized = toml::to_string_pretty(config).map_err(std::io::Error::other)?;
	let mut temporary_name = path.file_name().unwrap_or_default().to_os_string();
	temporary_name.push(format!(".{}.tmp", std::process::id()));
	let temporary_path = path.with_file_name(temporary_name);
	let path = path.to_path_buf();

	tokio::task::spawn_blocking(move || -> std::io::Result<()> {
		let mut options = std::fs::OpenOptions::new();
		options.write(true).create(true).truncate(true);

		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}

		let mut file = options.open(&temporary_path)?;
		file.write_all(serialized.as_bytes())?;
		file.sync_all()?;
		std::fs::rename(&temporary_path, &path)?;
		Ok(())
	})
	.await?
}

#[cfg(test)]
mod tests {
	use super::*;

	fn available_sources() -> Vec<String> {
		vec!["youtube".to_string(), "http".to_string()]
	}

	#[test]
	fn source_order_rejects_unknown_and_duplicate_names() {
		let config = RuntimeConfig {
			source_order: vec!["youtube".to_string(), "missing".to_string()],
			..RuntimeConfig::default()
		};
		assert!(config.validate(&available_sources()).is_err());

		let duplicate = RuntimeConfig {
			source_order: vec!["youtube".to_string(), "youtube".to_string()],
			..RuntimeConfig::default()
		};
		assert!(duplicate.validate(&available_sources()).is_err());
	}

	#[tokio::test]
	async fn runtime_config_replacement_persists_and_notifies_subscribers() {
		let path = std::env::temp_dir().join(format!("orpheus-runtime-{}.toml", std::process::id()));
		let sources = available_sources();
		let store = RuntimeConfigStore::load(path.clone(), RuntimeConfig::default(), sources.clone())
			.await
			.unwrap();
		let mut updates = store.subscribe();
		let mut updated = store.current();
		updated.source_order = vec!["http".to_string(), "youtube".to_string()];
		updated.log_level = "warn".to_string();

		store.replace(updated.clone()).await.unwrap();
		updates.changed().await.unwrap();
		assert_eq!(*updates.borrow_and_update(), updated);

		let loaded = RuntimeConfigStore::load(path.clone(), RuntimeConfig::default(), sources)
			.await
			.unwrap();
		assert_eq!(loaded.current(), updated);

		let _ = tokio::fs::remove_file(path).await;
	}
}
