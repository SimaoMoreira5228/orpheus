use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::{Filters, LoadData, PlayerStateInfo, Stats, Track};

#[derive(Debug, Clone)]
pub struct WsConfig {
	pub host: String,
	pub port: u16,
	pub password: String,
	pub user_id: u64,
	pub client_name: Option<String>,
	pub session_id: Option<String>,
	pub tls: bool,
}

impl WsConfig {
	pub fn url(&self) -> String {
		let scheme = if self.tls { "wss" } else { "ws" };

		format!("{scheme}://{}:{}/v4/websocket", self.host, self.port)
	}

	fn client_name(&self) -> String {
		self.client_name
			.clone()
			.unwrap_or_else(|| format!("orpheus-rust-sdk/{}", env!("CARGO_PKG_VERSION")))
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum WsEvent {
	#[serde(rename = "ready")]
	Ready {
		#[serde(rename = "sessionId")]
		session_id: String,
		resumed: bool,
		#[serde(rename = "sourceManagers", default)]
		source_managers: Vec<String>,
	},
	#[serde(rename = "playerUpdate")]
	PlayerUpdate {
		#[serde(rename = "guildId")]
		guild_id: String,
		state: PlayerStateInfo,
	},
	#[serde(rename = "stats")]
	Stats(Stats),
	#[serde(rename = "event")]
	Event(Box<WsTrackEvent>),
	#[serde(rename = "loadtracks")]
	LoadTracks(WsLoadTracks),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsTrackEvent {
	#[serde(rename = "TrackStartEvent")]
	TrackStart {
		track: Track,
		#[serde(rename = "guildId")]
		guild_id: String,
	},
	#[serde(rename = "TrackEndEvent")]
	TrackEnd {
		track: Track,
		reason: TrackEndReason,
		#[serde(rename = "guildId")]
		guild_id: String,
	},
	#[serde(rename = "TrackExceptionEvent")]
	TrackException {
		track: Track,
		exception: WsException,
		#[serde(rename = "guildId")]
		guild_id: String,
	},
	#[serde(rename = "TrackStuckEvent")]
	TrackStuck {
		track: Track,
		#[serde(rename = "thresholdMs")]
		threshold_ms: i64,
		#[serde(rename = "guildId")]
		guild_id: String,
	},
	#[serde(rename = "WebSocketClosedEvent")]
	WebSocketClosed {
		code: i32,
		reason: String,
		#[serde(rename = "byRemote")]
		by_remote: bool,
		#[serde(rename = "guildId")]
		guild_id: String,
	},
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackEndReason {
	Finished,
	LoadFailed,
	Stopped,
	Replaced,
	Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ExceptionSeverity {
	Common,
	Suspicious,
	Fault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsException {
	pub message: String,
	pub severity: ExceptionSeverity,
	pub cause: String,
	#[serde(rename = "causeStackTrace", default, skip_serializing_if = "Option::is_none")]
	pub cause_stack_trace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsLoadTracks {
	#[serde(rename = "loadType")]
	pub load_type: String,
	pub data: LoadData,
}

#[derive(Debug, Clone)]
pub enum ParsedFrame {
	Event(WsEvent),
	Pong(i64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PongFrame {
	timestamp: i64,
}

pub(crate) fn parse_server_frame(text: &str) -> Option<ParsedFrame> {
	if let Ok(event) = serde_json::from_str::<WsEvent>(text) {
		return Some(ParsedFrame::Event(event));
	}

	if let Ok(pong) = serde_json::from_str::<PongFrame>(text) {
		return Some(ParsedFrame::Pong(pong.timestamp));
	}

	None
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "op")]
pub enum ClientMessage {
	#[serde(rename = "ping")]
	Ping {
		timestamp: i64,
	},
	#[serde(rename = "destroy")]
	Destroy {
		#[serde(rename = "guildId")]
		guild_id: String,
	},
	#[serde(rename = "configureResuming")]
	ConfigureResuming {
		#[serde(default, skip_serializing_if = "Option::is_none")]
		key: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		timeout: Option<i32>,
	},
	#[serde(rename = "playerUpdate")]
	PlayerUpdate {
		#[serde(rename = "guildId")]
		guild_id: String,
		data: Box<WsPlayerUpdateData>,
	},
	#[serde(rename = "voiceUpdate")]
	VoiceUpdate {
		#[serde(rename = "guildId")]
		guild_id: String,
		data: WsVoiceData,
	},
	#[serde(rename = "loadtracks")]
	LoadTracks {
		identifier: String,
	},
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WsPlayerTrackUpdate {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub encoded: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub identifier: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WsVoiceData {
	pub token: String,
	pub endpoint: String,
	#[serde(rename = "sessionId")]
	pub session_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WsPlayerUpdateData {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub track: Option<WsPlayerTrackUpdate>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub position: Option<i64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub end_time: Option<i64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub volume: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub paused: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub voice: Option<WsVoiceData>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub filters: Option<Filters>,
}

#[derive(Debug)]
pub enum WsError {
	Connect(String),
	Send(String),
	Encode(String),
}

impl std::fmt::Display for WsError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			WsError::Connect(message) => write!(formatter, "websocket connect failed: {message}"),
			WsError::Send(message) => write!(formatter, "websocket send failed: {message}"),
			WsError::Encode(message) => write!(formatter, "websocket encode failed: {message}"),
		}
	}
}

impl std::error::Error for WsError {}

type WriteHalf = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

pub struct WsClient {
	write: Arc<Mutex<WriteHalf>>,
}

impl WsClient {
	pub async fn connect(config: &WsConfig, handler: impl Fn(WsEvent) + Send + Sync + 'static) -> Result<Self, WsError> {
		let mut request = http::Request::builder()
			.uri(config.url())
			.header("Authorization", &config.password)
			.header("User-Id", config.user_id.to_string())
			.header("Client-Name", config.client_name());

		if let Some(session_id) = config.session_id.as_deref() {
			request = request.header("Session-Id", session_id);
		}

		let request = request.body(()).map_err(|e| WsError::Connect(e.to_string()))?;
		let (stream, _) = connect_async(request).await.map_err(|e| WsError::Connect(e.to_string()))?;
		let (write, read) = stream.split();
		let write = Arc::new(Mutex::new(write));

		tokio::spawn(async move {
			let mut read = read;

			while let Some(message) = read.next().await {
				let Ok(Message::Text(text)) = message else {
					continue;
				};

				match parse_server_frame(&text) {
					Some(ParsedFrame::Event(event)) => handler(event),
					Some(ParsedFrame::Pong(_)) => {}
					None => {}
				}
			}
		});

		Ok(Self { write })
	}

	async fn send(&self, message: ClientMessage) -> Result<(), WsError> {
		let text = serde_json::to_string(&message).map_err(|e| WsError::Encode(e.to_string()))?;

		self.write
			.lock()
			.await
			.send(Message::Text(text.into()))
			.await
			.map_err(|e| WsError::Send(e.to_string()))
	}

	pub async fn ping(&self, timestamp: i64) -> Result<(), WsError> {
		self.send(ClientMessage::Ping { timestamp }).await
	}

	pub async fn destroy(&self, guild_id: u64) -> Result<(), WsError> {
		self.send(ClientMessage::Destroy {
			guild_id: guild_id.to_string(),
		})
		.await
	}

	pub async fn configure_resuming(&self, key: Option<String>, timeout: Option<i32>) -> Result<(), WsError> {
		self.send(ClientMessage::ConfigureResuming { key, timeout }).await
	}

	pub async fn player_update(&self, guild_id: u64, data: WsPlayerUpdateData) -> Result<(), WsError> {
		self.send(ClientMessage::PlayerUpdate {
			guild_id: guild_id.to_string(),
			data: Box::new(data),
		})
		.await
	}

	pub async fn voice_update(
		&self,
		guild_id: u64,
		token: String,
		endpoint: String,
		session_id: String,
	) -> Result<(), WsError> {
		self.send(ClientMessage::VoiceUpdate {
			guild_id: guild_id.to_string(),
			data: WsVoiceData {
				token,
				endpoint,
				session_id,
			},
		})
		.await
	}

	pub async fn load_tracks(&self, identifier: String) -> Result<(), WsError> {
		self.send(ClientMessage::LoadTracks { identifier }).await
	}

	pub async fn close(&self) -> Result<(), WsError> {
		self.write
			.lock()
			.await
			.close()
			.await
			.map_err(|e| WsError::Send(e.to_string()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn config() -> WsConfig {
		WsConfig {
			host: "127.0.0.1".to_string(),
			port: 2333,
			password: "secret".to_string(),
			user_id: 7,
			client_name: None,
			session_id: None,
			tls: false,
		}
	}

	#[test]
	fn url_uses_ws_or_wss_scheme() {
		assert_eq!(config().url(), "ws://127.0.0.1:2333/v4/websocket");

		let tls = WsConfig { tls: true, ..config() };

		assert_eq!(tls.url(), "wss://127.0.0.1:2333/v4/websocket");
	}

	#[test]
	fn default_client_name_carries_crate_version() {
		let name = config().client_name();

		assert!(name.starts_with("orpheus-rust-sdk/"));
		assert_eq!(name, format!("orpheus-rust-sdk/{}", env!("CARGO_PKG_VERSION")));
	}

	#[test]
	fn client_messages_use_lavalink_ops() {
		let ping = serde_json::to_value(ClientMessage::Ping { timestamp: 42 }).unwrap();

		assert_eq!(ping, serde_json::json!({"op": "ping", "timestamp": 42}));

		let destroy = serde_json::to_value(ClientMessage::Destroy {
			guild_id: "7".to_string(),
		})
		.unwrap();

		assert_eq!(destroy, serde_json::json!({"op": "destroy", "guildId": "7"}));

		let resume = serde_json::to_value(ClientMessage::ConfigureResuming {
			key: Some("key".to_string()),
			timeout: Some(60),
		})
		.unwrap();

		assert_eq!(
			resume,
			serde_json::json!({"op": "configureResuming", "key": "key", "timeout": 60})
		);

		let voice = serde_json::to_value(ClientMessage::VoiceUpdate {
			guild_id: "7".to_string(),
			data: WsVoiceData {
				token: "t".to_string(),
				endpoint: "e".to_string(),
				session_id: "s".to_string(),
			},
		})
		.unwrap();

		assert_eq!(
			voice,
			serde_json::json!({"op": "voiceUpdate", "guildId": "7", "data": {"token": "t", "endpoint": "e", "sessionId": "s"}})
		);

		let load = serde_json::to_value(ClientMessage::LoadTracks {
			identifier: "ytsearch:test".to_string(),
		})
		.unwrap();

		assert_eq!(load, serde_json::json!({"op": "loadtracks", "identifier": "ytsearch:test"}));
	}

	#[test]
	fn player_update_omits_absent_fields() {
		let update = serde_json::to_value(ClientMessage::PlayerUpdate {
			guild_id: "7".to_string(),
			data: Box::new(WsPlayerUpdateData {
				paused: Some(true),
				..WsPlayerUpdateData::default()
			}),
		})
		.unwrap();

		assert_eq!(
			update,
			serde_json::json!({"op": "playerUpdate", "guildId": "7", "data": {"paused": true}})
		);
	}

	#[test]
	fn server_frames_dispatch_by_op() {
		let ready = parse_server_frame(r#"{"op":"ready","sessionId":"abc","resumed":true,"sourceManagers":["youtube"]}"#);

		assert!(matches!(
			ready,
			Some(ParsedFrame::Event(WsEvent::Ready { resumed: true, .. }))
		));

		let update = parse_server_frame(
			r#"{"op":"playerUpdate","guildId":"7","state":{"time":1,"position":0,"connected":true,"ping":-1}}"#,
		);

		assert!(matches!(update, Some(ParsedFrame::Event(WsEvent::PlayerUpdate { .. }))));

		let event = parse_server_frame(
			r#"{"op":"event","type":"TrackStartEvent","track":{"info":{"identifier":"x","isSeekable":true,"author":"a","length":0,"isStream":false,"position":0,"title":"t","uri":null,"artworkUrl":null,"isrc":null,"sourceName":"s"}},"guildId":"7"}"#,
		);

		assert!(matches!(event, Some(ParsedFrame::Event(WsEvent::Event(_)))));

		let pong = parse_server_frame(r#"{"op":"pong","timestamp":42}"#);

		assert!(matches!(pong, Some(ParsedFrame::Pong(42))));
		assert!(parse_server_frame(r#"{"op":"futureOp","x":1}"#).is_none());
	}
}
