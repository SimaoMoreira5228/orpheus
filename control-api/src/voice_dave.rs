use std::num::NonZeroU16;

use davey::ProposalsOperationType;
use discordrs::voice_runtime::{VoiceDaveySession, VoiceRuntimeHandle};
use tracing::{debug, warn};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FailureKey {
	ExternalSender {
		len: usize,
	},
	Proposals {
		index: usize,
		len: usize,
	},
	Commit {
		transition_id: u64,
		len: usize,
	},
	Welcome {
		transition_id: u64,
		len: usize,
	},
}

pub struct DaveController {
	session: Option<VoiceDaveySession>,
	version: u16,
	user_id: u64,
	channel_id: u64,
	joined_epoch: Option<u64>,
	external_sender_applied: bool,
	consumed_proposals: usize,
	last_commit: Option<(u64, usize)>,
	last_welcome: Option<(u64, usize)>,
	key_package_sent: bool,
	passthrough: bool,
	last_failure: Option<FailureKey>,
}

impl DaveController {
	pub fn new(user_id: u64, channel_id: u64) -> Self {
		Self {
			session: None,
			version: 0,
			user_id,
			channel_id,
			joined_epoch: None,
			external_sender_applied: false,
			consumed_proposals: 0,
			last_commit: None,
			last_welcome: None,
			key_package_sent: false,
			passthrough: false,
			last_failure: None,
		}
	}

	pub fn is_ready(&self) -> bool {
		self.session.as_ref().map(|s| s.is_ready()).unwrap_or(false)
	}

	pub fn joined_epoch(&self) -> Option<u64> {
		self.joined_epoch
	}

	pub fn passthrough(&self) -> bool {
		self.passthrough
	}

	pub fn session_mut(&mut self) -> Option<&mut VoiceDaveySession> {
		self.session.as_mut()
	}

	fn note_success(&mut self) {
		self.last_failure = None;
	}

	fn note_failure(&mut self, key: FailureKey) -> bool {
		let repeated = self.last_failure == Some(key);
		self.last_failure = Some(key);
		!repeated
	}

	fn fresh_session(&mut self, version: u16) -> Result<(), discordrs::DiscordError> {
		let Some(protocol_version) = NonZeroU16::new(version) else {
			return Ok(());
		};
		let session = VoiceDaveySession::new(protocol_version, self.user_id, self.channel_id)
			.map_err(|e| discordrs::DiscordError::model(format!("failed to create DAVE session: {e:?}")))?;
		self.session = Some(session);
		self.version = version;
		self.joined_epoch = None;
		self.external_sender_applied = false;
		self.consumed_proposals = 0;
		self.last_commit = None;
		self.last_welcome = None;
		self.key_package_sent = false;
		self.last_failure = None;
		Ok(())
	}

	async fn send_key_package(&mut self, runtime: &VoiceRuntimeHandle) -> Result<(), discordrs::DiscordError> {
		let Some(session) = self.session.as_mut() else {
			return Ok(());
		};
		let key_package = session.create_key_package()?;
		runtime.send_dave_mls_key_package(key_package)?;
		self.key_package_sent = true;
		self.note_success();
		debug!("DAVE key package sent");
		Ok(())
	}

	async fn recover(
		&mut self,
		runtime: &VoiceRuntimeHandle,
		transition_id: u64,
		key: FailureKey,
	) -> Result<(), discordrs::DiscordError> {
		if self.note_failure(key) {
			warn!("DAVE state invalid, rejoining the MLS group");
		}
		runtime.send_dave_mls_invalid_commit_welcome(transition_id)?;
		self.fresh_session(self.version)?;
		self.send_key_package(runtime).await
	}

	pub async fn drive(&mut self, runtime: &VoiceRuntimeHandle) -> Result<(), discordrs::DiscordError> {
		let state = runtime.state();
		let dave = &state.dave;
		let version = dave.protocol_version.unwrap_or(0) as u16;
		let epoch = dave.epoch.unwrap_or(0);

		if self.session.is_none() {
			if version == 0 {
				return Ok(());
			}
			self.fresh_session(version)?;
			self.send_key_package(runtime).await?;
			return Ok(());
		}

		if version != 0 && version != self.version {
			debug!("DAVE protocol version changed: {} -> {}, rejoining", self.version, version);
			self.fresh_session(version)?;
			self.send_key_package(runtime).await?;
			return Ok(());
		}

		if !self.key_package_sent {
			self.send_key_package(runtime).await?;
		}

		if let Some(sender) = dave.external_sender.clone()
			&& !self.external_sender_applied
			&& let Some(session) = self.session.as_mut()
		{
			match session.set_external_sender(&sender) {
				Ok(()) => {
					self.external_sender_applied = true;
					self.note_success();
					debug!("DAVE external sender applied");
				}
				Err(e) => {
					if self.note_failure(FailureKey::ExternalSender { len: sender.len() }) {
						warn!("DAVE failed to apply external sender: {}", e);
					}
				}
			}
		}

		while self.consumed_proposals < dave.proposals.len() {
			let index = self.consumed_proposals;
			let Some((&operation, proposals)) = dave.proposals[index].split_first() else {
				self.consumed_proposals += 1;
				continue;
			};
			let operation = match operation {
				0 => ProposalsOperationType::APPEND,
				1 => ProposalsOperationType::REVOKE,
				_ => {
					self.consumed_proposals += 1;
					continue;
				}
			};

			let key = FailureKey::Proposals {
				index,
				len: proposals.len(),
			};
			if self.last_failure == Some(key) {
				self.consumed_proposals += 1;
				continue;
			}

			let commit = match self.session.as_mut() {
				Some(session) => match session.process_proposals(operation, proposals, None) {
					Ok(commit) => commit,
					Err(e) => {
						if self.note_failure(key) {
							warn!("DAVE failed to process proposals, skipping: {}", e);
						}
						self.consumed_proposals += 1;
						continue;
					}
				},
				None => return Ok(()),
			};
			self.consumed_proposals += 1;
			self.note_success();

			if let Some(commit_welcome) = commit {
				runtime.send_dave_mls_commit_welcome(commit_welcome.commit, commit_welcome.welcome)?;
			}
		}

		if let Some(commit) = dave.pending_commit.clone() {
			let transition_id = dave.transition_id.unwrap_or(0);
			let key = (transition_id, commit.len());
			if self.last_commit != Some(key) {
				match self.session.as_mut() {
					Some(session) => match session.process_commit(&commit) {
						Ok(()) => {
							debug!("DAVE commit applied for transition {}", transition_id);
							self.joined_epoch = Some(epoch.max(1));
							self.note_success();
							if transition_id != 0 {
								runtime.send_dave_transition_ready(transition_id)?;
							}
						}
						Err(e) => {
							let key = FailureKey::Commit {
								transition_id,
								len: commit.len(),
							};
							self.recover(runtime, transition_id, key).await?;
							return Err(discordrs::DiscordError::model(format!("DAVE commit rejected: {e:?}")));
						}
					},
					None => return Ok(()),
				}
				self.last_commit = Some(key);
			}
		}

		if let Some(welcome) = dave.pending_welcome.clone() {
			let transition_id = dave.transition_id.unwrap_or(0);
			let key = (transition_id, welcome.len());
			if self.last_welcome != Some(key) {
				match self.session.as_mut() {
					Some(session) => match session.process_welcome(&welcome) {
						Ok(()) => {
							debug!("DAVE welcome applied for transition {}", transition_id);
							self.joined_epoch = Some(epoch.max(1));
							self.note_success();
							if transition_id != 0 {
								runtime.send_dave_transition_ready(transition_id)?;
							}
						}
						Err(e) => {
							let key = FailureKey::Welcome {
								transition_id,
								len: welcome.len(),
							};
							self.recover(runtime, transition_id, key).await?;
							return Err(discordrs::DiscordError::model(format!("DAVE welcome rejected: {e:?}")));
						}
					},
					None => return Ok(()),
				}
				self.last_welcome = Some(key);
			}
		}

		let announced = dave.epoch.unwrap_or(0);
		let joined = self.joined_epoch.unwrap_or(0);
		if announced > 0
			&& announced != joined
			&& dave.pending_commit.is_none()
			&& dave.pending_welcome.is_none()
			&& self.consumed_proposals >= dave.proposals.len()
		{
			debug!(
				"DAVE missed epoch transition (at {}, announced {}), rejoining",
				joined, announced
			);
			self.fresh_session(version)?;
			self.send_key_package(runtime).await?;
			return Ok(());
		}

		if self.passthrough != dave.passthrough {
			self.passthrough = dave.passthrough;
			debug!("DAVE passthrough mode: {}", self.passthrough);
		}

		Ok(())
	}
}
