use std::collections::HashMap;
use std::path::PathBuf;

use crate::host::{HostError, HostResult, LoadedPlugin, PluginHost, PluginType, ResourceLimits, RouteResponse};

#[derive(Debug)]
pub struct PluginManager {
	host: PluginHost,
	plugins: HashMap<String, LoadedPlugin>,
	plugin_dirs: Vec<PathBuf>,
	auto_reload: bool,
}

impl PluginManager {
	pub async fn new() -> HostResult<Self> {
		let host = PluginHost::new()?;
		Ok(Self {
			host,
			plugins: HashMap::new(),
			plugin_dirs: Vec::new(),
			auto_reload: false,
		})
	}

	pub async fn new_with_limits(limits: ResourceLimits) -> HostResult<Self> {
		let host = PluginHost::new_with_limits(limits)?;
		Ok(Self {
			host,
			plugins: HashMap::new(),
			plugin_dirs: Vec::new(),
			auto_reload: false,
		})
	}

	pub fn set_plugin_dirs(&mut self, dirs: Vec<PathBuf>) {
		self.plugin_dirs = dirs;
	}

	pub fn enable_auto_reload(&mut self, enabled: bool) {
		self.auto_reload = enabled;
	}

	pub async fn load(&mut self, name: &str, wasm_bytes: &[u8]) -> HostResult<()> {
		let plugin = self.host.load_plugin(wasm_bytes, name).await?;
		tracing::info!(
			"Loaded plugin: {} v{} (type: {:?})",
			plugin.name,
			plugin.version,
			plugin.plugin_type
		);
		self.plugins.insert(name.to_string(), plugin);
		self.apply_stored_config(name).await;
		Ok(())
	}

	pub async fn load_from_file(&mut self, name: &str, path: &PathBuf) -> HostResult<()> {
		let plugin = self.host.load_plugin_from_file(path, name).await?;
		tracing::info!(
			"Loaded plugin: {} v{} (type: {:?}) from {}",
			plugin.name,
			plugin.version,
			plugin.plugin_type,
			path.display()
		);
		self.plugins.insert(name.to_string(), plugin);
		self.apply_stored_config(name).await;
		Ok(())
	}

	async fn apply_stored_config(&mut self, name: &str) {
		let Some(config) = self.host.plugin_config(name).await else {
			return;
		};

		let Ok(config_json) = serde_json::to_string(&config) else {
			return;
		};

		if let Err(e) = self.host.call_configure(name, &config_json).await {
			tracing::warn!("Stored config for plugin {name} failed to apply: {e}");
		}
	}

	pub async fn set_plugin_config(&mut self, name: &str, config: serde_json::Value) -> HostResult<()> {
		self.host.set_plugin_config(name, config).await
	}

	pub async fn plugin_config(&self, name: &str) -> Option<serde_json::Value> {
		self.host.plugin_config(name).await
	}

	pub async fn plugin_capabilities(&self, name: &str) -> HostResult<Vec<String>> {
		self.host.capabilities(name).await
	}

	pub async fn handle_route(
		&mut self,
		plugin_name: &str,
		method: &str,
		path: &str,
		query: &str,
		headers_json: &str,
		body_b64: &str,
	) -> HostResult<RouteResponse> {
		let plugin = self
			.plugins
			.get(plugin_name)
			.ok_or_else(|| HostError::NotFound(plugin_name.to_string()))?;

		if !plugin.routes.iter().any(|route| route.matches(method, path)) {
			return Err(HostError::NotFound(format!(
				"no route {method} {path} in plugin {plugin_name}"
			)));
		}

		self.host
			.call_route(plugin_name, method, path, query, headers_json, body_b64)
			.await
	}

	pub async fn load_all_from_dirs(&mut self) -> HostResult<Vec<String>> {
		let mut loaded = Vec::new();

		let dirs = self.plugin_dirs.clone();

		for dir in dirs.iter() {
			if dir.is_dir() {
				match std::fs::read_dir(dir) {
					Ok(entries) => {
						for entry in entries.flatten() {
							let path = entry.path();
							if path.extension().is_some_and(|ext| ext == "wasm") {
								let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown").to_string();

								match self.load_from_file(&name, &path).await {
									Ok(_) => loaded.push(name),
									Err(e) => {
										tracing::warn!("Failed to load plugin from {}: {}", path.display(), e);
									}
								}
							}
						}
					}
					Err(e) => {
						tracing::warn!("Failed to read plugin directory {}: {}", dir.display(), e);
					}
				}
			}
		}

		Ok(loaded)
	}

	pub async fn unload(&mut self, name: &str) {
		if self.plugins.remove(name).is_some() {
			if let Err(e) = self.host.unload_plugin(name).await {
				tracing::warn!("Error unloading plugin {}: {}", name, e);
			} else {
				tracing::info!("Unloaded plugin: {}", name);
			}
		}
	}

	pub async fn hot_reload(&mut self, name: &str, wasm_bytes: &[u8]) -> HostResult<()> {
		if !self.plugins.contains_key(name) {
			return Err(HostError::NotFound(name.to_string()));
		}

		self.unload(name).await;

		self.load(name, wasm_bytes)
			.await
			.map_err(|e| HostError::HotReloadFailed(format!("Failed to hot reload plugin {name}: {e}")))
	}

	pub async fn hot_reload_from_file(&mut self, name: &str) -> HostResult<()> {
		let path = self
			.plugins
			.get(name)
			.ok_or_else(|| HostError::NotFound(name.to_string()))?
			.path
			.clone();

		let Some(path) = path else {
			return Err(HostError::HotReloadFailed(format!("No stored file path for plugin {name}")));
		};

		self.unload(name).await;

		self.load_from_file(name, &path)
			.await
			.map_err(|e| HostError::HotReloadFailed(format!("Failed to hot reload plugin {name}: {e}")))
	}

	pub async fn reload_all(&mut self) -> HostResult<Vec<String>> {
		let names: Vec<String> = self.plugins.keys().cloned().collect();

		for name in &names {
			if let Err(e) = self.hot_reload_from_file(name).await {
				tracing::warn!("Failed to reload plugin {}: {}", name, e);
			}
		}

		Ok(names)
	}

	pub fn list(&self) -> Vec<String> {
		self.plugins.keys().cloned().collect()
	}

	pub fn list_by_type(&self, plugin_type: PluginType) -> Vec<String> {
		self.plugins
			.iter()
			.filter(|(_, p)| p.plugin_type == plugin_type)
			.map(|(name, _)| name.clone())
			.collect()
	}

	pub fn track_resolver_plugins(&self) -> Vec<(String, LoadedPlugin)> {
		self.plugins
			.iter()
			.filter(|(_, p)| p.plugin_type == PluginType::TrackResolver)
			.map(|(name, plugin)| (name.clone(), plugin.clone()))
			.collect()
	}

	pub async fn resolve_track(&mut self, source: &str, query: &str) -> HostResult<String> {
		let plugin = self
			.plugins
			.get(source)
			.ok_or_else(|| HostError::NotFound(source.to_string()))?;

		if plugin.plugin_type != PluginType::TrackResolver {
			return Err(HostError::CallFailed(format!("Plugin {} is not a track resolver", source)));
		}

		self.host.call_resolve(source, query).await
	}

	pub async fn resolve_all(&mut self, query: &str) -> HostResult<Vec<(String, String)>> {
		let resolvers = self.list_by_type(PluginType::TrackResolver);
		let mut results = Vec::new();

		for name in resolvers {
			match self.host.call_resolve(&name, query).await {
				Ok(result) => results.push((name, result)),
				Err(e) => {
					tracing::warn!("Plugin {} failed to resolve: {}", name, e);
				}
			}
		}

		Ok(results)
	}

	pub async fn filter_frame(&mut self, plugin_name: &str, frame: &mut [i16]) -> HostResult<()> {
		let plugin = self
			.plugins
			.get(plugin_name)
			.ok_or_else(|| HostError::NotFound(plugin_name.to_string()))?;

		if plugin.plugin_type != PluginType::AudioFilter {
			return Err(HostError::CallFailed(format!(
				"Plugin {} is not an audio filter",
				plugin_name
			)));
		}

		self.host.call_filter(plugin_name, frame).await
	}

	pub async fn filter_frames(&mut self, plugin_name: &str, frames: &mut [&mut [i16]]) -> HostResult<()> {
		for frame in frames.iter_mut() {
			self.filter_frame(plugin_name, frame).await?;
		}
		Ok(())
	}

	pub fn get_plugin(&self, name: &str) -> Option<&LoadedPlugin> {
		self.plugins.get(name)
	}

	pub async fn get_plugin_info(&self, name: &str) -> HostResult<LoadedPlugin> {
		self.plugins
			.get(name)
			.cloned()
			.ok_or_else(|| HostError::NotFound(name.to_string()))
	}

	pub fn plugin_count(&self) -> usize {
		self.plugins.len()
	}

	pub fn is_loaded(&self, name: &str) -> bool {
		self.plugins.contains_key(name)
	}

	pub fn get_plugin_dirs(&self) -> &[PathBuf] {
		&self.plugin_dirs
	}

	pub fn is_auto_reload_enabled(&self) -> bool {
		self.auto_reload
	}

	pub fn get_all_plugin_info(&self) -> Vec<crate::manifest::PluginInfo> {
		self.plugins
			.iter()
			.map(|(name, plugin)| {
				let plugin_type = match plugin.plugin_type {
					PluginType::TrackResolver => "track_resolver".to_string(),
					PluginType::AudioFilter => "audio_filter".to_string(),
					PluginType::Route => "route".to_string(),
					PluginType::Unknown => "unknown".to_string(),
				};
				crate::manifest::PluginInfo {
					name: name.clone(),
					version: plugin.version.clone(),
					description: None,
					author: None,
					main_class: name.clone(),
					enabled: true,
					loaded_at: Some(
						std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.unwrap_or_default()
							.as_millis() as i64,
					),
					plugin_type,
					file_path: plugin.path.as_ref().map(|p| p.to_string_lossy().to_string()),
					exports: vec![],
				}
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::host::LoadedPlugin;

	async fn manager_with_memory_plugin() -> PluginManager {
		let mut manager = PluginManager::new().await.expect("manager builds without wasm");

		manager.plugins.insert(
			"memory".to_string(),
			LoadedPlugin {
				name: "memory".to_string(),
				version: "0.1.0".to_string(),
				path: None,
				plugin_type: PluginType::Unknown,
				provides: Vec::new(),
				routes: Vec::new(),
			},
		);

		manager
	}

	#[tokio::test]
	async fn hot_reload_unknown_plugin_returns_not_found() {
		let mut manager = PluginManager::new().await.expect("manager builds without wasm");

		let err = manager.hot_reload("missing", &[0, 1, 2]).await.unwrap_err();

		assert!(matches!(err, HostError::NotFound(_)));
	}

	#[tokio::test]
	async fn hot_reload_from_file_unknown_plugin_returns_not_found() {
		let mut manager = PluginManager::new().await.expect("manager builds without wasm");

		let err = manager.hot_reload_from_file("missing").await.unwrap_err();

		assert!(matches!(err, HostError::NotFound(_)));
	}

	#[tokio::test]
	async fn hot_reload_from_file_without_stored_path_fails() {
		let mut manager = manager_with_memory_plugin().await;

		let err = manager.hot_reload_from_file("memory").await.unwrap_err();

		assert!(matches!(err, HostError::HotReloadFailed(_)));
	}

	#[tokio::test]
	async fn track_resolver_plugins_excludes_other_types() {
		let mut manager = PluginManager::new().await.expect("manager builds without wasm");

		for (key, plugin_type) in [
			("resolver", PluginType::TrackResolver),
			("filter", PluginType::AudioFilter),
			("mystery", PluginType::Unknown),
		] {
			manager.plugins.insert(
				key.to_string(),
				LoadedPlugin {
					name: key.to_string(),
					version: "0.1.0".to_string(),
					path: None,
					plugin_type,
					provides: vec!["ex:".to_string()],
					routes: Vec::new(),
				},
			);
		}

		let resolvers = manager.track_resolver_plugins();

		assert_eq!(resolvers.len(), 1);
		assert_eq!(resolvers[0].0, "resolver");
		assert_eq!(resolvers[0].1.provides, vec!["ex:".to_string()]);
	}
}
