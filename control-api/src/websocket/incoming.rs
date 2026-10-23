use actix::prelude::*;
use actix_web_actors::ws;

use super::session::WebSocketSession;
use super::ws_message;

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocketSession {
	fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
		match msg {
			Ok(ws::Message::Text(text)) => {
				tracing::info!(">>> WS TEXT: {}", text);

				if let Ok(incoming) = serde_json::from_str::<crate::websocket_messages::IncomingMessage>(&text) {
					match incoming {
						crate::websocket_messages::IncomingMessage::Ping { timestamp } => {
							let pong = serde_json::json!({"op":"pong","timestamp": timestamp});

							if let Ok(s) = serde_json::to_string(&pong) {
								ctx.text(s);
							}
						}
						crate::websocket_messages::IncomingMessage::Destroy { guild_id } => {
							if let Ok(gid) = guild_id.parse::<u64>() {
								let state = self.state.clone();
								let gid_clone = gid;
								let session_id = self.session_id.clone();
								let fut = async move {
									state.prefetch.invalidate(gid_clone);
									let stopped_track = {
										let player_lock = state.player_manager.get_or_create(gid_clone).await;
										let player = player_lock.read().await;
										player.current.clone()
									};

									state.player_manager.remove(gid_clone).await;
									state.voice_connection_manager.destroy(gid_clone).await;

									if state.sessions.delivers_to(&session_id, gid_clone) {
										return None;
									}

									let event = crate::websocket_messages::OutgoingMessage::Event(Box::new(
										crate::websocket_messages::Event::TrackEnd(
											crate::websocket_messages::TrackEndEvent {
												track: stopped_track.unwrap_or(orpheus_core::Track {
													encoded: None,
													info: orpheus_core::TrackInfo {
														identifier: String::new(),
														is_seekable: false,
														author: String::new(),
														length: 0,
														is_stream: false,
														position: 0,
														title: String::new(),
														artwork_url: None,
														isrc: None,
														source_name: String::new(),
														uri: None,
													},
													start_time: None,
													end_time: None,
													plugin_info: serde_json::Value::default(),
													user_data: serde_json::Value::default(),
												}),
												reason: crate::websocket_messages::TrackEndReason::Stopped,
												guild_id: gid_clone.to_string(),
											},
										),
									));
									serde_json::to_string(&event).ok()
								};
								let fut = fut.into_actor(self).map(|maybe, _actor, ctx| {
									if let Some(json) = maybe {
										ctx.text(json);
									}
								});
								ctx.spawn(fut);
							}
						}
						crate::websocket_messages::IncomingMessage::ConfigureResuming { key, timeout } => {
							let resuming = key.map(|key| !key.is_empty());
							let timeout = timeout.map(i64::from);

							self.state.sessions.update(&self.session_id, resuming, timeout);
						}
						crate::websocket_messages::IncomingMessage::PlayerUpdate { guild_id, data } => {
							if let Ok(gid) = guild_id.parse::<u64>() {
								let state = self.state.clone();
								let voice_mgr = self.state.voice_connection_manager.clone();
								let data_clone = data.clone();
								state.sessions.touch(&self.session_id, gid);
								ctx.spawn(
									async move {
										let player_lock = state.player_manager.get_or_create(gid).await;
										let mut player = player_lock.write().await;

										let live_position = voice_mgr.current_position(gid);

										let mut seek_request: Option<u64> = None;
										let mut volume_request: Option<u32> = None;
										let mut pause_request: Option<bool> = None;
										let mut filters_request: Option<orpheus_core::Filters> = None;
										let mut play_request: Option<(
											orpheus_core::Track,
											u64,
											u32,
											orpheus_core::Filters,
										)> = None;

										if let Some(position) = data_clone.position {
											let clamped = position.max(0) as u64;
											player.position = clamped;
											seek_request = Some(clamped);
										}

										if let Some(volume) = data_clone.volume {
											let clamped = volume.max(0.0) as u32;
											player.set_volume(clamped);
											volume_request = Some(player.volume);
										}

										if let Some(paused) = data_clone.paused {
											if paused {
												player.pause();
											} else {
												player.play();
											}

											pause_request = Some(paused);
										}

										if let Some(incoming) = data_clone.filters.clone() {
											player.set_filters(incoming.clone());
											filters_request = Some(incoming);
										}

										if let Some(track_data) = data_clone.track {
											if let Some(encoded) = track_data.encoded {
												match orpheus_track_resolver::decode_track(&encoded) {
													Ok(decoded) => {
														let track = decoded.track.clone();

														player.current = Some(decoded.track);

														if player.state != orpheus_core::PlayerState::Paused
															&& track.playable_uri().is_some()
														{
															let seek_base =
																seek_request.or(live_position).unwrap_or(player.position);

															play_request = Some((
																track,
																seek_base,
																player.volume,
																player.filters.clone(),
															));
														}
													}
													Err(e) => {
														tracing::warn!(
															"WS playerUpdate for guild {} — ignoring undecodable track: {}; keeping current track",
															gid,
															e
														);
													}
												}
											} else if let Some(identifier) = track_data.identifier {
												let track = orpheus_core::Track {
													encoded: track_data.encoded,
													info: orpheus_core::TrackInfo {
														identifier: identifier.clone(),
														is_seekable: false,
														author: String::new(),
														length: 0,
														is_stream: false,
														position: 0,
														title: String::new(),
														artwork_url: None,
														isrc: None,
														source_name: String::new(),
														uri: Some(identifier),
													},
													start_time: None,
													end_time: None,
													plugin_info: serde_json::Value::default(),
													user_data: serde_json::Value::default(),
												};

												player.current = Some(track.clone());

												if player.state != orpheus_core::PlayerState::Paused {
													let seek_base =
														seek_request.or(live_position).unwrap_or(player.position);

													play_request =
														Some((track, seek_base, player.volume, player.filters.clone()));
												}
											}
										}

										drop(player);

										if let Some(requested) = volume_request {
											voice_mgr.set_volume(gid, requested).await;
										}

										if let Some(requested) = seek_request {
											voice_mgr.seek(gid, requested).await;
										}

										if let Some(requested) = pause_request {
											voice_mgr.pause(gid, requested).await;
										}

										if let Some(requested) = filters_request {
											voice_mgr.set_filters(gid, requested).await;
										}

										if let Some((track, seek_position, play_volume, play_filters)) = play_request {
											let end_time = track.end_time;
											let playback_uri = state.track_resolver.playback_url(&track).await;

											let mut voice_track = track;

											voice_track.info.uri = Some(playback_uri);

											voice_mgr
												.play(gid, voice_track, seek_position, play_volume, play_filters, end_time)
												.await;
										}

										state.voice_connection_manager.request_player_update(gid);
									}
									.into_actor(self)
									.map(|_, _, _| {}),
								);
							}
						}
						crate::websocket_messages::IncomingMessage::VoiceUpdate { guild_id, data } => {
							if let Ok(gid) = guild_id.parse::<u64>() {
								let state = self.state.clone();
								let voice_mgr = self.state.voice_connection_manager.clone();
								let data_clone = data.clone();
								state.sessions.touch(&self.session_id, gid);
								let user_id = self
									.state
									.sessions
									.get(&self.session_id)
									.map(|session| session.user_id)
									.unwrap_or(0);
								ctx.spawn(
									async move {
										let voice_state = orpheus_core::VoiceState {
											token: data_clone.token.clone(),
											endpoint: data_clone.endpoint.clone(),
											session_id: data_clone.session_id.clone(),
											channel_id: None,
										};

										let resume = {
											let player_lock = state.player_manager.get_or_create(gid).await;
											let mut player = player_lock.write().await;
											player.voice_state = Some(voice_state.clone());

											if let Some(track) = player.current.clone()
												&& let Some(position) = state.take_restart_resume(gid)
											{
												player.seek(position);
												player.play();
												Some((track, position, player.volume, player.filters.clone()))
											} else {
												None
											}
										};

										voice_mgr.connect(gid, user_id, voice_state).await;

										if let Some((track, position, volume, filters)) = resume {
											let end_time = track.end_time;
											let playback_url = state.track_resolver.playback_url(&track).await;
											let mut voice_track = track;
											voice_track.info.uri = Some(playback_url);

											voice_mgr.play(gid, voice_track, position, volume, filters, end_time).await;
										}

										voice_mgr.request_player_update(gid);
									}
									.into_actor(self)
									.map(|_, _, _| {}),
								);
							}
						}
						crate::websocket_messages::IncomingMessage::LoadTracks { identifier } => {
							let state = self.state.clone();
							let identifier_clone = identifier.clone();
							let ctx_clone = ctx.address();
							ctx.spawn(
								async move {
									let result = state
										.track_resolver
										.load(&identifier_clone)
										.await
										.map(|(result, _)| result)
										.unwrap_or_else(|e| {
											tracing::error!("Failed to load tracks: {}", e);
											orpheus_track_resolver::LoadResult {
												load_type: orpheus_track_resolver::LoadType::Error,
												tracks: vec![],
												playlist_info: None,
											}
										});
									let load_type = match result.load_type {
										orpheus_track_resolver::LoadType::Track => "track",
										orpheus_track_resolver::LoadType::Playlist => "playlist",
										orpheus_track_resolver::LoadType::Search => "search",
										orpheus_track_resolver::LoadType::Empty => "empty",
										orpheus_track_resolver::LoadType::Error => "error",
									};
									let tracks: Vec<orpheus_core::Track> = result
										.tracks
										.into_iter()
										.map(|t| orpheus_core::Track {
											encoded: t.encoded,
											info: orpheus_core::TrackInfo {
												identifier: t.info.identifier,
												is_seekable: t.info.is_seekable,
												author: t.info.author,
												length: t.info.length,
												is_stream: t.info.is_stream,
												position: t.info.position,
												title: t.info.title,
												uri: t.info.uri,
												artwork_url: t.info.artwork_url,
												isrc: t.info.isrc,
												source_name: t.info.source_name,
											},
											start_time: t.start_time,
											end_time: t.end_time,
											plugin_info: serde_json::json!({}),
											user_data: serde_json::json!({}),
										})
										.collect();
									let data = match load_type {
										"track" => {
											if let Some(track) = tracks.into_iter().next() {
												crate::websocket_messages::LoadTracksResponseData::Track(Box::new(track))
											} else {
												crate::websocket_messages::LoadTracksResponseData::Search(vec![])
											}
										}
										"playlist" => {
											let playlist_info = result
												.playlist_info
												.map(|p| crate::websocket_messages::PlaylistInfoMsg {
													name: p.name,
													selected_track: p.selected_track,
												})
												.unwrap_or(crate::websocket_messages::PlaylistInfoMsg {
													name: None,
													selected_track: None,
												});
											crate::websocket_messages::LoadTracksResponseData::Playlist(
												crate::websocket_messages::PlaylistTracksDataMsg {
													info: playlist_info,
													plugin_info: Some(serde_json::json!({})),
													tracks,
												},
											)
										}
										_ => crate::websocket_messages::LoadTracksResponseData::Search(tracks),
									};
									let response = crate::websocket_messages::OutgoingMessage::LoadTracks(
										crate::websocket_messages::LoadTracksResponse {
											load_type: load_type.to_string(),
											data,
										},
									);

									if let Ok(json) = serde_json::to_string(&response) {
										ctx_clone.do_send(ws_message::Text(json));
									}
								}
								.into_actor(self)
								.map(|_, _, _| {}),
							);
						}
					}
				}
			}
			Ok(ws::Message::Ping(msg)) => {
				tracing::info!(">>> WS PING: {:?}", msg);
				ctx.pong(&msg)
			}
			Ok(ws::Message::Pong(_)) => {
				tracing::info!(">>> WS PONG");
			}
			Ok(ws::Message::Binary(data)) => {
				tracing::info!(">>> WS BINARY: {} bytes", data.len());
			}
			Ok(ws::Message::Close(_)) => {
				tracing::info!(">>> WS CLOSE for session {}", self.session_id);

				self.park_or_destroy();
				ctx.stop();
			}
			Err(e) => {
				tracing::warn!(">>> WS ERROR: {:?}", e);

				self.park_or_destroy();
				ctx.stop();
			}
			_ => {
				tracing::info!(">>> WS OTHER: {:?}", msg);
			}
		}
	}
}
