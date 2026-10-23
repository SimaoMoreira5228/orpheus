use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
	pub name: String,
	pub version: String,
	pub description: Option<String>,
	pub author: Option<String>,
	pub main_class: String,
	#[serde(default)]
	pub dependencies: Vec<String>,
	#[serde(default)]
	pub provides: Vec<String>,
	#[serde(default)]
	pub api_version: Option<String>,
	#[serde(default)]
	pub resource_limits: Option<ResourceLimitsConfig>,
	#[serde(default)]
	pub required_exports: Vec<String>,
	#[serde(default)]
	pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimitsConfig {
	#[serde(default = "default_memory_pages")]
	pub max_memory_pages: u32,
	#[serde(default = "default_execution_time_ms")]
	pub max_execution_time_ms: u64,
	#[serde(default = "default_call_depth")]
	pub max_call_depth: u32,
}

fn default_memory_pages() -> u32 {
	256
}
fn default_execution_time_ms() -> u64 {
	5000
}
fn default_call_depth() -> u32 {
	32
}

impl Default for ResourceLimitsConfig {
	fn default() -> Self {
		Self {
			max_memory_pages: default_memory_pages(),
			max_execution_time_ms: default_execution_time_ms(),
			max_call_depth: default_call_depth(),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
	pub name: String,
	pub version: String,
	pub description: Option<String>,
	pub author: Option<String>,
	pub main_class: String,
	pub enabled: bool,
	pub loaded_at: Option<i64>,
	pub plugin_type: String,
	pub file_path: Option<String>,
	pub exports: Vec<String>,
}

impl PluginManifest {
	pub fn validate(&self) -> Result<(), String> {
		if self.name.is_empty() {
			return Err("Plugin name cannot be empty".to_string());
		}
		if self.version.is_empty() {
			return Err("Plugin version cannot be empty".to_string());
		}
		if self.main_class.is_empty() {
			return Err("Plugin main class cannot be empty".to_string());
		}

		for export in &self.required_exports {
			if export.is_empty() {
				return Err("Required export cannot be empty".to_string());
			}
		}

		Ok(())
	}

	pub fn to_info(&self, enabled: bool, loaded_at: Option<i64>, plugin_type: String, exports: Vec<String>) -> PluginInfo {
		PluginInfo {
			name: self.name.clone(),
			version: self.version.clone(),
			description: self.description.clone(),
			author: self.author.clone(),
			main_class: self.main_class.clone(),
			enabled,
			loaded_at,
			plugin_type,
			file_path: None,
			exports,
		}
	}

	pub fn from_file(path: &PathBuf) -> Result<Self, String> {
		let content = std::fs::read_to_string(path).map_err(|e| format!("Failed to read manifest: {}", e))?;

		serde_json::from_str(&content).map_err(|e| format!("Failed to parse manifest: {}", e))
	}
}

impl Default for PluginManifest {
	fn default() -> Self {
		Self {
			name: "unknown".to_string(),
			version: "0.0.0".to_string(),
			description: None,
			author: None,
			main_class: String::new(),
			dependencies: Vec::new(),
			provides: Vec::new(),
			api_version: None,
			resource_limits: None,
			required_exports: Vec::new(),
			metadata: HashMap::new(),
		}
	}
}
