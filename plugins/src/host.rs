use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use crate::bindings::OrpheusPlugin;
use crate::bindings::orpheus::plugin::fetch::Host as GuestFetchImports;
use crate::bindings::orpheus::plugin::host::Host as GuestHostImports;
use crate::bindings::orpheus::plugin::kv::Host as GuestKvImports;
use crate::fetch::{FetchCtx, fetch_inner};
use crate::kv::KvStore;

const DEFAULT_MAX_EXECUTION_TIME_MS: u64 = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginType {
	TrackResolver,
	AudioFilter,
	Route,
	Unknown,
}

#[derive(Debug, Error)]
pub enum HostError {
	#[error("WASM runtime error: {0}")]
	Wasmtime(String),
	#[error("Plugin not found: {0}")]
	NotFound(String),
	#[error("Call failed: {0}")]
	CallFailed(String),
	#[error("Memory error: {0}")]
	Memory(String),
	#[error("WASM module error: {0}")]
	Module(String),
	#[error("Invalid track data: {0}")]
	InvalidTrackData(String),
	#[error("Resource limit exceeded: {0}")]
	ResourceLimitExceeded(String),
	#[error("Plugin panic: {0}")]
	PluginPanic(String),
	#[error("Hot reload failed: {0}")]
	HotReloadFailed(String),
	#[error("WASI error: {0}")]
	Wasi(String),
	#[error("Invalid exports: {0}")]
	InvalidExports(String),
	#[error("Component error: {0}")]
	Component(String),
}

pub type HostResult<T> = Result<T, HostError>;

#[derive(Debug, Clone)]
pub struct ResourceLimits {
	pub max_memory_pages: u32,
	pub max_execution_time_ms: u64,
	pub max_call_depth: u32,
	pub max_table_size: u32,
}

impl Default for ResourceLimits {
	fn default() -> Self {
		Self {
			max_memory_pages: 256,
			max_execution_time_ms: DEFAULT_MAX_EXECUTION_TIME_MS,
			max_call_depth: 32,
			max_table_size: 0,
		}
	}
}

impl ResourceLimits {
	fn effective_timeout(&self) -> Duration {
		if self.max_execution_time_ms == 0 {
			Duration::from_millis(DEFAULT_MAX_EXECUTION_TIME_MS)
		} else {
			Duration::from_millis(self.max_execution_time_ms)
		}
	}
}

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
	pub name: String,
	pub version: String,
	pub path: Option<PathBuf>,
	pub plugin_type: PluginType,
	pub provides: Vec<String>,
	pub routes: Vec<RouteDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RouteDecl {
	pub method: String,
	pub path: String,
}

impl RouteDecl {
	pub fn matches(&self, method: &str, path: &str) -> bool {
		self.method.eq_ignore_ascii_case(method) && self.path == path
	}
}

#[derive(Debug, Clone)]
pub struct RouteResponse {
	pub status: u16,
	pub headers: Vec<(String, String)>,
	pub body: Vec<u8>,
}

#[derive(Debug, serde::Deserialize)]
struct GuestTrackInfo {
	name: Option<String>,
	version: Option<String>,
	api_version: Option<String>,
	provides: Option<Vec<String>>,
	routes: Option<Vec<RouteDecl>>,
}

fn check_guest_api_version(guest: &GuestTrackInfo, plugin_name: &str) -> HostResult<()> {
	if guest.api_version.as_deref() == Some(crate::PLUGIN_API_VERSION) {
		return Ok(());
	}

	Err(HostError::Component(format!(
		"plugin API mismatch: guest {plugin_name} reports API version {:?}, host requires {}; rebuild the guest against plugins/wit/plugin.wit",
		guest.api_version,
		crate::PLUGIN_API_VERSION
	)))
}

pub struct PluginState {
	table: ResourceTable,
	ctx: WasiCtx,
	plugin_name: String,
	configs: Arc<RwLock<HashMap<String, serde_json::Value>>>,
	fetch_client: reqwest::Client,
	timeout_ms: u64,
	kv: KvStore,
}

impl PluginState {
	pub fn new() -> Self {
		Self::for_plugin(
			String::new(),
			Arc::new(RwLock::new(HashMap::new())),
			reqwest::Client::new(),
			DEFAULT_MAX_EXECUTION_TIME_MS,
			KvStore::new(),
		)
	}

	pub fn for_plugin(
		plugin_name: String,
		configs: Arc<RwLock<HashMap<String, serde_json::Value>>>,
		fetch_client: reqwest::Client,
		timeout_ms: u64,
		kv: KvStore,
	) -> Self {
		let table = ResourceTable::new();
		let ctx = WasiCtxBuilder::new().build();
		Self {
			table,
			ctx,
			plugin_name,
			configs,
			fetch_client,
			timeout_ms,
			kv,
		}
	}

	async fn fetch_ctx(&mut self) -> FetchCtx {
		FetchCtx::new(
			self.plugin_name.clone(),
			self.configs.clone(),
			self.fetch_client.clone(),
			self.timeout_ms,
		)
	}
}

impl Default for PluginState {
	fn default() -> Self {
		Self::new()
	}
}

impl WasiView for PluginState {
	fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
		wasmtime_wasi::WasiCtxView {
			ctx: &mut self.ctx,
			table: &mut self.table,
		}
	}
}

impl GuestHostImports for PluginState {
	async fn log(&mut self, msg: String) -> wasmtime::Result<()> {
		tracing::info!(target: "orpheus_plugin", "{msg}");
		Ok(())
	}

	async fn get_api_version(&mut self) -> wasmtime::Result<String> {
		Ok(crate::PLUGIN_API_VERSION.to_string())
	}
}

impl GuestFetchImports for PluginState {
	async fn fetch(&mut self, url: String, opts: String) -> wasmtime::Result<Result<String, String>> {
		Ok(fetch_inner(self.fetch_ctx().await, &url, &opts).await)
	}
}

impl GuestKvImports for PluginState {
	async fn kv_get(&mut self, key: String) -> wasmtime::Result<Option<String>> {
		let plugin = self.plugin_name.clone();

		Ok(self.kv.get(&plugin, &key).await)
	}

	async fn kv_set(&mut self, key: String, value: String) -> wasmtime::Result<()> {
		let plugin = self.plugin_name.clone();

		self.kv.set(&plugin, &key, &value).await;

		Ok(())
	}

	async fn kv_delete(&mut self, key: String) -> wasmtime::Result<()> {
		let plugin = self.plugin_name.clone();

		self.kv.delete(&plugin, &key).await;

		Ok(())
	}
}

struct LivePlugin {
	store: Store<PluginState>,
	bindings: OrpheusPlugin,
	has_configure: bool,
	has_handle_route: bool,
}

struct PluginSlot {
	info: LoadedPlugin,
	live: Mutex<LivePlugin>,
}

pub struct PluginHost {
	engine: Engine,
	limits: ResourceLimits,
	plugins: Arc<RwLock<HashMap<String, Arc<PluginSlot>>>>,
	configs: Arc<RwLock<HashMap<String, serde_json::Value>>>,
	fetch_client: reqwest::Client,
	kv: KvStore,
}

impl std::fmt::Debug for PluginHost {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PluginHost").finish_non_exhaustive()
	}
}

impl PluginHost {
	pub fn new() -> HostResult<Self> {
		Self::new_with_limits(ResourceLimits::default())
	}

	pub fn new_with_limits(limits: ResourceLimits) -> HostResult<Self> {
		let mut config = wasmtime::Config::new();
		config.consume_fuel(true);

		let engine = Engine::new(&config).map_err(|e| HostError::Wasmtime(e.to_string()))?;

		Ok(Self {
			engine,
			limits,
			plugins: Arc::new(RwLock::new(HashMap::new())),
			configs: Arc::new(RwLock::new(HashMap::new())),
			fetch_client: reqwest::Client::new(),
			kv: KvStore::new(),
		})
	}

	fn detect_type(component: &Component) -> PluginType {
		if component.get_export(None, "resolve").is_some() {
			return PluginType::TrackResolver;
		}

		if component.get_export(None, "filter").is_some() {
			return PluginType::AudioFilter;
		}

		PluginType::Unknown
	}

	async fn call_with_timeout<F, T>(&self, call: F) -> HostResult<T>
	where
		F: Future<Output = wasmtime::Result<T>>,
	{
		let timeout = self.limits.effective_timeout();

		match tokio::time::timeout(timeout, call).await {
			Ok(Ok(value)) => Ok(value),
			Ok(Err(e)) => Err(HostError::CallFailed(e.to_string())),
			Err(_) => Err(HostError::CallFailed(format!(
				"plugin call timed out after {}ms",
				timeout.as_millis()
			))),
		}
	}

	async fn slot_for(&self, plugin_name: &str) -> HostResult<Arc<PluginSlot>> {
		let slot = self
			.plugins
			.read()
			.await
			.get(plugin_name)
			.cloned()
			.ok_or_else(|| HostError::NotFound(plugin_name.to_string()))?;

		Ok(slot)
	}

	pub async fn load_plugin(&self, wasm_bytes: &[u8], name: &str) -> HostResult<LoadedPlugin> {
		self.load_plugin_inner(wasm_bytes, name, None).await
	}

	pub async fn load_plugin_from_file(&self, path: &PathBuf, name: &str) -> HostResult<LoadedPlugin> {
		let wasm_bytes = std::fs::read(path).map_err(|e| HostError::Module(format!("Failed to read file: {}", e)))?;

		self.load_plugin_inner(&wasm_bytes, name, Some(path.clone())).await
	}

	async fn load_plugin_inner(&self, wasm_bytes: &[u8], name: &str, path: Option<PathBuf>) -> HostResult<LoadedPlugin> {
		let component = Component::new(&self.engine, wasm_bytes).map_err(|e| HostError::Component(e.to_string()))?;

		let plugin_type = Self::detect_type(&component);

		let has_configure = component.get_export(None, "configure").is_some();
		let has_handle_route = component.get_export(None, "handle-route").is_some();

		let mut store = Store::new(
			&self.engine,
			PluginState::for_plugin(
				name.to_string(),
				self.configs.clone(),
				self.fetch_client.clone(),
				self.limits.effective_timeout().as_millis() as u64,
				self.kv.clone(),
			),
		);
		store.set_fuel(u64::MAX).map_err(|e| HostError::Wasmtime(e.to_string()))?;

		let mut linker = Linker::new(&self.engine);

		wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|e| HostError::Wasi(e.to_string()))?;

		OrpheusPlugin::add_to_linker::<_, HasSelf<PluginState>>(&mut linker, |state: &mut PluginState| state)
			.map_err(|e| HostError::Component(format!("failed to link host imports: {e}")))?;

		let bindings = OrpheusPlugin::instantiate_async(&mut store, &component, &linker)
			.await
			.map_err(|e| {
				HostError::Component(format!(
					"failed to instantiate plugin (host API v{}): {e}",
					crate::PLUGIN_API_VERSION
				))
			})?;

		let mut live = LivePlugin {
			store,
			bindings,
			has_configure,
			has_handle_route,
		};

		{
			let LivePlugin { store, bindings, .. } = &mut live;
			self.call_with_timeout(bindings.call_init(store)).await?;
		}

		let mut info = LoadedPlugin {
			name: name.to_string(),
			version: crate::PLUGIN_API_VERSION.to_string(),
			path,
			plugin_type,
			provides: Vec::new(),
			routes: Vec::new(),
		};

		let metadata = self
			.call_with_timeout({
				let LivePlugin { store, bindings, .. } = &mut live;
				bindings.call_track_info(store)
			})
			.await?;

		let guest = serde_json::from_str::<GuestTrackInfo>(&metadata)
			.map_err(|e| HostError::Component(format!("plugin {name} returned unparseable track-info metadata: {e}")))?;

		check_guest_api_version(&guest, name)?;

		info.provides = guest.provides.clone().unwrap_or_default();
		info.routes = guest.routes.clone().unwrap_or_default();

		if info.plugin_type == PluginType::Unknown && !info.routes.is_empty() {
			info.plugin_type = PluginType::Route;
		}

		if let Some(guest_name) = guest.name.filter(|n| !n.is_empty()) {
			info.name = guest_name;
		}

		if let Some(guest_version) = guest.version.filter(|v| !v.is_empty()) {
			info.version = guest_version;
		}

		self.plugins.write().await.insert(
			name.to_string(),
			Arc::new(PluginSlot {
				info: info.clone(),
				live: Mutex::new(live),
			}),
		);

		Ok(info)
	}

	pub async fn call_resolve(&self, plugin_name: &str, query: &str) -> HostResult<String> {
		let slot = self.slot_for(plugin_name).await?;
		let mut live = slot.live.lock().await;
		let LivePlugin { store, bindings, .. } = &mut *live;

		store.set_fuel(u64::MAX).map_err(|e| HostError::Wasmtime(e.to_string()))?;

		self.call_with_timeout(bindings.call_resolve(store, query)).await
	}

	pub async fn call_filter(&self, plugin_name: &str, frame: &mut [i16]) -> HostResult<()> {
		let slot = self.slot_for(plugin_name).await?;
		let mut live = slot.live.lock().await;
		let LivePlugin { store, bindings, .. } = &mut *live;

		store.set_fuel(u64::MAX).map_err(|e| HostError::Wasmtime(e.to_string()))?;

		// NOTE: the guest cannot see host memory; frame-ptr/len are an ABI placeholder
		// until the P4 PCM tap exists. The reference guest ignores both arguments.
		let frame_len = frame.len() as u32;
		self.call_with_timeout(bindings.call_filter(store, 0, frame_len)).await
	}

	pub async fn call_track_info(&self, plugin_name: &str) -> HostResult<String> {
		let slot = self.slot_for(plugin_name).await?;
		let mut live = slot.live.lock().await;
		let LivePlugin { store, bindings, .. } = &mut *live;

		store.set_fuel(u64::MAX).map_err(|e| HostError::Wasmtime(e.to_string()))?;

		self.call_with_timeout(bindings.call_track_info(store)).await
	}

	pub async fn set_plugin_config(&self, plugin_name: &str, config: serde_json::Value) -> HostResult<()> {
		self.configs.write().await.insert(plugin_name.to_string(), config.clone());

		let config_json = serde_json::to_string(&config)
			.map_err(|e| HostError::CallFailed(format!("config is not JSON-serializable: {e}")))?;

		self.call_configure(plugin_name, &config_json).await
	}

	pub async fn plugin_config(&self, plugin_name: &str) -> Option<serde_json::Value> {
		self.configs.read().await.get(plugin_name).cloned()
	}

	pub async fn call_configure(&self, plugin_name: &str, config_json: &str) -> HostResult<()> {
		let slot = self.slot_for(plugin_name).await?;
		let mut live = slot.live.lock().await;

		if !live.has_configure {
			return Ok(());
		}

		let LivePlugin { store, bindings, .. } = &mut *live;

		store.set_fuel(u64::MAX).map_err(|e| HostError::Wasmtime(e.to_string()))?;

		self.call_with_timeout(bindings.call_configure(store, config_json)).await
	}

	pub async fn call_route(
		&self,
		plugin_name: &str,
		method: &str,
		path: &str,
		query: &str,
		headers_json: &str,
		body_b64: &str,
	) -> HostResult<RouteResponse> {
		let slot = self.slot_for(plugin_name).await?;
		let mut live = slot.live.lock().await;

		if !live.has_handle_route {
			return Err(HostError::NotFound(format!("plugin {plugin_name} exports no route handler")));
		}

		let LivePlugin { store, bindings, .. } = &mut *live;

		store.set_fuel(u64::MAX).map_err(|e| HostError::Wasmtime(e.to_string()))?;

		let raw = self
			.call_with_timeout(bindings.call_handle_route(store, method, path, query, headers_json, body_b64))
			.await?
			.map_err(HostError::CallFailed)?;

		#[derive(serde::Deserialize)]
		struct GuestRouteResponse {
			status: u16,
			headers: Option<Vec<(String, String)>>,
			body_b64: Option<String>,
		}

		let parsed: GuestRouteResponse = serde_json::from_str(&raw)
			.map_err(|e| HostError::CallFailed(format!("plugin {plugin_name} returned unparseable route response: {e}")))?;

		let body = match parsed.body_b64 {
			Some(encoded) => base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &encoded)
				.map_err(|e| HostError::CallFailed(format!("plugin {plugin_name} returned invalid body_b64: {e}")))?,
			None => Vec::new(),
		};

		Ok(RouteResponse {
			status: parsed.status,
			headers: parsed.headers.unwrap_or_default(),
			body,
		})
	}

	pub async fn capabilities(&self, plugin_name: &str) -> HostResult<Vec<String>> {
		let slot = self.slot_for(plugin_name).await?;
		let live = slot.live.lock().await;

		let mut capabilities = match slot.info.plugin_type {
			PluginType::TrackResolver => vec!["resolve".to_string()],
			PluginType::AudioFilter => vec!["filter".to_string()],
			PluginType::Route => vec!["handle-route".to_string()],
			PluginType::Unknown => Vec::new(),
		};

		if live.has_configure {
			capabilities.push("configure".to_string());
		}

		let allow_hosts: Vec<String> = self
			.configs
			.read()
			.await
			.get(plugin_name)
			.and_then(|config| config.get("allow_hosts"))
			.and_then(|hosts| serde_json::from_value::<Vec<String>>(hosts.clone()).ok())
			.unwrap_or_default();

		if !allow_hosts.is_empty() {
			capabilities.push("fetch".to_string());
		}

		if live.has_handle_route && !capabilities.iter().any(|c| c == "handle-route") {
			capabilities.push("handle-route".to_string());
		}

		Ok(capabilities)
	}

	pub async fn unload_plugin(&self, name: &str) -> HostResult<()> {
		let slot = self.plugins.write().await.remove(name);

		let Some(slot) = slot else {
			return Ok(());
		};

		let mut live = slot.live.lock().await;
		let LivePlugin { store, bindings, .. } = &mut *live;

		if let Err(e) = self.call_with_timeout(bindings.call_shutdown(store)).await {
			tracing::warn!("plugin {name} shutdown failed: {e}");
		}

		Ok(())
	}

	pub async fn list_plugins(&self) -> Vec<String> {
		self.plugins.read().await.keys().cloned().collect()
	}

	pub async fn plugin_info(&self, name: &str) -> HostResult<LoadedPlugin> {
		Ok(self.slot_for(name).await?.info.clone())
	}
}

impl Default for PluginHost {
	fn default() -> Self {
		Self::new().expect("Failed to create default PluginHost")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn zero_timeout_falls_back_to_default() {
		let limits = ResourceLimits {
			max_execution_time_ms: 0,
			..ResourceLimits::default()
		};

		assert_eq!(
			limits.effective_timeout(),
			Duration::from_millis(DEFAULT_MAX_EXECUTION_TIME_MS)
		);
	}

	#[tokio::test]
	async fn timed_out_call_maps_to_call_failed() {
		let host = PluginHost::new_with_limits(ResourceLimits {
			max_execution_time_ms: 1,
			..ResourceLimits::default()
		})
		.expect("host builds without wasm");

		let err = host
			.call_with_timeout(std::future::pending::<wasmtime::Result<String>>())
			.await
			.unwrap_err();

		assert!(matches!(err, HostError::CallFailed(_)));
	}

	#[tokio::test]
	async fn guest_trap_maps_to_call_failed() {
		let host = PluginHost::new().expect("host builds without wasm");

		let err = host
			.call_with_timeout(async { Err::<String, _>(wasmtime::Error::msg("boom")) })
			.await
			.unwrap_err();

		assert!(matches!(err, HostError::CallFailed(_)));
	}

	#[test]
	fn matching_guest_api_version_passes() {
		let guest = GuestTrackInfo {
			name: Some("example-plugin".to_string()),
			version: Some("0.1.0".to_string()),
			api_version: Some(crate::PLUGIN_API_VERSION.to_string()),
			provides: Some(vec!["example:".to_string()]),
			routes: None,
		};

		assert!(check_guest_api_version(&guest, "example").is_ok());
	}

	#[test]
	fn missing_guest_api_version_is_mismatch() {
		let guest = GuestTrackInfo {
			name: Some("stale-plugin".to_string()),
			version: Some("0.1.0".to_string()),
			api_version: None,
			provides: None,
			routes: None,
		};

		let err = check_guest_api_version(&guest, "stale").unwrap_err();

		assert!(matches!(err, HostError::Component(_)));
		assert!(err.to_string().contains(crate::PLUGIN_API_VERSION));
	}

	#[test]
	fn wrong_guest_api_version_is_mismatch() {
		let guest = GuestTrackInfo {
			name: Some("future-plugin".to_string()),
			version: Some("9.9.9".to_string()),
			api_version: Some("9.9.9".to_string()),
			provides: None,
			routes: None,
		};

		let err = check_guest_api_version(&guest, "future").unwrap_err();

		assert!(matches!(err, HostError::Component(_)));
		assert!(err.to_string().contains("9.9.9"));
	}
}
