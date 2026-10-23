use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use parking_lot::Mutex;

pub struct Metrics {
	pub players: Arc<AtomicU64>,
	pub playing_players: Arc<AtomicU64>,
	pub uptime_seconds: Arc<AtomicU64>,
	pub memory_used_bytes: Arc<AtomicU64>,
	pub memory_free_bytes: Arc<AtomicU64>,
	pub memory_total_bytes: Arc<AtomicU64>,
	pub system_cpu_load_bits: Arc<AtomicU64>,
	pub process_cpu_load_bits: Arc<AtomicU64>,
	pub tracks_loaded_total: Arc<AtomicU64>,
	pub tracks_played_total: Arc<AtomicU64>,
	pub websocket_connections: Arc<AtomicU64>,
	rest_calls: Arc<Mutex<Vec<RestCall>>>,
	pub resolve_counts: Arc<Mutex<std::collections::BTreeMap<(String, String), u64>>>,
	pub resolve_latency: Arc<Mutex<std::collections::BTreeMap<String, LatencySummary>>>,
	pub queue_depth: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
pub struct RestCall {
	pub method: String,
	pub endpoint: String,
	pub status: u16,
}

const LATENCY_BOUNDS_MS: [u64; 6] = [100, 500, 1000, 2000, 5000, 10000];
const LATENCY_BOUND_LABELS: [&str; 6] = ["0.1", "0.5", "1", "2", "5", "10"];

#[derive(Debug, Clone, Copy, Default)]
pub struct LatencySummary {
	buckets: [u64; 6],
	sum_ms: u64,
	count: u64,
}

impl LatencySummary {
	fn observe(&mut self, elapsed_ms: u64) {
		for (bucket, bound) in self.buckets.iter_mut().zip(LATENCY_BOUNDS_MS) {
			if elapsed_ms <= bound {
				*bucket += 1;
			}
		}

		self.sum_ms += elapsed_ms;
		self.count += 1;
	}
}

impl Metrics {
	pub fn new() -> Self {
		Self {
			players: Arc::new(AtomicU64::new(0)),
			playing_players: Arc::new(AtomicU64::new(0)),
			uptime_seconds: Arc::new(AtomicU64::new(0)),
			memory_used_bytes: Arc::new(AtomicU64::new(0)),
			memory_free_bytes: Arc::new(AtomicU64::new(0)),
			memory_total_bytes: Arc::new(AtomicU64::new(0)),
			system_cpu_load_bits: Arc::new(AtomicU64::new(0.0f64.to_bits())),
			process_cpu_load_bits: Arc::new(AtomicU64::new(0.0f64.to_bits())),
			tracks_loaded_total: Arc::new(AtomicU64::new(0)),
			tracks_played_total: Arc::new(AtomicU64::new(0)),
			websocket_connections: Arc::new(AtomicU64::new(0)),
			rest_calls: Arc::new(Mutex::new(Vec::new())),
			resolve_counts: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
			resolve_latency: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
			queue_depth: Arc::new(AtomicU64::new(0)),
		}
	}

	pub fn encode_metrics(&self) -> String {
		let mut output = String::new();

		output.push_str("# HELP orpheus_players_total Total number of players\n");
		output.push_str("# TYPE orpheus_players_total gauge\n");
		output.push_str(&format!("orpheus_players_total {}\n", self.players.load(Ordering::Relaxed)));

		output.push_str("# HELP orpheus_playing_players Number of currently playing players\n");
		output.push_str("# TYPE orpheus_playing_players gauge\n");
		output.push_str(&format!(
			"orpheus_playing_players {}\n",
			self.playing_players.load(Ordering::Relaxed)
		));

		output.push_str("# HELP orpheus_uptime_seconds Server uptime in seconds\n");
		output.push_str("# TYPE orpheus_uptime_seconds gauge\n");
		output.push_str(&format!(
			"orpheus_uptime_seconds {}\n",
			self.uptime_seconds.load(Ordering::Relaxed)
		));

		output.push_str("# HELP orpheus_memory_used_bytes Used memory in bytes\n");
		output.push_str("# TYPE orpheus_memory_used_bytes gauge\n");
		output.push_str(&format!(
			"orpheus_memory_used_bytes {}\n",
			self.memory_used_bytes.load(Ordering::Relaxed)
		));

		output.push_str("# HELP orpheus_memory_free_bytes Free memory in bytes\n");
		output.push_str("# TYPE orpheus_memory_free_bytes gauge\n");
		output.push_str(&format!(
			"orpheus_memory_free_bytes {}\n",
			self.memory_free_bytes.load(Ordering::Relaxed)
		));

		output.push_str("# HELP orpheus_memory_total_bytes Total memory in bytes\n");
		output.push_str("# TYPE orpheus_memory_total_bytes gauge\n");
		output.push_str(&format!(
			"orpheus_memory_total_bytes {}\n",
			self.memory_total_bytes.load(Ordering::Relaxed)
		));

		output.push_str("# HELP orpheus_system_cpu_load System CPU load as a 0-1 fraction\n");
		output.push_str("# TYPE orpheus_system_cpu_load gauge\n");
		output.push_str(&format!("orpheus_system_cpu_load {}\n", self.system_cpu_load()));

		output.push_str("# HELP orpheus_process_cpu_load Process CPU load as a 0-1 fraction\n");
		output.push_str("# TYPE orpheus_process_cpu_load gauge\n");
		output.push_str(&format!("orpheus_process_cpu_load {}\n", self.process_cpu_load()));

		output.push_str("# HELP orpheus_tracks_loaded_total Total number of tracks loaded\n");
		output.push_str("# TYPE orpheus_tracks_loaded_total counter\n");
		output.push_str(&format!(
			"orpheus_tracks_loaded_total {}\n",
			self.tracks_loaded_total.load(Ordering::Relaxed)
		));

		output.push_str("# HELP orpheus_tracks_played_total Total number of tracks played\n");
		output.push_str("# TYPE orpheus_tracks_played_total counter\n");
		output.push_str(&format!(
			"orpheus_tracks_played_total {}\n",
			self.tracks_played_total.load(Ordering::Relaxed)
		));

		output.push_str("# HELP orpheus_websocket_connections Number of active WebSocket connections\n");
		output.push_str("# TYPE orpheus_websocket_connections gauge\n");
		output.push_str(&format!(
			"orpheus_websocket_connections {}\n",
			self.websocket_connections.load(Ordering::Relaxed)
		));

		output.push_str("# HELP orpheus_rest_calls_total Total number of REST API calls\n");
		output.push_str("# TYPE orpheus_rest_calls_total counter\n");

		let calls = self.rest_calls.lock();
		let mut counts: std::collections::HashMap<(String, String, String), u64> = std::collections::HashMap::new();
		for call in calls.iter() {
			let key = (call.method.clone(), call.endpoint.clone(), call.status.to_string());
			*counts.entry(key).or_insert(0) += 1;
		}
		for ((method, endpoint, status), count) in counts {
			output.push_str(&format!(
				"orpheus_rest_calls_total{{method=\"{}\",endpoint=\"{}\",status=\"{}\"}} {}\n",
				method, endpoint, status, count
			));
		}

		output.push_str("# HELP orpheus_resolve_requests_total Resolve requests by source and result code\n");
		output.push_str("# TYPE orpheus_resolve_requests_total counter\n");

		for ((source, code), count) in self.resolve_counts.lock().iter() {
			output.push_str(&format!(
				"orpheus_resolve_requests_total{{source=\"{}\",code=\"{}\"}} {}\n",
				source, code, count
			));
		}

		output.push_str("# HELP orpheus_resolve_seconds Resolve latency by source\n");
		output.push_str("# TYPE orpheus_resolve_seconds histogram\n");

		for (source, summary) in self.resolve_latency.lock().iter() {
			for (bucket, label) in summary.buckets.iter().zip(LATENCY_BOUND_LABELS) {
				output.push_str(&format!(
					"orpheus_resolve_seconds_bucket{{source=\"{}\",le=\"{}\"}} {}\n",
					source, label, bucket
				));
			}

			output.push_str(&format!(
				"orpheus_resolve_seconds_bucket{{source=\"{}\",le=\"+Inf\"}} {}\n",
				source, summary.count
			));
			output.push_str(&format!(
				"orpheus_resolve_seconds_sum{{source=\"{}\"}} {:.3}\n",
				source,
				summary.sum_ms as f64 / 1000.0
			));
			output.push_str(&format!(
				"orpheus_resolve_seconds_count{{source=\"{}\"}} {}\n",
				source, summary.count
			));
		}

		output.push_str("# HELP orpheus_queue_depth Pending tracks queued across all players\n");
		output.push_str("# TYPE orpheus_queue_depth gauge\n");
		output.push_str(&format!("orpheus_queue_depth {}\n", self.queue_depth.load(Ordering::Relaxed)));

		output
	}

	pub fn increment_rest_call(&self, method: &str, endpoint: &str, status: u16) {
		let mut calls = self.rest_calls.lock();
		calls.push(RestCall {
			method: method.to_string(),
			endpoint: endpoint.to_string(),
			status,
		});
		if calls.len() > 10000 {
			calls.drain(0..5000);
		}
	}

	pub fn set_players(&self, count: usize) {
		self.players.store(count as u64, Ordering::Relaxed);
	}

	pub fn set_playing_players(&self, count: usize) {
		self.playing_players.store(count as u64, Ordering::Relaxed);
	}

	pub fn set_uptime(&self, seconds: u64) {
		self.uptime_seconds.store(seconds, Ordering::Relaxed);
	}

	pub fn set_memory(&self, used: u64, free: u64, total: u64) {
		self.memory_used_bytes.store(used, Ordering::Relaxed);
		self.memory_free_bytes.store(free, Ordering::Relaxed);
		self.memory_total_bytes.store(total, Ordering::Relaxed);
	}

	pub fn set_cpu(&self, system_load: f64, process_load: f64) {
		self.system_cpu_load_bits.store(system_load.to_bits(), Ordering::Relaxed);
		self.process_cpu_load_bits.store(process_load.to_bits(), Ordering::Relaxed);
	}

	pub fn system_cpu_load(&self) -> f64 {
		f64::from_bits(self.system_cpu_load_bits.load(Ordering::Relaxed))
	}

	pub fn process_cpu_load(&self) -> f64 {
		f64::from_bits(self.process_cpu_load_bits.load(Ordering::Relaxed))
	}

	pub fn increment_tracks_loaded(&self, count: u64) {
		self.tracks_loaded_total.fetch_add(count, Ordering::Relaxed);
	}

	pub fn increment_tracks_played(&self, count: u64) {
		self.tracks_played_total.fetch_add(count, Ordering::Relaxed);
	}

	pub fn record_resolve(&self, source: &str, code: &str, elapsed_ms: u64) {
		let mut counts = self.resolve_counts.lock();
		*counts.entry((source.to_string(), code.to_string())).or_insert(0) += 1;

		let mut latency = self.resolve_latency.lock();
		latency.entry(source.to_string()).or_default().observe(elapsed_ms);
	}

	pub fn set_queue_depth(&self, depth: u64) {
		self.queue_depth.store(depth, Ordering::Relaxed);
	}

	pub fn set_websocket_connections(&self, count: usize) {
		self.websocket_connections.store(count as u64, Ordering::Relaxed);
	}
}

impl Default for Metrics {
	fn default() -> Self {
		Self::new()
	}
}

pub struct MetricsState {
	pub metrics: Arc<Metrics>,
	pub start_time: Instant,
	system: Mutex<sysinfo::System>,
}

impl MetricsState {
	pub fn new() -> Self {
		Self {
			metrics: Arc::new(Metrics::new()),
			start_time: Instant::now(),
			system: Mutex::new(sysinfo::System::new()),
		}
	}

	pub fn update_system_metrics(&self) {
		let uptime = self.start_time.elapsed().as_secs();
		self.metrics.set_uptime(uptime);

		let mut sampler = self.system.lock();
		sampler.refresh_memory();
		sampler.refresh_cpu_usage();
		sampler.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

		self.metrics
			.set_memory(sampler.used_memory(), sampler.free_memory(), sampler.total_memory());

		let system_load = (f64::from(sampler.global_cpu_usage()) / 100.0).clamp(0.0, 1.0);

		let process_load = sysinfo::get_current_pid()
			.ok()
			.and_then(|pid| sampler.process(pid))
			.map(|process| (f64::from(process.cpu_usage()) / 100.0).clamp(0.0, 1.0))
			.unwrap_or(0.0);

		self.metrics.set_cpu(system_load, process_load);
	}

	pub fn update_player_metrics(&self, player_count: usize, playing_count: usize) {
		self.metrics.set_players(player_count);
		self.metrics.set_playing_players(playing_count);
	}

	pub fn uptime_ms(&self) -> u64 {
		self.start_time.elapsed().as_millis() as u64
	}

	pub fn snapshot(&self) -> SystemSnapshot {
		self.update_system_metrics();

		let metrics = &self.metrics;

		SystemSnapshot {
			uptime_ms: self.uptime_ms(),
			memory_used: metrics.memory_used_bytes.load(Ordering::Relaxed),
			memory_free: metrics.memory_free_bytes.load(Ordering::Relaxed),
			memory_total: metrics.memory_total_bytes.load(Ordering::Relaxed),
			system_load: metrics.system_cpu_load(),
			process_load: metrics.process_cpu_load(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct SystemSnapshot {
	pub uptime_ms: u64,
	pub memory_used: u64,
	pub memory_free: u64,
	pub memory_total: u64,
	pub system_load: f64,
	pub process_load: f64,
}

impl Default for MetricsState {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn resolve_counts_and_latency_encode_as_prometheus() {
		let state = MetricsState::new();

		state.metrics.record_resolve("youtube", "OK", 200);
		state.metrics.record_resolve("youtube", "OK", 1500);
		state.metrics.record_resolve("spotify", "SOURCE_UNAVAILABLE", 50);
		state.metrics.set_queue_depth(7);

		let output = state.metrics.encode_metrics();

		assert!(output.contains("orpheus_resolve_requests_total{source=\"spotify\",code=\"SOURCE_UNAVAILABLE\"} 1\n"));
		assert!(output.contains("orpheus_resolve_requests_total{source=\"youtube\",code=\"OK\"} 2\n"));
		assert!(output.contains("orpheus_resolve_seconds_bucket{source=\"youtube\",le=\"0.5\"} 1\n"));
		assert!(output.contains("orpheus_resolve_seconds_bucket{source=\"youtube\",le=\"2\"} 2\n"));
		assert!(output.contains("orpheus_resolve_seconds_bucket{source=\"youtube\",le=\"+Inf\"} 2\n"));
		assert!(output.contains("orpheus_resolve_seconds_sum{source=\"youtube\"} 1.700\n"));
		assert!(output.contains("orpheus_resolve_seconds_count{source=\"youtube\"} 2\n"));
		assert!(output.contains("orpheus_queue_depth 7\n"));
	}

	#[test]
	fn snapshot_reports_host_memory_and_bounded_loads() {
		let state = MetricsState::new();

		let first = state.snapshot();
		let second = state.snapshot();

		for snapshot in [&first, &second] {
			assert!(snapshot.memory_total > 0);
			assert!(snapshot.memory_used <= snapshot.memory_total);
			assert!((0.0..=1.0).contains(&snapshot.system_load));
			assert!((0.0..=1.0).contains(&snapshot.process_load));
		}

		assert!(second.uptime_ms < 60_000);
	}
}
