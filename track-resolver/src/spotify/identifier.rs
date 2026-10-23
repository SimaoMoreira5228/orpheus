pub(crate) fn is_spotify_url(identifier: &str) -> bool {
	parse_spotify_id(identifier).is_some()
}

pub(super) fn is_spotify_search(identifier: &str) -> bool {
	identifier.to_ascii_lowercase().starts_with("spsearch:")
}

pub(super) fn parse_spotify_id(identifier: &str) -> Option<(String, String)> {
	if let Some(rest) = identifier.strip_prefix("spotify:") {
		let mut parts = rest.splitn(3, ':');

		match (parts.next(), parts.next()) {
			(Some(kind), Some(id)) => {
				let kind = kind.to_ascii_lowercase();

				if !matches!(kind.as_str(), "track" | "album" | "playlist" | "artist") {
					return None;
				}

				let id = id.split(['?', '#']).next().unwrap_or_default();

				if id.is_empty() {
					return None;
				}

				return Some((kind, id.to_string()));
			}
			_ => return None,
		}
	}

	let url = reqwest::Url::parse(identifier).ok()?;
	if !matches!(url.scheme(), "http" | "https") {
		return None;
	}

	if !matches!(url.host_str()?, "open.spotify.com" | "www.open.spotify.com") {
		return None;
	}

	let mut segments = url.path_segments()?;
	for segment in segments.by_ref() {
		let kind = segment.to_ascii_lowercase();
		if matches!(kind.as_str(), "track" | "album" | "playlist" | "artist") {
			let id = segments.next()?;
			if id.is_empty() {
				return None;
			}

			return Some((kind, id.to_string()));
		}
	}

	None
}

pub(super) fn canonical_url(kind: &str, id: &str) -> String {
	format!("https://open.spotify.com/{kind}/{id}")
}

pub(super) fn url_encode(input: &str) -> String {
	let mut encoded = String::new();

	for byte in input.bytes() {
		match byte {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
				encoded.push(byte as char);
			}
			_ => encoded.push_str(&format!("%{:02X}", byte)),
		}
	}

	encoded
}

pub(crate) fn mirror_query(title: &str, author: &str) -> String {
	match (author.trim(), title.trim()) {
		("", "") => String::new(),
		("", title) => title.to_string(),
		(author, "") => author.to_string(),
		(author, title) => format!("{author} - {title}"),
	}
}
