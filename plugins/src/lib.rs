mod bindings;
mod fetch;
pub mod host;
pub mod kv;
pub mod manager;
pub mod manifest;
pub mod plugin;

pub use host::{HostError, HostResult, LoadedPlugin, PluginHost, PluginType, ResourceLimits, RouteDecl, RouteResponse};
pub use manager::PluginManager;
pub use manifest::{PluginInfo, PluginManifest, ResourceLimitsConfig};
pub use plugin::{AudioFilterPlugin, Plugin, PluginError, PluginResult, TrackInfo, TrackResolverPlugin};

pub const PLUGIN_API_VERSION: &str = "0.2.0";
