use crate::*;

#[test]
fn rich_results_merge_preserves_source_order() {
	let spotify = RichResults {
		tracks: vec![new_track(TrackInfo {
			identifier: "https://open.spotify.com/track/1".to_string(),
			is_seekable: true,
			author: "a".to_string(),
			length: 0,
			is_stream: false,
			position: 0,
			title: "t".to_string(),
			uri: Some("https://open.spotify.com/track/1".to_string()),
			artwork_url: None,
			isrc: None,
			source_name: "spotify".to_string(),
		})],
		albums: vec![AlbumRef {
			id: "a".to_string(),
			name: "album".to_string(),
			artist: "a".to_string(),
			artwork_url: None,
			uri: "https://open.spotify.com/album/a".to_string(),
		}],
		..RichResults::empty()
	};

	let youtube = RichResults {
		tracks: vec![new_track(TrackInfo {
			identifier: "https://www.youtube.com/watch?v=x".to_string(),
			is_seekable: true,
			author: "b".to_string(),
			length: 0,
			is_stream: false,
			position: 0,
			title: "u".to_string(),
			uri: Some("https://www.youtube.com/watch?v=x".to_string()),
			artwork_url: None,
			isrc: None,
			source_name: "youtube".to_string(),
		})],
		..RichResults::empty()
	};

	let merged = RichResults::merge(vec![spotify, RichResults::empty(), youtube]);

	assert_eq!(merged.tracks.len(), 2);
	assert_eq!(merged.tracks[0].info.source_name, "spotify");
	assert_eq!(merged.tracks[1].info.source_name, "youtube");
	assert_eq!(merged.albums.len(), 1);
	assert!(merged.artists.is_empty());
}

fn track_info(uri: Option<&str>) -> TrackInfo {
	TrackInfo {
		identifier: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
		is_seekable: true,
		author: "author".to_string(),
		length: 0,
		is_stream: false,
		position: 0,
		title: "title".to_string(),
		uri: uri.map(|value| value.to_string()),
		artwork_url: None,
		isrc: None,
		source_name: "youtube".to_string(),
	}
}

fn load_result_with_uri(uri: Option<&str>) -> LoadResult {
	LoadResult {
		load_type: LoadType::Track,
		tracks: vec![new_track(track_info(uri))],
		playlist_info: None,
	}
}

#[test]
fn watch_url_needs_resolve() {
	assert!(needs_stream_resolve("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
}

#[test]
fn short_url_needs_resolve() {
	assert!(needs_stream_resolve("https://youtu.be/dQw4w9WgXcQ"));
}

#[test]
fn music_subdomain_watch_url_needs_resolve() {
	assert!(needs_stream_resolve("https://music.youtube.com/watch?v=dQw4w9WgXcQ"));
}

#[test]
fn googlevideo_url_passes_through() {
	assert!(!needs_stream_resolve(
		"https://rr1---sn-abc123.googlevideo.com/videoplayback?expire=999&mime=audio%2Fwebm"
	));
}

#[test]
fn direct_media_url_passes_through() {
	assert!(!needs_stream_resolve("https://example.com/song.mp3"));
}

#[test]
fn youtube_domain_in_path_does_not_resolve() {
	assert!(!needs_stream_resolve("https://example.com/watch?v=youtube.com"));
}

#[test]
fn empty_uri_passes_through() {
	// NOTE: play sites already skip tracks without a playable URI, so the empty
	// value falls back unchanged instead of triggering a re-resolve.
	assert!(!needs_stream_resolve(""));
}

#[test]
fn expired_stream_url_needs_resolve() {
	const NOW: u64 = 1_800_000_000;

	assert!(baked_stream_is_expired(
		"https://rr1---sn-abc123.googlevideo.com/videoplayback?expire=1799999999&mime=audio%2Fwebm",
		NOW
	));
	assert!(baked_stream_is_expired(
		"https://rr1---sn-abc123.googlevideo.com/videoplayback?mime=audio%2Fwebm&expires=1799999999",
		NOW
	));
	assert!(baked_stream_is_expired(
		"https://rr1---sn-abc123.googlevideo.com/videoplayback?EXPIRE=1799999999",
		NOW
	));
}

#[test]
fn fresh_stream_url_passes_through() {
	const NOW: u64 = 1_800_000_000;

	assert!(!baked_stream_is_expired(
		"https://rr1---sn-abc123.googlevideo.com/videoplayback?expire=1800003600&mime=audio%2Fwebm",
		NOW
	));
	assert!(!baked_stream_is_expired("https://example.com/song.mp3", NOW));
	assert!(!baked_stream_is_expired("", NOW));
}

#[test]
fn expiring_soon_counts_as_expired() {
	const NOW: u64 = 1_800_000_000;

	assert!(baked_stream_is_expired(
		"https://rr1---sn-abc123.googlevideo.com/videoplayback?expire=1800000299",
		NOW
	));
	assert!(!baked_stream_is_expired(
		"https://rr1---sn-abc123.googlevideo.com/videoplayback?expire=1800000301",
		NOW
	));
}

#[test]
fn unparseable_expiry_stays_fresh() {
	const NOW: u64 = 1_800_000_000;

	assert!(!baked_stream_is_expired("https://example.com/videoplayback?expire=soon", NOW));
	assert!(!baked_stream_is_expired("https://example.com/videoplayback?expire=-5", NOW));
	assert!(!baked_stream_is_expired(
		"https://example.com/videoplayback?mime=audio#expire=1",
		NOW
	));
}

#[test]
fn fresh_url_wins_over_baked() {
	let loaded: Result<LoadResult> = Ok(load_result_with_uri(Some(
		"https://rr1---sn-x.googlevideo.com/videoplayback?fresh=1",
	)));

	assert_eq!(
		select_playback_uri("https://www.youtube.com/watch?v=dQw4w9WgXcQ", &loaded),
		"https://rr1---sn-x.googlevideo.com/videoplayback?fresh=1"
	);
}

#[test]
fn empty_tracks_fall_back_to_baked() {
	let loaded: Result<LoadResult> = Ok(LoadResult {
		load_type: LoadType::Track,
		tracks: vec![],
		playlist_info: None,
	});

	assert_eq!(
		select_playback_uri("https://www.youtube.com/watch?v=dQw4w9WgXcQ", &loaded),
		"https://www.youtube.com/watch?v=dQw4w9WgXcQ"
	);
}

#[test]
fn missing_or_blank_fresh_url_falls_back_to_baked() {
	for uri in [None, Some("")] {
		let loaded: Result<LoadResult> = Ok(load_result_with_uri(uri));

		assert_eq!(
			select_playback_uri("https://www.youtube.com/watch?v=dQw4w9WgXcQ", &loaded),
			"https://www.youtube.com/watch?v=dQw4w9WgXcQ"
		);
	}
}

#[test]
fn load_error_falls_back_to_baked() {
	let loaded: Result<LoadResult> = Err(ResolverError::NoAudioStream);

	assert_eq!(
		select_playback_uri("https://www.youtube.com/watch?v=dQw4w9WgXcQ", &loaded),
		"https://www.youtube.com/watch?v=dQw4w9WgXcQ"
	);
}
