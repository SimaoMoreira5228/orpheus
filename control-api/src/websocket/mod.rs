pub mod incoming;
pub mod outgoing;
pub mod session;

pub(crate) use outgoing::player_update_json;
pub use session::{WebSocketSession, WebSocketState, configure, ws_index};

pub(crate) mod ws_message {
	use actix::Message;

	#[derive(Message)]
	#[rtype(result = "()")]
	pub struct Text(pub String);
}
