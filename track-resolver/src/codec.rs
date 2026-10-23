use base64::Engine;
use orpheus_core::{Track as CoreTrack, TrackInfo as CoreTrackInfo};
use thiserror::Error;

pub const ENCODE_VERSION: u8 = 3;

const VERSIONED_FLAG: u32 = 1;
const FLAG_SHIFT: u32 = 30;
const SIZE_MASK: u32 = 0x3FFF_FFFF;

#[derive(Debug, Error)]
pub enum CodecError {
	#[error("invalid base64: {0}")]
	InvalidBase64(String),

	#[error("truncated track message in {field}")]
	Truncated {
		field: &'static str,
	},

	#[error("zero-length track message")]
	EmptyMessage,

	#[error("unsupported track version: {0}")]
	UnsupportedVersion(u8),

	#[error("invalid utf-8 in {field}")]
	InvalidUtf8 {
		field: &'static str,
	},

	#[error("{field} out of range")]
	OutOfRange {
		field: &'static str,
	},

	#[error("{field} exceeds maximum wire length")]
	TooLong {
		field: &'static str,
	},
}

#[derive(Debug, Clone)]
pub struct DecodedTrack {
	pub track: CoreTrack,
	pub extras: Vec<u8>,
	pub version: u8,
}

struct Reader<'a> {
	data: &'a [u8],
	pos: usize,
}

impl<'a> Reader<'a> {
	fn new(data: &'a [u8]) -> Self {
		Self { data, pos: 0 }
	}

	fn remaining(&self) -> usize {
		self.data.len() - self.pos
	}

	fn take(&mut self, len: usize, field: &'static str) -> Result<&'a [u8], CodecError> {
		if self.remaining() < len {
			return Err(CodecError::Truncated { field });
		}

		let bytes = &self.data[self.pos..self.pos + len];
		self.pos += len;

		Ok(bytes)
	}

	fn read_u8(&mut self, field: &'static str) -> Result<u8, CodecError> {
		let bytes = self.take(1, field)?;

		Ok(bytes[0])
	}

	fn read_u16(&mut self, field: &'static str) -> Result<u16, CodecError> {
		let bytes = self.take(2, field)?;

		Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
	}

	fn read_i32(&mut self, field: &'static str) -> Result<i32, CodecError> {
		let bytes = self.take(4, field)?;

		Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
	}

	fn read_i64(&mut self, field: &'static str) -> Result<i64, CodecError> {
		let bytes = self.take(8, field)?;

		Ok(i64::from_be_bytes([
			bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
		]))
	}

	fn read_bool(&mut self, field: &'static str) -> Result<bool, CodecError> {
		let value = self.read_u8(field)?;

		Ok(value != 0)
	}

	fn read_string(&mut self, field: &'static str) -> Result<String, CodecError> {
		let len = usize::from(self.read_u16(field)?);
		let bytes = self.take(len, field)?;

		String::from_utf8(bytes.to_vec()).map_err(|_| CodecError::InvalidUtf8 { field })
	}

	fn read_nullable_string(&mut self, field: &'static str) -> Result<Option<String>, CodecError> {
		if !self.read_bool(field)? {
			return Ok(None);
		}

		Ok(Some(self.read_string(field)?))
	}
}

pub fn decode_track(encoded: &str) -> Result<DecodedTrack, CodecError> {
	let data = base64::engine::general_purpose::STANDARD
		.decode(encoded)
		.map_err(|e| CodecError::InvalidBase64(e.to_string()))?;

	let mut reader = Reader::new(&data);

	let header = reader.read_i32("header")? as u32;
	let versioned = (header >> FLAG_SHIFT) & VERSIONED_FLAG == VERSIONED_FLAG;

	if header & SIZE_MASK == 0 {
		return Err(CodecError::EmptyMessage);
	}

	let version = if versioned {
		let version = reader.read_u8("version")?;

		if version != 2 && version != 3 {
			return Err(CodecError::UnsupportedVersion(version));
		}

		version
	} else {
		1
	};

	let title = reader.read_string("title")?;
	let author = reader.read_string("author")?;
	let length = read_duration(reader.read_i64("length")?, "length")?;
	let identifier = reader.read_string("identifier")?;
	let is_stream = reader.read_bool("is_stream")?;

	let uri = if version >= 2 {
		reader.read_nullable_string("uri")?
	} else {
		None
	};

	let (artwork_url, isrc) = if version >= 3 {
		(
			reader.read_nullable_string("artwork_url")?,
			reader.read_nullable_string("isrc")?,
		)
	} else {
		(None, None)
	};

	let source_name = reader.read_string("source_name")?;

	if reader.remaining() < 8 {
		return Err(CodecError::Truncated { field: "position" });
	}

	let extras_end = reader.data.len() - 8;
	let extras = reader.data[reader.pos..extras_end].to_vec();

	reader.pos = extras_end;

	let position = read_duration(reader.read_i64("position")?, "position")?;

	let track = CoreTrack {
		encoded: Some(encoded.to_string()),
		info: CoreTrackInfo {
			identifier,
			is_seekable: !is_stream,
			author,
			length,
			is_stream,
			position,
			title,
			uri,
			artwork_url,
			isrc,
			source_name,
		},
		start_time: None,
		end_time: None,
		plugin_info: serde_json::json!({}),
		user_data: serde_json::json!({}),
	};

	Ok(DecodedTrack { track, extras, version })
}

pub fn encode_track(track: &CoreTrack, extras: &[u8]) -> Result<String, CodecError> {
	let mut body = Vec::new();

	body.push(ENCODE_VERSION);

	write_string(&mut body, &track.info.title, "title")?;
	write_string(&mut body, &track.info.author, "author")?;
	write_i64(&mut body, write_duration(track.info.length, "length")?);
	write_string(&mut body, &track.info.identifier, "identifier")?;

	body.push(u8::from(track.info.is_stream));

	write_nullable_string(&mut body, track.info.uri.as_deref().filter(|uri| !uri.is_empty()), "uri")?;
	write_nullable_string(&mut body, track.info.artwork_url.as_deref(), "artwork_url")?;
	write_nullable_string(&mut body, track.info.isrc.as_deref(), "isrc")?;
	write_string(&mut body, &track.info.source_name, "source_name")?;

	if extras.is_empty() && matches!(track.info.source_name.as_str(), "http" | "local") {
		// NOTE: real decoders always read a probe string for http/local tracks,
		// so fresh encodes must carry at least an empty one to stay decodable.
		write_string(&mut body, "", "probe_info")?;
	} else {
		body.extend_from_slice(extras);
	}

	write_i64(&mut body, write_duration(track.info.position, "position")?);

	let size = u32::try_from(body.len()).map_err(|_| CodecError::TooLong { field: "message" })?;

	if size & !SIZE_MASK != 0 {
		return Err(CodecError::TooLong { field: "message" });
	}

	let header = size | (VERSIONED_FLAG << FLAG_SHIFT);

	let mut message = Vec::with_capacity(4 + body.len());
	message.extend_from_slice(&header.to_be_bytes());
	message.extend_from_slice(&body);

	Ok(base64::engine::general_purpose::STANDARD.encode(message))
}

fn read_duration(value: i64, field: &'static str) -> Result<u64, CodecError> {
	if value < 0 {
		return Err(CodecError::OutOfRange { field });
	}

	Ok(value as u64)
}

fn write_duration(value: u64, field: &'static str) -> Result<i64, CodecError> {
	i64::try_from(value).map_err(|_| CodecError::OutOfRange { field })
}

fn write_i64(body: &mut Vec<u8>, value: i64) {
	body.extend_from_slice(&value.to_be_bytes());
}

fn write_string(body: &mut Vec<u8>, value: &str, field: &'static str) -> Result<(), CodecError> {
	let len = u16::try_from(value.len()).map_err(|_| CodecError::TooLong { field })?;

	body.extend_from_slice(&len.to_be_bytes());
	body.extend_from_slice(value.as_bytes());

	Ok(())
}

fn write_nullable_string(body: &mut Vec<u8>, value: Option<&str>, field: &'static str) -> Result<(), CodecError> {
	match value {
		None => body.push(0),
		Some(value) => {
			body.push(1);
			write_string(body, value, field)?;
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	const GOLDEN_V1: &str =
		"AAAATQAXTmV2ZXIgR29ubmEgR2l2ZSBZb3UgVXAAC1JpY2sgQXN0bGV5AAAAAAADPCAAC2RRdzR3OVdnWGNRAAAHeW91dHViZQAAAAAAADLI";

	const GOLDEN_V2: &str = "QAAAYwIADU1pZG5pZ2h0IENpdHkAA004MwAAAAAAA7U4AAlhYmMxMjNYWVoBAQAaaHR0cHM6Ly9leGFtcGxlLmNvbS9zdHJlYW0ABGh0dHAADW1wMywgNDQxMDAgSHoAAAAAAAATiA==";

	const GOLDEN_V3: &str = "QAAAtQMAD0JsaW5kaW5nIExpZ2h0cwAKVGhlIFdlZWtuZAAAAAAAAw1oAAs0TlJYeDZVOEFCUQABACtodHRwczovL3d3dy55b3V0dWJlLmNvbS93YXRjaD92PTROUlh4NlU4QUJRAQAwaHR0cHM6Ly9pLnl0aW1nLmNvbS92aS80TlJYeDZVOEFCUS9ocWRlZmF1bHQuanBnAQAMVVNVRzExOTAxNDk2AAd5b3V0dWJlAAAAAAAA7kg=";

	const GOLDEN_V3_NULLS: &str =
		"QAAAQAMACkxpdmUgUmFkaW8ADFNvbWUgU3RhdGlvbgAAAAAAAAAAAAdsaXZlMTIzAQAAAAAEaHR0cAAAAAAAAAAAAAA=";

	fn representative_info() -> CoreTrackInfo {
		CoreTrackInfo {
			identifier: "4NRXx6U8ABQ".to_string(),
			is_seekable: true,
			author: "The Weeknd àè🔥".to_string(),
			length: 200040,
			is_stream: false,
			position: 61000,
			title: "Blinding Lights".to_string(),
			uri: Some("https://www.youtube.com/watch?v=4NRXx6U8ABQ".to_string()),
			artwork_url: Some("https://i.ytimg.com/vi/4NRXx6U8ABQ/hqdefault.jpg".to_string()),
			isrc: Some("USUG11901496".to_string()),
			source_name: "youtube".to_string(),
		}
	}

	fn representative_track() -> CoreTrack {
		CoreTrack {
			encoded: None,
			info: representative_info(),
			start_time: None,
			end_time: None,
			plugin_info: serde_json::json!({}),
			user_data: serde_json::json!({}),
		}
	}

	#[test]
	fn golden_v1_decodes_every_field() {
		let decoded = decode_track(GOLDEN_V1).expect("v1 golden must decode");
		let info = &decoded.track.info;

		assert_eq!(decoded.version, 1);
		assert!(decoded.extras.is_empty());
		assert_eq!(info.title, "Never Gonna Give You Up");
		assert_eq!(info.author, "Rick Astley");
		assert_eq!(info.length, 212000);
		assert_eq!(info.identifier, "dQw4w9WgXcQ");
		assert!(!info.is_stream);
		assert!(info.is_seekable);
		assert_eq!(info.uri, None);
		assert_eq!(info.artwork_url, None);
		assert_eq!(info.isrc, None);
		assert_eq!(info.source_name, "youtube");
		assert_eq!(info.position, 13000);
		assert_eq!(decoded.track.encoded.as_deref(), Some(GOLDEN_V1));
	}

	#[test]
	fn golden_v2_decodes_every_field() {
		let decoded = decode_track(GOLDEN_V2).expect("v2 golden must decode");
		let info = &decoded.track.info;

		assert_eq!(decoded.version, 2);
		assert_eq!(info.title, "Midnight City");
		assert_eq!(info.author, "M83");
		assert_eq!(info.length, 243000);
		assert_eq!(info.identifier, "abc123XYZ");
		assert!(info.is_stream);
		assert!(!info.is_seekable);
		assert_eq!(info.uri.as_deref(), Some("https://example.com/stream"));
		assert_eq!(info.artwork_url, None);
		assert_eq!(info.isrc, None);
		assert_eq!(info.source_name, "http");
		assert_eq!(info.position, 5000);

		let probe = "mp3, 44100 Hz".as_bytes();
		let mut expected = Vec::with_capacity(2 + probe.len());
		expected.extend_from_slice(&(probe.len() as u16).to_be_bytes());
		expected.extend_from_slice(probe);

		assert_eq!(decoded.extras, expected);
	}

	#[test]
	fn golden_v3_decodes_every_field() {
		let decoded = decode_track(GOLDEN_V3).expect("v3 golden must decode");
		let info = &decoded.track.info;

		assert_eq!(decoded.version, 3);
		assert!(decoded.extras.is_empty());
		assert_eq!(info.title, "Blinding Lights");
		assert_eq!(info.author, "The Weeknd");
		assert_eq!(info.length, 200040);
		assert_eq!(info.identifier, "4NRXx6U8ABQ");
		assert!(!info.is_stream);
		assert!(info.is_seekable);
		assert_eq!(info.uri.as_deref(), Some("https://www.youtube.com/watch?v=4NRXx6U8ABQ"));
		assert_eq!(
			info.artwork_url.as_deref(),
			Some("https://i.ytimg.com/vi/4NRXx6U8ABQ/hqdefault.jpg")
		);
		assert_eq!(info.isrc.as_deref(), Some("USUG11901496"));
		assert_eq!(info.source_name, "youtube");
		assert_eq!(info.position, 61000);
	}

	#[test]
	fn golden_v3_nulls_decodes() {
		let decoded = decode_track(GOLDEN_V3_NULLS).expect("v3 nulls golden must decode");
		let info = &decoded.track.info;

		assert_eq!(decoded.version, 3);
		assert_eq!(info.title, "Live Radio");
		assert_eq!(info.author, "Some Station");
		assert_eq!(info.length, 0);
		assert_eq!(info.identifier, "live123");
		assert!(info.is_stream);
		assert!(!info.is_seekable);
		assert_eq!(info.uri, None);
		assert_eq!(info.artwork_url, None);
		assert_eq!(info.isrc, None);
		assert_eq!(info.source_name, "http");
		assert_eq!(info.position, 0);
		assert_eq!(decoded.extras, vec![0, 0]);
	}

	#[test]
	fn reencode_preserves_extras_verbatim() {
		for golden in [GOLDEN_V3, GOLDEN_V3_NULLS] {
			let decoded = decode_track(golden).expect("golden must decode");
			let reencoded = encode_track(&decoded.track, &decoded.extras).expect("re-encode must succeed");

			assert_eq!(reencoded, golden);
		}
	}

	#[test]
	fn older_versions_upgrade_to_v3_with_same_fields() {
		for golden in [GOLDEN_V1, GOLDEN_V2] {
			let decoded = decode_track(golden).expect("golden must decode");
			let reencoded = encode_track(&decoded.track, &decoded.extras).expect("re-encode must succeed");
			let upgraded = decode_track(&reencoded).expect("upgraded blob must decode");

			assert_eq!(upgraded.version, 3);
			assert_eq!(upgraded.extras, decoded.extras);
			assert_eq!(upgraded.track.info.title, decoded.track.info.title);
			assert_eq!(upgraded.track.info.author, decoded.track.info.author);
			assert_eq!(upgraded.track.info.length, decoded.track.info.length);
			assert_eq!(upgraded.track.info.identifier, decoded.track.info.identifier);
			assert_eq!(upgraded.track.info.is_stream, decoded.track.info.is_stream);
			assert_eq!(upgraded.track.info.uri, decoded.track.info.uri);
			assert_eq!(upgraded.track.info.source_name, decoded.track.info.source_name);
			assert_eq!(upgraded.track.info.position, decoded.track.info.position);
		}
	}

	#[test]
	fn round_trip_representative_track() {
		let track = representative_track();
		let encoded = encode_track(&track, &[]).expect("encode must succeed");
		let decoded = decode_track(&encoded).expect("round trip must decode");

		assert_eq!(decoded.version, 3);
		assert!(decoded.extras.is_empty());
		assert_eq!(decoded.track.info.title, track.info.title);
		assert_eq!(decoded.track.info.author, track.info.author);
		assert_eq!(decoded.track.info.length, track.info.length);
		assert_eq!(decoded.track.info.identifier, track.info.identifier);
		assert_eq!(decoded.track.info.is_stream, track.info.is_stream);
		assert_eq!(decoded.track.info.is_seekable, track.info.is_seekable);
		assert_eq!(decoded.track.info.uri, track.info.uri);
		assert_eq!(decoded.track.info.artwork_url, track.info.artwork_url);
		assert_eq!(decoded.track.info.isrc, track.info.isrc);
		assert_eq!(decoded.track.info.source_name, track.info.source_name);
		assert_eq!(decoded.track.info.position, track.info.position);

		let reencoded = encode_track(&decoded.track, &decoded.extras).expect("re-encode must succeed");

		assert_eq!(reencoded, encoded);
	}

	#[test]
	fn round_trip_null_uri_stream() {
		let mut track = representative_track();

		track.info.uri = None;
		track.info.artwork_url = None;
		track.info.isrc = None;
		track.info.is_stream = true;
		track.info.is_seekable = false;
		track.info.length = 0;
		track.info.position = 0;
		track.info.source_name = "bandcamp".to_string();

		let encoded = encode_track(&track, &[]).expect("encode must succeed");
		let decoded = decode_track(&encoded).expect("round trip must decode");

		assert_eq!(decoded.track.info.uri, None);
		assert_eq!(decoded.track.info.artwork_url, None);
		assert_eq!(decoded.track.info.isrc, None);
		assert!(decoded.track.info.is_stream);
		assert!(!decoded.track.info.is_seekable);
	}

	#[test]
	fn http_encode_carries_empty_probe_extras() {
		let mut track = representative_track();

		track.info.source_name = "http".to_string();

		let encoded = encode_track(&track, &[]).expect("encode must succeed");
		let decoded = decode_track(&encoded).expect("http blob must decode");

		assert_eq!(decoded.extras, vec![0, 0]);
		assert_eq!(decoded.track.info.position, track.info.position);
	}

	#[test]
	fn rejects_invalid_base64() {
		assert!(matches!(decode_track("!!!not-base64!!!"), Err(CodecError::InvalidBase64(_))));
	}

	#[test]
	fn rejects_truncated_header() {
		assert!(matches!(decode_track("QQ=="), Err(CodecError::Truncated { .. })));
	}

	#[test]
	fn rejects_truncated_body() {
		let raw = base64::engine::general_purpose::STANDARD
			.decode(GOLDEN_V3)
			.expect("golden must be valid base64");
		let cut = base64::engine::general_purpose::STANDARD.encode(&raw[..raw.len() - 3]);

		assert!(matches!(decode_track(&cut), Err(CodecError::Truncated { .. })));
	}

	#[test]
	fn rejects_zero_length_message() {
		let encoded = base64::engine::general_purpose::STANDARD.encode([0x40, 0x00, 0x00, 0x00]);

		assert!(matches!(decode_track(&encoded), Err(CodecError::EmptyMessage)));
	}

	#[test]
	fn rejects_unsupported_version() {
		let mut raw = base64::engine::general_purpose::STANDARD
			.decode(GOLDEN_V3)
			.expect("golden must be valid base64");

		raw[4] = 9;

		let encoded = base64::engine::general_purpose::STANDARD.encode(raw);

		assert!(matches!(decode_track(&encoded), Err(CodecError::UnsupportedVersion(9))));
	}

	#[test]
	fn rejects_negative_length() {
		let mut raw = vec![0x40, 0x00, 0x00, 0x0D, 0x03, 0x00, 0x00, 0x00, 0x00];
		raw.extend_from_slice(&(-1i64).to_be_bytes());

		let encoded = base64::engine::general_purpose::STANDARD.encode(raw);

		assert!(matches!(
			decode_track(&encoded),
			Err(CodecError::OutOfRange { field: "length" })
		));
	}

	#[test]
	fn rejects_legacy_json_blob() {
		let encoded = base64::engine::general_purpose::STANDARD.encode(r#"{"info":{"title":"x"}}"#);

		assert!(decode_track(&encoded).is_err());
	}

	#[test]
	fn rejects_overlong_string_on_encode() {
		let mut track = representative_track();

		track.info.title = "x".repeat(70_000);

		assert!(matches!(
			encode_track(&track, &[]),
			Err(CodecError::TooLong { field: "title" })
		));
	}
}
