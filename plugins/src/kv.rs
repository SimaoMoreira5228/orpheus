use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

#[derive(Debug, Default, Clone)]
pub struct KvStore {
	inner: Arc<RwLock<HashMap<(String, String), String>>>,
}

impl KvStore {
	pub fn new() -> Self {
		Self {
			inner: Arc::new(RwLock::new(HashMap::new())),
		}
	}

	pub async fn get(&self, plugin: &str, key: &str) -> Option<String> {
		let guard = self.inner.read().await;

		guard.get(&(plugin.to_string(), key.to_string())).cloned()
	}

	pub async fn set(&self, plugin: &str, key: &str, value: &str) {
		let mut guard = self.inner.write().await;

		guard.insert((plugin.to_string(), key.to_string()), value.to_string());
	}

	pub async fn delete(&self, plugin: &str, key: &str) {
		let mut guard = self.inner.write().await;

		guard.remove(&(plugin.to_string(), key.to_string()));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn set_then_get_round_trip() {
		let store = KvStore::new();

		store.set("example", "theme", "dark").await;

		assert_eq!(store.get("example", "theme").await.as_deref(), Some("dark"));
	}

	#[tokio::test]
	async fn identical_keys_are_isolated_between_plugins() {
		let store = KvStore::new();

		store.set("alpha", "theme", "dark").await;
		store.set("beta", "theme", "light").await;

		assert_eq!(store.get("alpha", "theme").await.as_deref(), Some("dark"));
		assert_eq!(store.get("beta", "theme").await.as_deref(), Some("light"));
	}
}
