use std::collections::BTreeMap;

use orpheus_core::Filters;
use parking_lot::RwLock;

pub struct FilterPresetStore {
	presets: RwLock<BTreeMap<String, Filters>>,
}

impl FilterPresetStore {
	pub fn new() -> Self {
		Self {
			presets: RwLock::new(BTreeMap::new()),
		}
	}

	pub fn put(&self, name: String, filters: Filters) {
		self.presets.write().insert(name, filters);
	}

	pub fn get(&self, name: &str) -> Option<Filters> {
		self.presets.read().get(name).cloned()
	}

	pub fn remove(&self, name: &str) -> bool {
		self.presets.write().remove(name).is_some()
	}

	pub fn list(&self) -> BTreeMap<String, Filters> {
		self.presets.read().clone()
	}
}

impl Default for FilterPresetStore {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn bass_boost() -> Filters {
		Filters {
			equalizer: Some(vec![orpheus_core::EqualizerBand { band: 0, gain: 0.8 }]),
			..Filters::default()
		}
	}

	#[test]
	fn put_get_remove_round_trip() {
		let store = FilterPresetStore::new();

		assert!(store.get("bass").is_none());

		store.put("bass".to_string(), bass_boost());

		assert_eq!(store.get("bass").unwrap().equalizer.as_ref().unwrap()[0].gain, 0.8);

		assert!(store.remove("bass"));
		assert!(!store.remove("bass"));
		assert!(store.get("bass").is_none());
	}

	#[test]
	fn put_overwrites_and_list_reports_everything() {
		let store = FilterPresetStore::new();

		store.put("a".to_string(), bass_boost());
		store.put("b".to_string(), Filters::default());
		store.put("a".to_string(), Filters::default());

		let listed = store.list();

		assert_eq!(listed.len(), 2);
		assert!(listed["a"].equalizer.is_none());
		assert!(listed.contains_key("b"));
	}
}
