use std::collections::{BinaryHeap, HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

pub trait AbstractIpBlocker: Send + Sync {
	fn ip_blocked(&self, ip: &IpAddr) -> bool;
	fn block_ip(&mut self, ip: IpAddr);
	fn unblock_ip(&mut self, ip: &IpAddr) -> bool;
	fn unblock_all(&mut self);
	fn get_blocked_count(&self) -> usize;
	fn get_details(&self) -> serde_json::Value;
	fn get_status(&self) -> RoutePlannerStatus;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedAddress {
	pub failing_address: String,
	pub failing_timestamp: i64,
	pub failing_time: Option<String>,
}

fn failing_entry(address: &str) -> FailedAddress {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_millis() as i64;

	FailedAddress {
		failing_address: address.to_string(),
		failing_timestamp: now,
		failing_time: None,
	}
}

fn ip_block_details(address_count: usize, failing: serde_json::Value) -> serde_json::Value {
	serde_json::json!({
		"ipBlock": {
			"type": "Inet4Address",
			"size": address_count.to_string(),
		},
		"failingAddresses": failing,
	})
}

#[derive(Debug)]
pub struct IpBlocker {
	blocked_ips: HashSet<IpAddr>,
}

impl IpBlocker {
	pub fn new() -> Self {
		Self {
			blocked_ips: HashSet::new(),
		}
	}
}

impl AbstractIpBlocker for IpBlocker {
	fn ip_blocked(&self, ip: &IpAddr) -> bool {
		self.blocked_ips.contains(ip)
	}

	fn block_ip(&mut self, ip: IpAddr) {
		self.blocked_ips.insert(ip);
	}

	fn unblock_ip(&mut self, ip: &IpAddr) -> bool {
		self.blocked_ips.remove(ip)
	}

	fn unblock_all(&mut self) {
		self.blocked_ips.clear();
	}

	fn get_blocked_count(&self) -> usize {
		self.blocked_ips.len()
	}

	fn get_details(&self) -> serde_json::Value {
		serde_json::json!({
			"blocked": self.blocked_ips.len(),
			"addresses": self.blocked_ips.iter().map(|ip| ip.to_string()).collect::<Vec<_>>()
		})
	}

	fn get_status(&self) -> RoutePlannerStatus {
		RoutePlannerStatus {
			class_name: "IpBlocker".to_string(),
			details: self.get_details(),
		}
	}
}

impl Default for IpBlocker {
	fn default() -> Self {
		Self::new()
	}
}

#[derive(Debug)]
pub struct RotatingIpRoutePlanner {
	current_index: usize,
	addresses: Vec<IpAddr>,
	failing_addresses: HashMap<String, FailedAddress>,
	ip_blocker: IpBlocker,
}

impl RotatingIpRoutePlanner {
	pub fn new(addresses: Vec<IpAddr>) -> Self {
		Self {
			current_index: 0,
			addresses,
			failing_addresses: HashMap::new(),
			ip_blocker: IpBlocker::new(),
		}
	}

	pub fn get_next_address(&mut self) -> Option<IpAddr> {
		let start = self.current_index;
		loop {
			if self.addresses.is_empty() {
				return None;
			}

			let addr = self.addresses[self.current_index];
			self.current_index = (self.current_index + 1) % self.addresses.len();

			if !self.ip_blocker.ip_blocked(&addr) {
				return Some(addr);
			}

			if self.current_index == start {
				return None;
			}
		}
	}

	pub fn report_failure(&mut self, ip: IpAddr) {
		self.failing_addresses.insert(ip.to_string(), failing_entry(&ip.to_string()));
		self.ip_blocker.block_ip(ip);
	}

	pub fn free_address(&mut self, address: &str) -> bool {
		if let Ok(ip) = address.parse::<IpAddr>() {
			self.ip_blocker.unblock_ip(&ip);
			self.failing_addresses.remove(address);
			return true;
		}
		false
	}

	pub fn free_all(&mut self) {
		self.ip_blocker.unblock_all();
		self.failing_addresses.clear();
	}

	fn current_address(&self) -> String {
		if self.addresses.is_empty() {
			return "0.0.0.0".to_string();
		}

		self.addresses[self.current_index % self.addresses.len()].to_string()
	}

	pub fn get_status(&self) -> RoutePlannerStatus {
		RoutePlannerStatus {
			class_name: "RotatingIpRoutePlanner".to_string(),
			details: self.status_details(),
		}
	}

	fn status_details(&self) -> serde_json::Value {
		let mut details = ip_block_details(
			self.addresses.len(),
			serde_json::to_value(self.failing_addresses.values().collect::<Vec<_>>()).unwrap_or_default(),
		);

		details["rotateIndex"] = serde_json::json!(self.current_index.to_string());
		details["ipIndex"] = serde_json::json!(self.current_index.to_string());
		details["currentAddress"] = serde_json::json!(self.current_address());

		details
	}
}

impl AbstractIpBlocker for RotatingIpRoutePlanner {
	fn ip_blocked(&self, ip: &IpAddr) -> bool {
		self.ip_blocker.ip_blocked(ip)
	}

	fn block_ip(&mut self, ip: IpAddr) {
		self.ip_blocker.block_ip(ip);
	}

	fn unblock_ip(&mut self, ip: &IpAddr) -> bool {
		self.ip_blocker.unblock_ip(ip)
	}

	fn unblock_all(&mut self) {
		self.ip_blocker.unblock_all();
	}

	fn get_blocked_count(&self) -> usize {
		self.ip_blocker.get_blocked_count()
	}

	fn get_details(&self) -> serde_json::Value {
		RotatingIpRoutePlanner::get_status(self).details
	}

	fn get_status(&self) -> RoutePlannerStatus {
		RotatingIpRoutePlanner::get_status(self)
	}
}

impl Default for RotatingIpRoutePlanner {
	fn default() -> Self {
		Self::new(vec![])
	}
}

#[derive(Debug)]
pub struct NanoIpRoutePlanner {
	ip_blocker: IpBlocker,
	failing_addresses: HashMap<String, FailedAddress>,
	current_ip: u64,
	block_size: u64,
}

impl NanoIpRoutePlanner {
	pub fn new(block_size: u64) -> Self {
		Self {
			ip_blocker: IpBlocker::new(),
			failing_addresses: HashMap::new(),
			current_ip: 0,
			block_size: block_size.max(1),
		}
	}

	pub fn get_next_address(&mut self) -> Option<IpAddr> {
		let start = self.current_ip;
		loop {
			let ip = IpAddr::V4(std::net::Ipv4Addr::new(
				((self.current_ip >> 24) & 0xFF) as u8,
				((self.current_ip >> 16) & 0xFF) as u8,
				((self.current_ip >> 8) & 0xFF) as u8,
				(self.current_ip & 0xFF) as u8,
			));
			self.current_ip = (self.current_ip + 1) % (256 * 256);

			if !self.ip_blocker.ip_blocked(&ip) {
				return Some(ip);
			}

			if self.current_ip == start {
				return None;
			}
		}
	}

	pub fn report_failure(&mut self, ip: IpAddr) {
		self.failing_addresses.insert(ip.to_string(), failing_entry(&ip.to_string()));
		self.ip_blocker.block_ip(ip);
	}

	pub fn free_address(&mut self, address: &str) -> bool {
		if let Ok(ip) = address.parse::<IpAddr>() {
			self.ip_blocker.unblock_ip(&ip);
			self.failing_addresses.remove(address);
			return true;
		}
		false
	}

	pub fn free_all(&mut self) {
		self.ip_blocker.unblock_all();
		self.failing_addresses.clear();
	}
}

impl AbstractIpBlocker for NanoIpRoutePlanner {
	fn ip_blocked(&self, ip: &IpAddr) -> bool {
		self.ip_blocker.ip_blocked(ip)
	}

	fn block_ip(&mut self, ip: IpAddr) {
		self.ip_blocker.block_ip(ip);
	}

	fn unblock_ip(&mut self, ip: &IpAddr) -> bool {
		self.ip_blocker.unblock_ip(ip)
	}

	fn unblock_all(&mut self) {
		self.ip_blocker.unblock_all();
	}

	fn get_blocked_count(&self) -> usize {
		self.ip_blocker.get_blocked_count()
	}

	fn get_details(&self) -> serde_json::Value {
		let mut details = ip_block_details(
			self.block_size as usize,
			serde_json::to_value(self.failing_addresses.values().collect::<Vec<_>>()).unwrap_or_default(),
		);

		details["currentAddressIndex"] = serde_json::json!(self.current_ip.to_string());

		details
	}

	fn get_status(&self) -> RoutePlannerStatus {
		RoutePlannerStatus {
			class_name: "NanoIpRoutePlanner".to_string(),
			details: self.get_details(),
		}
	}
}

impl Default for NanoIpRoutePlanner {
	fn default() -> Self {
		Self::new(128)
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedAddressWithPriority {
	pub failing_address: String,
	pub failing_timestamp: i64,
	pub failing_time: Option<String>,
	#[serde(default, skip_serializing)]
	pub priority: i64,
}

impl PartialEq for FailedAddressWithPriority {
	fn eq(&self, other: &Self) -> bool {
		self.priority == other.priority
	}
}

impl Eq for FailedAddressWithPriority {}

impl PartialOrd for FailedAddressWithPriority {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for FailedAddressWithPriority {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.priority.cmp(&other.priority).reverse()
	}
}

pub struct BalancingIpRoutePlanner {
	addresses: Vec<String>,
	current_index: usize,
	failing_addresses: BinaryHeap<FailedAddressWithPriority>,
	ip_blocker: IpBlocker,
}

impl BalancingIpRoutePlanner {
	pub fn new(addresses: Vec<String>) -> Self {
		Self {
			addresses,
			current_index: 0,
			failing_addresses: BinaryHeap::new(),
			ip_blocker: IpBlocker::new(),
		}
	}

	pub fn get_next_address(&mut self) -> Option<String> {
		if self.addresses.is_empty() {
			return None;
		}

		let start = self.current_index;
		loop {
			let addr = self.addresses[self.current_index].clone();
			self.current_index = (self.current_index + 1) % self.addresses.len();

			if !self
				.ip_blocker
				.ip_blocked(&addr.parse().unwrap_or_else(|_| "0.0.0.0".parse().unwrap()))
			{
				return Some(addr);
			}

			if self.current_index == start {
				return None;
			}
		}
	}

	pub fn report_failure(&mut self, address: String) {
		let entry = failing_entry(&address);

		self.failing_addresses.push(FailedAddressWithPriority {
			failing_address: entry.failing_address,
			failing_timestamp: entry.failing_timestamp,
			failing_time: entry.failing_time,
			priority: entry.failing_timestamp,
		});

		if let Ok(ip) = address.parse::<IpAddr>() {
			self.ip_blocker.block_ip(ip);
		}
	}

	pub fn free_address(&mut self, address: &str) -> bool {
		if let Ok(ip) = address.parse::<IpAddr>() {
			self.ip_blocker.unblock_ip(&ip);
		}

		self.failing_addresses.retain(|f| f.failing_address != address);
		true
	}

	pub fn free_all(&mut self) {
		self.ip_blocker.unblock_all();
		self.failing_addresses.clear();
	}
}

impl AbstractIpBlocker for BalancingIpRoutePlanner {
	fn ip_blocked(&self, ip: &IpAddr) -> bool {
		self.ip_blocker.ip_blocked(ip)
	}

	fn block_ip(&mut self, ip: IpAddr) {
		self.ip_blocker.block_ip(ip);
	}

	fn unblock_ip(&mut self, ip: &IpAddr) -> bool {
		self.ip_blocker.unblock_ip(ip)
	}

	fn unblock_all(&mut self) {
		self.ip_blocker.unblock_all();
	}

	fn get_blocked_count(&self) -> usize {
		self.ip_blocker.get_blocked_count()
	}

	fn get_details(&self) -> serde_json::Value {
		ip_block_details(
			self.addresses.len(),
			serde_json::to_value(self.failing_addresses.iter().collect::<Vec<_>>()).unwrap_or_default(),
		)
	}

	fn get_status(&self) -> RoutePlannerStatus {
		RoutePlannerStatus {
			class_name: "BalancingIpRoutePlanner".to_string(),
			details: self.get_details(),
		}
	}
}

impl Default for BalancingIpRoutePlanner {
	fn default() -> Self {
		Self::new(vec![])
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutePlannerStatus {
	#[serde(rename = "class")]
	pub class_name: String,
	pub details: serde_json::Value,
}

pub struct RoutePlannerManager {
	planner: Arc<RwLock<Option<Box<dyn AbstractIpBlocker + Send>>>>,
}

impl RoutePlannerManager {
	pub fn new() -> Self {
		Self {
			planner: Arc::new(RwLock::new(None)),
		}
	}

	fn with_planner(planner: Box<dyn AbstractIpBlocker + Send>) -> Self {
		Self {
			planner: Arc::new(RwLock::new(Some(planner))),
		}
	}

	pub async fn with_rotating(addresses: Vec<IpAddr>) -> Self {
		Self::with_planner(Box::new(RotatingIpRoutePlanner::new(addresses)))
	}

	pub async fn with_nano_ip(block_size: u64) -> Self {
		Self::with_planner(Box::new(NanoIpRoutePlanner::new(block_size)))
	}

	pub async fn with_balancing(addresses: Vec<String>) -> Self {
		Self::with_planner(Box::new(BalancingIpRoutePlanner::new(addresses)))
	}

	pub async fn is_enabled(&self) -> bool {
		self.planner.read().await.is_some()
	}

	pub async fn ip_blocked(&self, ip: &IpAddr) -> bool {
		let planner = self.planner.read().await;

		planner.as_ref().is_some_and(|planner| planner.ip_blocked(ip))
	}

	pub async fn block_ip(&self, ip: IpAddr) {
		if let Some(planner) = self.planner.write().await.as_mut() {
			planner.block_ip(ip);
		}
	}

	pub async fn unblock_ip(&self, ip: &IpAddr) -> Option<bool> {
		self.planner.write().await.as_mut().map(|planner| planner.unblock_ip(ip))
	}

	pub async fn unblock_all(&self) -> bool {
		match self.planner.write().await.as_mut() {
			Some(planner) => {
				planner.unblock_all();

				true
			}
			None => false,
		}
	}

	pub async fn get_status(&self) -> Option<RoutePlannerStatus> {
		let planner = self.planner.read().await;

		planner.as_ref().map(|planner| planner.get_status())
	}
}

impl Default for RoutePlannerManager {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn planner_classes_match_lavalink_names() {
		let rotating = RotatingIpRoutePlanner::new(vec!["1.2.3.4".parse().unwrap()]);
		let nano = NanoIpRoutePlanner::new(64);
		let balancing = BalancingIpRoutePlanner::new(vec!["https://example.invalid".to_string()]);

		assert_eq!(rotating.get_status().class_name, "RotatingIpRoutePlanner");
		assert_eq!(nano.get_status().class_name, "NanoIpRoutePlanner");
		assert_eq!(balancing.get_status().class_name, "BalancingIpRoutePlanner");
	}

	#[test]
	fn rotating_details_carry_lavalink_keys() {
		let planner = RotatingIpRoutePlanner::new(vec!["1.2.3.4".parse().unwrap()]);
		let details = planner.get_status().details;

		assert_eq!(details["ipBlock"]["type"], "Inet4Address");
		assert!(details["ipBlock"]["size"].is_string());
		assert!(details["failingAddresses"].is_array());
		assert!(details["rotateIndex"].is_string());
		assert!(details["ipIndex"].is_string());
		assert_eq!(details["currentAddress"], "1.2.3.4");
	}

	#[test]
	fn nano_details_carry_current_address_index() {
		let planner = NanoIpRoutePlanner::new(64);
		let details = planner.get_status().details;

		assert_eq!(details["ipBlock"]["type"], "Inet4Address");
		assert_eq!(details["ipBlock"]["size"], "64");
		assert!(details["failingAddresses"].is_array());
		assert_eq!(details["currentAddressIndex"], "0");
	}

	#[test]
	fn balancing_details_have_no_planner_specific_keys() {
		let planner = BalancingIpRoutePlanner::new(vec!["https://example.invalid".to_string()]);
		let details = planner.get_status().details;

		assert_eq!(details["ipBlock"]["type"], "Inet4Address");
		assert!(details["failingAddresses"].is_array());
		assert!(details.get("rotateIndex").is_none());
		assert!(details.get("currentAddressIndex").is_none());
	}

	#[test]
	fn failing_addresses_use_lavalink_keys() {
		let mut planner = RotatingIpRoutePlanner::new(vec!["1.2.3.4".parse().unwrap()]);
		planner.report_failure("1.2.3.4".parse().unwrap());

		let details = planner.get_status().details;
		let failing = details["failingAddresses"].as_array().expect("array");

		assert_eq!(failing.len(), 1);
		assert_eq!(failing[0]["failingAddress"], "1.2.3.4");
		assert!(failing[0]["failingTimestamp"].is_number());
		assert!(failing[0].get("failingTime").is_some());
	}

	#[tokio::test]
	async fn disabled_planner_reports_no_status() {
		let manager = RoutePlannerManager::new();

		assert!(!(manager.is_enabled().await));
		assert!(manager.get_status().await.is_none());
		assert!(manager.unblock_ip(&"1.2.3.4".parse().unwrap()).await.is_none());
		assert!(!(manager.unblock_all().await));
		assert!(!(manager.ip_blocked(&"1.2.3.4".parse().unwrap()).await));
	}

	#[tokio::test]
	async fn configured_planner_reports_status_and_frees() {
		let manager = RoutePlannerManager::with_rotating(vec!["1.2.3.4".parse().unwrap()]).await;

		assert!(manager.is_enabled().await);

		let status = manager.get_status().await.expect("status");

		assert_eq!(status.class_name, "RotatingIpRoutePlanner");

		manager.block_ip("1.2.3.4".parse().unwrap()).await;

		assert!(manager.ip_blocked(&"1.2.3.4".parse().unwrap()).await);
		assert_eq!(manager.unblock_ip(&"1.2.3.4".parse().unwrap()).await, Some(true));
		assert!(manager.unblock_all().await);
	}
}
