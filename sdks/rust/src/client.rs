use orpheus_core::{Filters, PlayerSnapshot};
use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::models::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
	pub host: String,
	pub port: u16,
	pub password: String,
	pub ws_port: Option<u16>,
}

impl NodeConfig {
	pub fn new(host: &str, port: u16, password: &str) -> Self {
		Self {
			host: host.to_string(),
			port,
			password: password.to_string(),
			ws_port: None,
		}
	}

	pub fn with_ws_port(mut self, ws_port: u16) -> Self {
		self.ws_port = Some(ws_port);
		self
	}
}

pub struct OrpheusClient {
	node: NodeConfig,
	http_client: reqwest::Client,
	session_id: String,
}

#[derive(Debug)]
pub enum OrpheusError {
	Transport(reqwest::Error),
	Server {
		status: StatusCode,
		code: Option<String>,
		message: String,
		body: String,
	},
}

impl OrpheusError {
	pub fn code(&self) -> Option<&str> {
		match self {
			Self::Transport(_) => None,
			Self::Server { code, .. } => code.as_deref(),
		}
	}

	pub fn status(&self) -> Option<StatusCode> {
		match self {
			Self::Transport(_) => None,
			Self::Server { status, .. } => Some(*status),
		}
	}

	pub fn body(&self) -> Option<&str> {
		match self {
			Self::Transport(_) => None,
			Self::Server { body, .. } => Some(body),
		}
	}

	fn from_response(status: StatusCode, body: String) -> Self {
		let envelope = serde_json::from_str::<ErrorEnvelope>(&body).ok();
		let code = envelope.as_ref().and_then(|envelope| envelope.code.clone());
		let message = envelope
			.and_then(|envelope| envelope.message)
			.unwrap_or_else(|| status.canonical_reason().unwrap_or("Request failed").to_string());

		Self::Server {
			status,
			code,
			message,
			body,
		}
	}
}

impl std::fmt::Display for OrpheusError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Transport(error) => write!(f, "{error}"),
			Self::Server { status, message, .. } => write!(f, "Orpheus request failed: {status} {message}"),
		}
	}
}

impl std::error::Error for OrpheusError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Transport(error) => Some(error),
			Self::Server { .. } => None,
		}
	}
}

impl From<reqwest::Error> for OrpheusError {
	fn from(error: reqwest::Error) -> Self {
		Self::Transport(error)
	}
}

#[derive(Deserialize)]
struct ErrorEnvelope {
	code: Option<String>,
	message: Option<String>,
}

impl OrpheusClient {
	pub fn new(node: NodeConfig) -> Self {
		Self {
			http_client: reqwest::Client::new(),
			node,
			session_id: "default".to_string(),
		}
	}

	pub fn with_session(mut self, session_id: &str) -> Self {
		self.session_id = session_id.to_string();
		self
	}

	fn base_url(&self) -> String {
		format!("http://{}:{}", self.node.host, self.node.port)
	}

	async fn send_json<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, OrpheusError> {
		let response = request.send().await?;
		let status = response.status();

		if !status.is_success() {
			let body = response.text().await?;
			return Err(OrpheusError::from_response(status, body));
		}

		response.json().await.map_err(Into::into)
	}

	async fn send_empty(&self, request: RequestBuilder) -> Result<(), OrpheusError> {
		let response = request.send().await?;
		let status = response.status();

		if status.is_success() {
			return Ok(());
		}

		let body = response.text().await?;
		Err(OrpheusError::from_response(status, body))
	}

	pub async fn load_tracks(&self, identifier: &str) -> Result<LoadResult, OrpheusError> {
		let url = format!("{}/v4/loadtracks", self.base_url());

		self.send_json(
			self.http_client
				.get(&url)
				.header("Authorization", self.node.password.clone())
				.query(&[("identifier", identifier)]),
		)
		.await
	}

	pub async fn decode_track(&self, encoded: &str) -> Result<Track, OrpheusError> {
		let url = format!("{}/v4/decodetrack", self.base_url());

		self.send_json(
			self.http_client
				.get(&url)
				.header("Authorization", self.node.password.clone())
				.query(&[("encodedTrack", encoded)]),
		)
		.await
	}

	pub async fn decode_tracks(&self, tracks: &[String]) -> Result<Vec<Track>, OrpheusError> {
		let url = format!("{}/v4/decodetracks", self.base_url());

		self.send_json(
			self.http_client
				.post(&url)
				.header("Authorization", self.node.password.clone())
				.json(&serde_json::json!({ "tracks": tracks })),
		)
		.await
	}

	pub async fn load_tracks_batch(&self, identifiers: &[String]) -> Result<Vec<LoadResult>, OrpheusError> {
		let url = format!("{}/v4/loadtracks", self.base_url());

		self.send_json(
			self.http_client
				.post(&url)
				.header("Authorization", self.node.password.clone())
				.json(&serde_json::json!({ "identifiers": identifiers })),
		)
		.await
	}

	pub async fn get_all_players(&self) -> Result<Vec<PlayerState>, OrpheusError> {
		let url = format!("{}/v4/sessions/{}/players", self.base_url(), self.session_id);

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}

	pub async fn get_player(&self, guild_id: u64) -> Result<PlayerState, OrpheusError> {
		let url = format!("{}/v4/sessions/{}/players/{}", self.base_url(), self.session_id, guild_id);

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}

	pub async fn update_player(&self, guild_id: u64, update: &PlayerUpdate) -> Result<PlayerState, OrpheusError> {
		let url = format!("{}/v4/sessions/{}/players/{}", self.base_url(), self.session_id, guild_id);

		self.send_json(
			self.http_client
				.patch(&url)
				.header("Authorization", self.node.password.clone())
				.json(update),
		)
		.await
	}

	pub async fn export_player(&self, guild_id: u64) -> Result<PlayerSnapshot, OrpheusError> {
		let url = format!(
			"{}/v4/sessions/{}/players/{}/export",
			self.base_url(),
			self.session_id,
			guild_id
		);

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}

	pub async fn import_player(&self, guild_id: u64, snapshot: &PlayerSnapshot) -> Result<PlayerState, OrpheusError> {
		let url = format!(
			"{}/v4/sessions/{}/players/{}/import",
			self.base_url(),
			self.session_id,
			guild_id
		);

		self.send_json(
			self.http_client
				.post(&url)
				.header("Authorization", self.node.password.clone())
				.json(snapshot),
		)
		.await
	}

	pub async fn delete_player(&self, guild_id: u64) -> Result<(), OrpheusError> {
		let url = format!("{}/v4/sessions/{}/players/{}", self.base_url(), self.session_id, guild_id);

		self.send_empty(
			self.http_client
				.delete(&url)
				.header("Authorization", self.node.password.clone()),
		)
		.await
	}

	pub async fn play(&self, guild_id: u64, track: &Track) -> Result<PlayerState, OrpheusError> {
		let track_update = if track.encoded.is_some() {
			PlayerTrackUpdate {
				encoded: track.encoded.clone(),
				identifier: None,
				user_data: None,
			}
		} else {
			PlayerTrackUpdate {
				encoded: None,
				identifier: track.info.uri.clone(),
				user_data: None,
			}
		};
		let update = PlayerUpdate {
			track: Some(Some(track_update)),
			position: Some(0),
			paused: Some(false),
			..Default::default()
		};
		self.update_player(guild_id, &update).await
	}

	pub async fn pause(&self, guild_id: u64, paused: bool) -> Result<PlayerState, OrpheusError> {
		let update = PlayerUpdate {
			paused: Some(paused),
			..Default::default()
		};
		self.update_player(guild_id, &update).await
	}

	pub async fn set_volume(&self, guild_id: u64, volume: i32) -> Result<PlayerState, OrpheusError> {
		let update = PlayerUpdate {
			volume: Some(volume),
			..Default::default()
		};
		self.update_player(guild_id, &update).await
	}

	pub async fn seek(&self, guild_id: u64, position: i64) -> Result<PlayerState, OrpheusError> {
		let update = PlayerUpdate {
			position: Some(position),
			..Default::default()
		};
		self.update_player(guild_id, &update).await
	}

	pub async fn get_queue(&self, guild_id: u64) -> Result<QueueResponse, OrpheusError> {
		let url = format!(
			"{}/v4/sessions/{}/players/{}/queue",
			self.base_url(),
			self.session_id,
			guild_id
		);

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}

	pub async fn add_to_queue(&self, guild_id: u64, request: &AddToQueueRequest) -> Result<QueueResponse, OrpheusError> {
		let url = format!(
			"{}/v4/sessions/{}/players/{}/queue",
			self.base_url(),
			self.session_id,
			guild_id
		);

		self.send_json(
			self.http_client
				.post(&url)
				.header("Authorization", self.node.password.clone())
				.json(request),
		)
		.await
	}

	pub async fn update_queue(&self, guild_id: u64, request: &QueueUpdateRequest) -> Result<QueueResponse, OrpheusError> {
		let url = format!(
			"{}/v4/sessions/{}/players/{}/queue",
			self.base_url(),
			self.session_id,
			guild_id
		);

		self.send_json(
			self.http_client
				.patch(&url)
				.header("Authorization", self.node.password.clone())
				.json(request),
		)
		.await
	}

	pub async fn remove_from_queue(&self, guild_id: u64, position: usize) -> Result<QueuePositionResponse, OrpheusError> {
		let url = format!(
			"{}/v4/sessions/{}/players/{}/queue/{}",
			self.base_url(),
			self.session_id,
			guild_id,
			position
		);

		self.send_json(
			self.http_client
				.delete(&url)
				.header("Authorization", self.node.password.clone()),
		)
		.await
	}

	pub async fn set_voice_channel(&self, guild_id: u64, channel_id: u64) -> Result<(), OrpheusError> {
		let url = format!(
			"{}/v4/sessions/{}/players/{}/voice-channel",
			self.base_url(),
			self.session_id,
			guild_id
		);

		self.send_empty(
			self.http_client
				.put(&url)
				.header("Authorization", self.node.password.clone())
				.json(&serde_json::json!({ "channelId": channel_id })),
		)
		.await
	}

	pub async fn update_session(
		&self,
		resuming: Option<bool>,
		timeout_secs: Option<i64>,
	) -> Result<SessionState, OrpheusError> {
		let url = format!("{}/v4/sessions/{}", self.base_url(), self.session_id);

		self.send_json(
			self.http_client
				.patch(&url)
				.header("Authorization", self.node.password.clone())
				.json(&SessionUpdate { resuming, timeout_secs }),
		)
		.await
	}

	pub async fn list_filter_presets(&self) -> Result<std::collections::HashMap<String, Filters>, OrpheusError> {
		let url = format!("{}/v4/filters/presets", self.base_url());

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}

	pub async fn get_filter_preset(&self, name: &str) -> Result<Filters, OrpheusError> {
		let url = format!("{}/v4/filters/presets/{}", self.base_url(), name);

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}

	pub async fn put_filter_preset(&self, name: &str, filters: &Filters) -> Result<Filters, OrpheusError> {
		let url = format!("{}/v4/filters/presets/{}", self.base_url(), name);

		self.send_json(
			self.http_client
				.put(&url)
				.header("Authorization", self.node.password.clone())
				.json(filters),
		)
		.await
	}

	pub async fn delete_filter_preset(&self, name: &str) -> Result<(), OrpheusError> {
		let url = format!("{}/v4/filters/presets/{}", self.base_url(), name);

		self.send_empty(
			self.http_client
				.delete(&url)
				.header("Authorization", self.node.password.clone()),
		)
		.await
	}

	pub async fn load_search(&self, query: &str) -> Result<RichSearchResponse, OrpheusError> {
		let url = format!("{}/v4/loadsearch", self.base_url());

		self.send_json(
			self.http_client
				.get(&url)
				.header("Authorization", self.node.password.clone())
				.query(&[("query", query)]),
		)
		.await
	}

	pub async fn load_tracks_dry_run(&self, identifiers: &[String]) -> Result<Vec<DryRunItem>, OrpheusError> {
		let url = format!("{}/v4/loadtracks/dry-run", self.base_url());

		self.send_json(
			self.http_client
				.post(&url)
				.header("Authorization", self.node.password.clone())
				.json(&serde_json::json!({ "identifiers": identifiers })),
		)
		.await
	}

	pub async fn get_info(&self) -> Result<NodeInfo, OrpheusError> {
		let url = format!("{}/v4/info", self.base_url());

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}

	pub async fn get_stats(&self) -> Result<Stats, OrpheusError> {
		let url = format!("{}/v4/stats", self.base_url());

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}

	pub async fn get_version(&self) -> Result<Version, OrpheusError> {
		let url = format!("{}/version", self.base_url());

		self.send_json(self.http_client.get(&url).header("Authorization", self.node.password.clone()))
			.await
	}
}

impl Default for OrpheusClient {
	fn default() -> Self {
		Self::new(NodeConfig::new("127.0.0.1", 2333, "youshallnotpass"))
	}
}

#[cfg(test)]
mod tests {
	use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
	use tokio::net::TcpListener;

	use super::*;

	#[test]
	fn server_error_exposes_code_status_and_body() {
		let body = serde_json::json!({
			"status": 404,
			"message": "Session not found",
			"code": "SESSION_NOT_FOUND",
		})
		.to_string();
		let error = OrpheusError::from_response(StatusCode::NOT_FOUND, body);

		assert_eq!(error.code(), Some("SESSION_NOT_FOUND"));
		assert_eq!(error.status(), Some(StatusCode::NOT_FOUND));
		assert!(error.body().unwrap().contains("SESSION_NOT_FOUND"));
		assert!(error.to_string().contains("Session not found"));
	}

	#[tokio::test]
	async fn load_tracks_keeps_identifier_in_one_query_parameter() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		let server = tokio::spawn(async move {
			let (stream, _) = listener.accept().await.unwrap();
			let mut stream = BufReader::new(stream);
			let mut request_line = String::new();
			stream.read_line(&mut request_line).await.unwrap();

			let body = r#"{"loadType":"empty","data":[]}"#;
			let response = format!(
				"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
				body.len()
			);
			stream.get_mut().write_all(response.as_bytes()).await.unwrap();
			request_line
		});
		let host = address.ip().to_string();
		let client = OrpheusClient::new(NodeConfig::new(&host, address.port(), "secret"));
		let identifier = "ytsearch:lofi hip hop&trace=true";

		client.load_tracks(identifier).await.unwrap();

		let request_line = server.await.unwrap();
		let target = request_line.split_whitespace().nth(1).unwrap();
		let url = reqwest::Url::parse(&format!("http://{address}{target}")).unwrap();
		let query: Vec<_> = url.query_pairs().collect();

		assert_eq!(query.len(), 1);
		assert_eq!(query[0].0, "identifier");
		assert_eq!(query[0].1, identifier);
	}
}
