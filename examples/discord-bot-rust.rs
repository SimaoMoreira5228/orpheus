use std::env;

use orpheus_client_sdk::{NodeConfig, OrpheusClient};
use serenity::async_trait;
use serenity::client::{Client, EventHandler};
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

struct Handler {
	orpheus: OrpheusClient,
}

#[async_trait]
impl EventHandler for Handler {
	async fn ready(&self, _: Context, ready: Ready) {
		println!("Logged in as {}", ready.user.name);
	}

	async fn message(&self, ctx: Context, msg: Message) {
		if !msg.content.starts_with('!') {
			return;
		}

		let content = msg.content.strip_prefix('!').unwrap_or("");
		let parts: Vec<&str> = content.splitn(2, ' ').collect();
		let cmd = parts[0];
		let args = parts.get(1).copied();

		let guild_id = match msg.guild_id {
			Some(id) => id.get(),
			None => return,
		};

		let orpheus = &self.orpheus;

		match cmd {
			"play" => {
				let query = args.unwrap_or("");
				if query.is_empty() {
					let _ = msg.reply(&ctx, "Usage: !play <query>").await;
					return;
				}

				match orpheus.load_tracks(query).await {
					Ok(result) => {
						if let Some(track) = result.first_track() {
							match orpheus.play(guild_id, track).await {
								Ok(_) => {
									let title = &track.info.title;
									let _ = msg.reply(&ctx, &format!("Now playing: {}", title)).await;
								}
								Err(e) => {
									let _ = msg.reply(&ctx, &format!("Error: {}", e)).await;
								}
							}
						} else {
							let _ = msg.reply(&ctx, "No tracks found").await;
						}
					}
					Err(e) => {
						let _ = msg.reply(&ctx, &format!("Error: {}", e)).await;
					}
				}
			}
			"pause" => {
				let _ = orpheus.pause(guild_id, true).await;
				let _ = msg.reply(&ctx, "Paused").await;
			}
			"resume" => {
				let _ = orpheus.pause(guild_id, false).await;
				let _ = msg.reply(&ctx, "Resumed").await;
			}
			"stop" => {
				let _ = orpheus.delete_player(guild_id).await;
				let _ = msg.reply(&ctx, "Stopped").await;
			}
			"volume" => {
				if let Some(vol_str) = args {
					if let Ok(volume) = vol_str.parse::<i32>() {
						let _ = orpheus.set_volume(guild_id, volume).await;
						let _ = msg.reply(&ctx, &format!("Volume set to {}%", volume)).await;
					}
				} else {
					let _ = msg.reply(&ctx, "Usage: !volume <0-1000>").await;
				}
			}
			"help" => {
				let _ = msg
					.reply(&ctx, "Commands: !play, !pause, !resume, !stop, !volume, !help")
					.await;
			}
			_ => {}
		}
	}
}

#[tokio::main]
async fn main() {
	let token = env::var("DISCORD_TOKEN").expect("Expected DISCORD_TOKEN environment variable");

	let host = env::var("ORPHEUS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
	let port: u16 = env::var("ORPHEUS_PORT")
		.unwrap_or_else(|_| "2333".to_string())
		.parse()
		.unwrap_or(2333);
	let password = env::var("ORPHEUS_PASSWORD").unwrap_or_else(|_| "youshallnotpass".to_string());

	let node = NodeConfig::new(&host, port, &password);
	let orpheus = OrpheusClient::new(node);

	let handler = Handler { orpheus };

	let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;

	let mut client = Client::builder(&token, intents)
		.event_handler(handler)
		.await
		.expect("Error creating client");

	if let Err(e) = client.start().await {
		println!("Client error: {:?}", e);
	}
}
