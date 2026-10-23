use std::collections::VecDeque;
use std::time::{Duration, Instant};

use dashmap::DashMap;

pub const MAX_PARKED_EVENTS: usize = 256;

#[derive(Clone, Debug)]
pub struct StoredSession {
	pub id: String,
	pub user_id: u64,
	pub resumable: bool,
	pub timeout_secs: u64,
	paused: bool,
	parked: VecDeque<String>,
	expires_at: Option<Instant>,
}

impl StoredSession {
	pub fn is_paused(&self) -> bool {
		self.paused
	}
}

#[derive(Debug, PartialEq)]
pub enum CloseOutcome {
	Parked,
	Destroyed,
	Unknown,
}

#[derive(Debug)]
pub enum Reconnect {
	Resumed {
		session: StoredSession,
		parked: Vec<String>,
	},
	Live(StoredSession),
	Gone,
}

fn expired(session: &StoredSession) -> bool {
	session.paused && session.expires_at.is_some_and(|deadline| deadline <= Instant::now())
}

pub struct SessionStore {
	sessions: DashMap<String, StoredSession>,
	affinity: DashMap<u64, String>,
}

impl SessionStore {
	pub fn new() -> Self {
		Self {
			sessions: DashMap::new(),
			affinity: DashMap::new(),
		}
	}

	pub fn create(&self, user_id: u64) -> StoredSession {
		let session = StoredSession {
			id: format!("{:032x}", rand::random::<u128>()),
			user_id,
			resumable: false,
			timeout_secs: 60,
			paused: false,
			parked: VecDeque::new(),
			expires_at: None,
		};

		self.sessions.insert(session.id.clone(), session.clone());

		session
	}

	pub fn get(&self, session_id: &str) -> Option<StoredSession> {
		self.sessions.get(session_id).map(|entry| entry.clone())
	}

	pub fn touch(&self, session_id: &str, guild_id: u64) {
		self.affinity.insert(guild_id, session_id.to_string());
	}

	pub fn delivers_to(&self, session_id: &str, guild_id: u64) -> bool {
		self.affinity.get(&guild_id).is_none_or(|owner| owner.as_str() == session_id)
	}

	pub fn update(&self, session_id: &str, resuming: Option<bool>, timeout_secs: Option<i64>) -> Option<(bool, u64)> {
		let mut entry = self.sessions.get_mut(session_id)?;

		if let Some(resuming) = resuming {
			entry.resumable = resuming;
		}

		if let Some(timeout) = timeout_secs
			&& timeout >= 0
		{
			entry.timeout_secs = timeout as u64;
		}

		Some((entry.resumable, entry.timeout_secs))
	}

	pub fn reconnect(&self, session_id: &str, user_id: u64) -> Reconnect {
		let Some(mut entry) = self.sessions.get_mut(session_id) else {
			return Reconnect::Gone;
		};

		if entry.paused {
			if expired(&entry) {
				drop(entry);
				self.sessions.remove(session_id);

				return Reconnect::Gone;
			}

			entry.paused = false;
			entry.expires_at = None;

			let parked = entry.parked.drain(..).collect();
			let session = entry.clone();

			return Reconnect::Resumed { session, parked };
		}

		if entry.user_id == user_id {
			return Reconnect::Live(entry.clone());
		}

		Reconnect::Gone
	}

	pub fn on_close(&self, session_id: &str) -> CloseOutcome {
		let Some(mut entry) = self.sessions.get_mut(session_id) else {
			return CloseOutcome::Unknown;
		};

		if !entry.resumable {
			drop(entry);
			self.sessions.remove(session_id);

			return CloseOutcome::Destroyed;
		}

		entry.paused = true;
		entry.expires_at = Some(Instant::now() + Duration::from_secs(entry.timeout_secs));

		CloseOutcome::Parked
	}

	pub fn park(&self, session_id: &str, guild_id: u64, payload: String) {
		if !self.delivers_to(session_id, guild_id) {
			return;
		}

		let Some(mut entry) = self.sessions.get_mut(session_id) else {
			return;
		};

		if !entry.paused {
			return;
		}

		if entry.parked.len() >= MAX_PARKED_EVENTS {
			entry.parked.pop_front();
		}

		entry.parked.push_back(payload);
	}

	pub fn park_owned(&self, guild_id: u64, payload: String) {
		if let Some(owner) = self.affinity.get(&guild_id).map(|entry| entry.clone()) {
			self.park(&owner, guild_id, payload);

			return;
		}

		for mut entry in self.sessions.iter_mut() {
			if !entry.paused {
				continue;
			}

			if entry.parked.len() >= MAX_PARKED_EVENTS {
				entry.parked.pop_front();
			}

			entry.parked.push_back(payload.clone());
		}
	}

	pub fn reap(&self) -> Vec<String> {
		let mut reaped = Vec::new();

		self.sessions.retain(|id, session| {
			if expired(session) {
				reaped.push(id.clone());

				return false;
			}

			true
		});

		reaped
	}
}

impl Default for SessionStore {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn resume_before_timeout_replays_parked_events_in_order() {
		let store = SessionStore::new();
		let created = store.create(7);

		store.update(&created.id, Some(true), Some(60));
		store.touch(&created.id, 7);

		assert_eq!(store.on_close(&created.id), CloseOutcome::Parked);

		store.park(&created.id, 7, "first".to_string());
		store.park(&created.id, 7, "second".to_string());

		let Reconnect::Resumed { session, parked } = store.reconnect(&created.id, 7) else {
			panic!("expected resume");
		};

		assert_eq!(parked, vec!["first".to_string(), "second".to_string()]);
		assert!(!session.is_paused());

		assert!(matches!(store.reconnect(&created.id, 7), Reconnect::Live(_)));
	}

	#[test]
	fn expiry_removes_the_session_for_destruction() {
		let store = SessionStore::new();
		let created = store.create(7);

		store.update(&created.id, Some(true), Some(0));

		assert_eq!(store.on_close(&created.id), CloseOutcome::Parked);

		let reaped = store.reap();

		assert_eq!(reaped, vec![created.id.clone()]);
		assert!(store.get(&created.id).is_none());
		assert!(matches!(store.reconnect(&created.id, 7), Reconnect::Gone));
	}

	#[test]
	fn non_resumable_close_destroys_immediately() {
		let store = SessionStore::new();
		let created = store.create(7);

		assert_eq!(store.on_close(&created.id), CloseOutcome::Destroyed);
		assert!(store.get(&created.id).is_none());
	}

	#[test]
	fn close_of_unknown_session_is_unknown() {
		let store = SessionStore::new();

		assert_eq!(store.on_close("missing"), CloseOutcome::Unknown);
		assert!(matches!(store.reconnect("missing", 7), Reconnect::Gone));
	}

	#[test]
	fn session_update_changes_resuming_and_timeout() {
		let store = SessionStore::new();
		let created = store.create(7);

		assert_eq!(store.update(&created.id, Some(true), Some(120)), Some((true, 120)));
		assert_eq!(store.update(&created.id, None, Some(30)), Some((true, 30)));
		assert_eq!(store.update(&created.id, Some(false), None), Some((false, 30)));
		assert_eq!(store.update(&created.id, None, Some(-5)), Some((false, 30)));
		assert_eq!(store.update("missing", Some(true), None), None);
	}

	#[test]
	fn parked_queue_is_bounded_and_keeps_the_newest() {
		let store = SessionStore::new();
		let created = store.create(7);

		store.update(&created.id, Some(true), Some(60));
		store.on_close(&created.id);

		for index in 0..MAX_PARKED_EVENTS + 10 {
			store.park(&created.id, 7, index.to_string());
		}

		let Reconnect::Resumed { parked, .. } = store.reconnect(&created.id, 7) else {
			panic!("expected resume");
		};

		assert_eq!(parked.len(), MAX_PARKED_EVENTS);
		assert_eq!(parked.first().unwrap(), "10");
		assert_eq!(parked.last().unwrap(), &(MAX_PARKED_EVENTS + 9).to_string());
	}

	#[test]
	fn affinity_takeover_moves_delivery_to_last_writer() {
		let store = SessionStore::new();
		let first = store.create(7);
		let second = store.create(7);

		store.touch(&first.id, 7);

		assert!(store.delivers_to(&first.id, 7));
		assert!(!store.delivers_to(&second.id, 7));

		store.touch(&second.id, 7);

		assert!(!store.delivers_to(&first.id, 7));
		assert!(store.delivers_to(&second.id, 7));
	}

	#[test]
	fn unknown_guild_delivers_to_every_session() {
		let store = SessionStore::new();
		let first = store.create(7);
		let second = store.create(8);

		assert!(store.delivers_to(&first.id, 9));
		assert!(store.delivers_to(&second.id, 9));
	}

	#[test]
	fn park_only_lands_in_the_owning_paused_session() {
		let store = SessionStore::new();
		let first = store.create(7);
		let second = store.create(8);

		store.update(&first.id, Some(true), Some(60));
		store.update(&second.id, Some(true), Some(60));
		store.touch(&first.id, 7);
		store.touch(&second.id, 8);
		store.on_close(&first.id);
		store.on_close(&second.id);

		store.park(&first.id, 7, "seven".to_string());
		store.park(&second.id, 7, "stolen".to_string());
		store.park(&second.id, 9, "nine".to_string());

		let Reconnect::Resumed {
			parked: first_parked, ..
		} = store.reconnect(&first.id, 7)
		else {
			panic!("expected resume");
		};

		assert_eq!(first_parked, vec!["seven".to_string()]);

		let Reconnect::Resumed {
			parked: second_parked, ..
		} = store.reconnect(&second.id, 8)
		else {
			panic!("expected resume");
		};

		assert_eq!(second_parked, vec!["nine".to_string()]);
	}

	#[test]
	fn park_skips_live_sessions() {
		let store = SessionStore::new();
		let created = store.create(7);

		store.touch(&created.id, 7);
		store.park(&created.id, 7, "live".to_string());

		let Reconnect::Live(_) = store.reconnect(&created.id, 7) else {
			panic!("expected live session");
		};
	}

	#[test]
	fn park_owned_routes_to_affinity_owner() {
		let store = SessionStore::new();
		let first = store.create(7);
		let second = store.create(8);

		store.update(&first.id, Some(true), Some(60));
		store.update(&second.id, Some(true), Some(60));
		store.touch(&first.id, 7);
		store.on_close(&first.id);
		store.on_close(&second.id);

		store.park_owned(7, "seven".to_string());

		let Reconnect::Resumed {
			parked: first_parked, ..
		} = store.reconnect(&first.id, 7)
		else {
			panic!("expected resume");
		};

		assert_eq!(first_parked, vec!["seven".to_string()]);

		let Reconnect::Resumed {
			parked: second_parked, ..
		} = store.reconnect(&second.id, 8)
		else {
			panic!("expected resume");
		};

		assert!(second_parked.is_empty());
	}

	#[test]
	fn park_owned_fails_open_without_affinity() {
		let store = SessionStore::new();
		let first = store.create(7);
		let second = store.create(8);

		store.update(&first.id, Some(true), Some(60));
		store.update(&second.id, Some(true), Some(60));
		store.on_close(&first.id);
		store.on_close(&second.id);

		store.park_owned(9, "nine".to_string());

		for (id, user) in [(&first.id, 7), (&second.id, 8)] {
			let Reconnect::Resumed { parked, .. } = store.reconnect(id, user) else {
				panic!("expected resume");
			};

			assert_eq!(parked, vec!["nine".to_string()]);
		}
	}
}
