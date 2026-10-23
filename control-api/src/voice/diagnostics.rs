use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use parking_lot::Mutex;
use serde::Serialize;

const MAX_GUILDS: usize = 512;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceDiagnosticKind {
	ConnectStarted,
	Connected,
	ConnectFailed,
	ReconnectStarted,
	Disconnected,
	DaveEpoch,
	StreamStarted,
	StreamEnded,
	StreamFailed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDiagnostic {
	pub timestamp_ms: u64,
	pub kind: VoiceDiagnosticKind,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub endpoint: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub dave_version: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub dave_epoch: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub detail: Option<String>,
}

pub struct VoiceDiagnostics {
	entries: DashMap<u64, Mutex<VecDeque<VoiceDiagnostic>>>,
	capacity: AtomicUsize,
}

impl VoiceDiagnostics {
	pub fn new(capacity: usize) -> Self {
		Self {
			entries: DashMap::new(),
			capacity: AtomicUsize::new(capacity),
		}
	}

	pub fn set_capacity(&self, capacity: usize) {
		self.capacity.store(capacity, Ordering::Relaxed);

		if capacity == 0 {
			self.entries.clear();
			return;
		}

		for entry in &self.entries {
			let mut events = entry.lock();
			while events.len() > capacity {
				events.pop_front();
			}
		}
	}

	pub fn record(
		&self,
		guild_id: u64,
		kind: VoiceDiagnosticKind,
		endpoint: Option<String>,
		dave_version: Option<u8>,
		dave_epoch: Option<u64>,
		detail: Option<String>,
	) {
		let capacity = self.capacity.load(Ordering::Relaxed);
		if capacity == 0 {
			return;
		}

		if !self.entries.contains_key(&guild_id)
			&& self.entries.len() >= MAX_GUILDS
			&& let Some(oldest) = self
				.entries
				.iter()
				.min_by_key(|entry| entry.value().lock().front().map(|event| event.timestamp_ms).unwrap_or(0))
				.map(|entry| *entry.key())
		{
			self.entries.remove(&oldest);
		}

		let entry = self.entries.entry(guild_id).or_default();
		let mut events = entry.lock();
		while events.len() >= capacity {
			events.pop_front();
		}
		events.push_back(VoiceDiagnostic {
			timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
			kind,
			endpoint,
			dave_version,
			dave_epoch,
			detail,
		});
	}

	pub fn for_guild(&self, guild_id: u64) -> Vec<VoiceDiagnostic> {
		self.entries
			.get(&guild_id)
			.map(|entry| entry.lock().iter().cloned().collect())
			.unwrap_or_default()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn diagnostic_buffer_retains_only_the_most_recent_events() {
		let diagnostics = VoiceDiagnostics::new(2);
		for detail in ["first", "second", "third"] {
			diagnostics.record(7, VoiceDiagnosticKind::Connected, None, None, None, Some(detail.to_string()));
		}

		let events = diagnostics.for_guild(7);
		let details: Vec<_> = events.iter().filter_map(|event| event.detail.as_deref()).collect();

		assert_eq!(details, ["second", "third"]);
	}
}
