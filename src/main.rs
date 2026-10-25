use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use actix_web::middleware::from_fn;
use actix_web::web::Data;
use actix_web::{App, HttpServer};
use dotenvy::dotenv;
use orpheus_control_api::runtime_config::{RuntimeConfig, RuntimeConfigStore};
use orpheus_control_api::state::AppState;
use orpheus_control_api::websocket::WebSocketState;
use orpheus_plugins::PluginManager;
use orpheus_track_resolver::TrackResolver;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[derive(Deserialize, Debug)]
#[serde(default)]
struct Config {
	#[serde(default = "default_host")]
	host: String,
	#[serde(default = "default_http_port")]
	http_port: u16,
	#[serde(default)]
	runtime: RuntimeConfig,
	#[serde(default = "default_restart_state_path")]
	restart_state_path: String,
	plugins: PluginConfig,
}

#[derive(Deserialize, Debug)]
struct PluginConfig {
	#[serde(default)]
	enabled: bool,
	#[serde(default = "default_plugin_dir")]
	dir: String,
	#[serde(flatten)]
	per_plugin: std::collections::HashMap<String, toml::Value>,
}

impl Default for PluginConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			dir: default_plugin_dir(),
			per_plugin: std::collections::HashMap::new(),
		}
	}
}

fn default_plugin_dir() -> String {
	"./plugins".to_string()
}

fn default_host() -> String {
	"0.0.0.0".to_string()
}

fn default_http_port() -> u16 {
	2333
}

fn default_restart_state_path() -> String {
	"./data/restart-state.json".to_string()
}

impl Default for Config {
	fn default() -> Self {
		Self {
			host: default_host(),
			http_port: default_http_port(),
			runtime: RuntimeConfig::default(),
			restart_state_path: default_restart_state_path(),
			plugins: PluginConfig::default(),
		}
	}
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	dotenv().ok();

	if rustls::crypto::ring::default_provider().install_default().is_err() {
		tracing::warn!("rustls default CryptoProvider already installed");
	}

	let config = load_config()?;

	let (filter_layer, filter_handle) = tracing_subscriber::reload::Layer::new(EnvFilter::new(&config.runtime.log_level));
	tracing_subscriber::registry()
		.with(filter_layer)
		.with(tracing_subscriber::fmt::layer())
		.init();

	info!("Starting Orpheus server on {}:{}", config.host, config.http_port);

	let addr: SocketAddr = format!("{}:{}", config.host, config.http_port)
		.parse()
		.expect("Invalid address");

	let mut track_resolver = TrackResolver::new().await;

	let plugin_manager = Arc::new(RwLock::new(
		PluginManager::new().await.expect("Failed to create plugin manager"),
	));

	if config.plugins.enabled {
		let plugin_dir = PathBuf::from(&config.plugins.dir);
		if plugin_dir.exists() {
			info!("Loading plugins from: {}", plugin_dir.display());
			let mut manager = plugin_manager.write().await;
			manager.set_plugin_dirs(vec![plugin_dir]);
			match manager.load_all_from_dirs().await {
				Ok(loaded) => {
					info!("Loaded {} plugins: {:?}", loaded.len(), loaded);
				}
				Err(e) => {
					tracing::warn!("Failed to load plugins: {}", e);
				}
			}

			for (name, table) in &config.plugins.per_plugin {
				match serde_json::to_value(table) {
					Ok(json) => {
						if let Err(e) = manager.set_plugin_config(name, json).await {
							tracing::warn!("Failed to apply config for plugin {name}: {e}");
						}
					}
					Err(e) => {
						tracing::warn!("Plugin {name} config is not JSON-serializable: {e}");
					}
				}
			}
		} else {
			info!("Plugin directory does not exist: {}", plugin_dir.display());
		}
	}

	track_resolver.register_plugin_source(plugin_manager.clone());
	let runtime_config = Arc::new(
		RuntimeConfigStore::load(
			PathBuf::from("./runtime.toml"),
			config.runtime,
			track_resolver.configurable_source_names(),
		)
		.await
		.map_err(anyhow::Error::msg)?,
	);
	let current_runtime_config = runtime_config.current();
	track_resolver
		.configure_runtime(
			current_runtime_config.source_order.clone(),
			current_runtime_config.default_source_timeout_ms,
			current_runtime_config.source_timeouts_ms.clone(),
		)
		.map_err(anyhow::Error::msg)?;
	if let Err(error) = filter_handle.reload(EnvFilter::new(current_runtime_config.log_level.clone())) {
		tracing::warn!("Failed to apply runtime log filter: {}", error);
	}
	let mut runtime_updates = runtime_config.subscribe();
	tokio::spawn(async move {
		while runtime_updates.changed().await.is_ok() {
			let level = runtime_updates.borrow_and_update().log_level.clone();
			if let Err(error) = filter_handle.reload(EnvFilter::new(level)) {
				tracing::warn!("Failed to apply runtime log filter: {}", error);
			}
		}
	});

	let track_resolver = Arc::new(track_resolver);
	let state = Arc::new(AppState::new(track_resolver.clone(), plugin_manager.clone(), runtime_config));

	let data = Data::new(state.clone());

	let ws_password = std::env::var("ORPHEUS_PASSWORD").unwrap_or_else(|_| "youshallnotpass".to_string());
	let password_data = Data::new(orpheus_control_api::auth::Password(ws_password.clone()));
	let ws_state = Arc::new(WebSocketState::new(
		state.player_manager.clone(),
		ws_password,
		track_resolver,
		state.voice_connection_manager.clone(),
		state.metrics.clone(),
		state.prefetch.clone(),
	));
	let ws_data = actix_web::web::Data::new(ws_state);
	let restart_store = orpheus_control_api::restart::RestartSnapshotStore::new(PathBuf::from(&config.restart_state_path));
	let restored = restart_store.restore(&state, ws_data.get_ref().as_ref()).await?;
	if restored > 0 {
		info!("Restored {} players from the graceful-restart snapshot", restored);
	}

	info!("Orpheus server listening on http://{}", addr);

	let server = HttpServer::new(move || {
		App::new()
			.app_data(data.clone())
			.app_data(ws_data.clone())
			.app_data(password_data.clone())
			.wrap(from_fn(orpheus_control_api::auth::require_auth))
			.configure(orpheus_control_api::rest::configure)
			.configure(orpheus_control_api::health::configure)
			.configure(orpheus_control_api::websocket::configure)
	})
	.shutdown_timeout(5)
	.disable_signals()
	.bind(addr)?
	.run();
	let server_handle = server.handle();
	let mut server_task = tokio::spawn(server);

	let (server_result, snapshot_result) = tokio::select! {
		result = &mut server_task => {
			let server_result = result
				.map_err(anyhow::Error::from)
				.and_then(|result| result.map_err(anyhow::Error::from));
			let snapshot = restart_store
				.save(&state.player_manager, &state.voice_connection_manager)
				.await;
			(server_result, snapshot)
		},
		_ = shutdown_signal() => {
			info!("Graceful shutdown requested");
			info!("Pausing HTTP listener");
			server_handle.pause().await;
			info!("Saving restart snapshot");
			let snapshot = restart_store
				.save(&state.player_manager, &state.voice_connection_manager)
				.await;
			if let Err(error) = &snapshot {
				tracing::error!("Failed to save restart snapshot: {error}");
			}
			info!("Stopping HTTP server");
			server_handle.stop(true).await;
			let server_result = server_task
				.await
				.map_err(anyhow::Error::from)
				.and_then(|result| result.map_err(anyhow::Error::from));
			info!("HTTP server stopped");
			(server_result, snapshot)
		}
	};

	info!("Shutting down voice connections");
	state.voice_connection_manager.shutdown_all().await;
	info!("Voice connections shut down");

	server_result?;
	let saved = snapshot_result?;
	info!("Saved {} players for restart", saved);
	Ok(())
}

async fn shutdown_signal() {
	#[cfg(unix)]
	{
		use tokio::signal::unix::{SignalKind, signal};

		let mut terminate = signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
		tokio::select! {
			_ = tokio::signal::ctrl_c() => {},
			_ = terminate.recv() => {},
		}
	}

	#[cfg(not(unix))]
	{
		let _ = tokio::signal::ctrl_c().await;
	}
}

fn load_config() -> anyhow::Result<Config> {
	dotenv().ok();

	let content = match std::fs::read_to_string("config.toml") {
		Ok(content) => content,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
		Err(error) => return Err(error.into()),
	};

	toml::from_str(&content).map_err(Into::into)
}
