use actix_web::web;

use crate::errors::{json_error_handler, query_error_handler};

pub mod diagnostics;
pub mod migration;
pub mod players;
pub mod plugins;
pub mod presets;
pub mod queue;
pub mod runtime_config;
pub mod server;
pub mod sessions;
pub mod tracks;

pub use migration::{export_player, import_player};
pub use players::{
	NoReplaceQuery, PlayerOptions, PlayerResponse, PlayerStateResponse, PlayerUpdate, UpdatePlayerTrack, VoiceStateResponse,
	delete_player, get_all_players, get_player, update_player,
};
pub use presets::{delete_preset, get_preset, list_presets, put_preset};
pub use queue::{
	AddToQueueRequest, QueuePositionResponse, QueueResponse, QueueUpdateRequest, add_to_queue, get_queue, remove_from_queue,
	update_queue,
};
pub use server::{
	CpuInfo, FreeAddressRequest, MemoryInfo, Stats, VersionResponse, free_address, free_all, get_info, get_metrics,
	get_routeplanner_status, get_stats, get_version,
};
pub use sessions::{SessionUpdateRequest, SessionUpdateResponse, VoiceChannelRequest, set_voice_channel, update_session};
pub use tracks::{
	DecodeTrackQuery, DecodeTracksRequest, DryRunItem, LoadFailure, LoadResultType, LoadTracksQuery, LoadTracksResponse,
	LoadTracksResponseData, PlaylistInfo, PlaylistTracksData, RichSearchQuery, RichSearchResponse, decode_track,
	decode_tracks, load_search, load_tracks, load_tracks_batch, load_tracks_dry_run,
};

pub fn configure(cfg: &mut web::ServiceConfig) {
	cfg.app_data(web::QueryConfig::default().error_handler(query_error_handler))
		.app_data(web::JsonConfig::default().error_handler(json_error_handler))
		.service(
			web::resource("/v4/loadtracks")
				.route(web::get().to(tracks::load_tracks))
				.route(web::post().to(tracks::load_tracks_batch)),
		)
		.service(web::resource("/v4/loadtracks/dry-run").route(web::post().to(tracks::load_tracks_dry_run)))
		.service(
			web::resource("/v4/config")
				.route(web::get().to(runtime_config::get_runtime_config))
				.route(web::put().to(runtime_config::replace_runtime_config)),
		)
		.service(web::resource("/v4/diagnostics/voice/{guild_id}").route(web::get().to(diagnostics::get_voice_diagnostics)))
		.service(web::resource("/v4/loadsearch").route(web::get().to(tracks::load_search)))
		.service(web::resource("/v4/decodetrack").route(web::get().to(tracks::decode_track)))
		.service(web::resource("/v4/decodetracks").route(web::post().to(tracks::decode_tracks)))
		.service(web::resource("/v4/sessions/{session_id}/players").route(web::get().to(players::get_all_players)))
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}")
				.route(web::get().to(players::get_player))
				.route(web::patch().to(players::update_player))
				.route(web::delete().to(players::delete_player)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/queue")
				.route(web::get().to(queue::get_queue))
				.route(web::post().to(queue::add_to_queue))
				.route(web::patch().to(queue::update_queue)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/queue/{position}")
				.route(web::delete().to(queue::remove_from_queue)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/export")
				.route(web::get().to(migration::export_player)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/import")
				.route(web::post().to(migration::import_player)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/voice-channel")
				.route(web::put().to(sessions::set_voice_channel)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/sponsorblock/categories")
				.route(web::get().to(server::plugin_unavailable))
				.route(web::put().to(server::plugin_unavailable))
				.route(web::delete().to(server::plugin_unavailable)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/track/lyrics")
				.route(web::get().to(server::plugin_unavailable)),
		)
		.service(web::resource("/v4/lyrics").route(web::get().to(server::plugin_unavailable)))
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/chapters")
				.route(web::get().to(server::plugin_unavailable)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/youtube/config")
				.route(web::get().to(server::plugin_unavailable)),
		)
		.service(
			web::resource("/v4/sessions/{session_id}/players/{guild_id}/mix")
				.route(web::get().to(server::plugin_unavailable))
				.route(web::patch().to(server::plugin_unavailable)),
		)
		.service(web::resource("/v4/sessions/{session_id}").route(web::patch().to(sessions::update_session)))
		.service(web::resource("/v4/info").route(web::get().to(server::get_info)))
		.service(web::resource("/v4/plugins").route(web::get().to(plugins::list_plugins)))
		.service(
			web::resource("/v4/plugins/{name}")
				.route(web::post().to(plugins::load_plugin))
				.route(web::delete().to(plugins::unload_plugin)),
		)
		.service(web::resource("/v4/plugins/{name}/reload").route(web::post().to(plugins::reload_plugin)))
		.service(web::resource("/v4/plugins/{name}/config").route(web::put().to(plugins::set_plugin_config)))
		.service(web::resource("/v4/plugins/{name}/{rest:.*}").route(web::route().to(plugins::dispatch_route)))
		.service(web::resource("/v4/filters/presets").route(web::get().to(presets::list_presets)))
		.service(
			web::resource("/v4/filters/presets/{name}")
				.route(web::put().to(presets::put_preset))
				.route(web::get().to(presets::get_preset))
				.route(web::delete().to(presets::delete_preset)),
		)
		.service(web::resource("/v4/stats").route(web::get().to(server::get_stats)))
		.service(web::resource("/version").route(web::get().to(server::get_version)))
		.service(web::resource("/v4/routeplanner/status").route(web::get().to(server::get_routeplanner_status)))
		.service(web::resource("/v4/routeplanner/free/address").route(web::post().to(server::free_address)))
		.service(web::resource("/v4/routeplanner/free/all").route(web::post().to(server::free_all)))
		.service(web::resource("/metrics").route(web::get().to(server::get_metrics)));
}
