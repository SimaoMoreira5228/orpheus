use super::identifier::canonical_url;
use crate::{AlbumRef, ArtistRef, PlaylistRef, RichResults, Track, TrackInfo};

fn track_artists(value: &serde_json::Value) -> String {
	value
		.get("artists")
		.and_then(|artists| artists.as_array())
		.map(|artists| {
			artists
				.iter()
				.filter_map(|artist| artist.get("name").and_then(|name| name.as_str()))
				.collect::<Vec<_>>()
				.join(", ")
		})
		.filter(|author| !author.is_empty())
		.unwrap_or_else(|| "Unknown Artist".to_string())
}

fn first_image_url(value: &serde_json::Value) -> Option<String> {
	value
		.get("images")
		.and_then(|images| images.as_array())
		.and_then(|images| images.first())
		.and_then(|image| image.get("url"))
		.and_then(|url| url.as_str())
		.map(|url| url.to_string())
}

fn track_artwork(value: &serde_json::Value) -> Option<String> {
	value
		.get("album")
		.and_then(|album| album.get("images"))
		.or_else(|| value.get("images"))
		.and_then(|images| images.as_array())
		.and_then(|images| images.first())
		.and_then(|image| image.get("url"))
		.and_then(|url| url.as_str())
		.map(|url| url.to_string())
}

pub(super) fn track_info_from_json(kind: &str, id: &str, value: &serde_json::Value) -> Option<TrackInfo> {
	let title = value.get("name").and_then(|name| name.as_str())?;
	let duration_ms = value.get("duration_ms").and_then(|duration| duration.as_u64()).unwrap_or(0);

	Some(TrackInfo {
		identifier: canonical_url(kind, id),
		is_seekable: true,
		author: track_artists(value),
		length: duration_ms,
		is_stream: false,
		position: 0,
		title: title.to_string(),
		uri: Some(canonical_url(kind, id)),
		artwork_url: track_artwork(value),
		isrc: value
			.get("external_ids")
			.and_then(|ids| ids.get("isrc"))
			.and_then(|isrc| isrc.as_str())
			.map(|isrc| isrc.to_string()),
		source_name: "spotify".to_string(),
	})
}

pub(super) fn rich_search_results(json: &serde_json::Value, tracks: Vec<Track>) -> RichResults {
	let mut results = RichResults {
		tracks,
		..RichResults::empty()
	};

	let albums = json
		.get("albums")
		.and_then(|v| v.get("items"))
		.and_then(|v| v.as_array())
		.cloned()
		.unwrap_or_default();

	for album in &albums {
		let (Some(id), Some(name)) = (
			album.get("id").and_then(|v| v.as_str()),
			album.get("name").and_then(|v| v.as_str()),
		) else {
			continue;
		};

		results.albums.push(AlbumRef {
			id: id.to_string(),
			name: name.to_string(),
			artist: track_artists(album),
			artwork_url: first_image_url(album),
			uri: canonical_url("album", id),
		});
	}

	let artists = json
		.get("artists")
		.and_then(|v| v.get("items"))
		.and_then(|v| v.as_array())
		.cloned()
		.unwrap_or_default();

	for artist in &artists {
		let (Some(id), Some(name)) = (
			artist.get("id").and_then(|v| v.as_str()),
			artist.get("name").and_then(|v| v.as_str()),
		) else {
			continue;
		};

		results.artists.push(ArtistRef {
			id: id.to_string(),
			name: name.to_string(),
			artwork_url: first_image_url(artist),
			uri: canonical_url("artist", id),
		});
	}

	let playlists = json
		.get("playlists")
		.and_then(|v| v.get("items"))
		.and_then(|v| v.as_array())
		.cloned()
		.unwrap_or_default();

	for playlist in &playlists {
		let (Some(id), Some(name)) = (
			playlist.get("id").and_then(|v| v.as_str()),
			playlist.get("name").and_then(|v| v.as_str()),
		) else {
			continue;
		};

		results.playlists.push(PlaylistRef {
			id: id.to_string(),
			name: name.to_string(),
			author: playlist
				.get("owner")
				.and_then(|owner| owner.get("display_name"))
				.and_then(|name| name.as_str())
				.unwrap_or("Unknown")
				.to_string(),
			artwork_url: first_image_url(playlist),
			uri: canonical_url("playlist", id),
			track_count: playlist
				.get("tracks")
				.and_then(|tracks| tracks.get("total"))
				.and_then(|total| total.as_u64())
				.unwrap_or(0),
		});
	}

	results
}
