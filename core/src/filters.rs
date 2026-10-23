use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub volume: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub equalizer: Option<Vec<EqualizerBand>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub karaoke: Option<KaraokeFilter>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub timescale: Option<TimescaleFilter>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tremolo: Option<TremoloFilter>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub vibrato: Option<VibratoFilter>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rotation: Option<RotationFilter>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub distortion: Option<DistortionFilter>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel_mix: Option<ChannelMixFilter>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub low_pass: Option<LowPassFilter>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub normalization: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub plugin_filters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqualizerBand {
	pub band: i32,
	pub gain: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KaraokeFilter {
	pub level: f32,
	pub mono_level: f32,
	pub filter_band: f32,
	pub filter_width: f32,
}

impl Default for KaraokeFilter {
	fn default() -> Self {
		Self {
			level: 1.0,
			mono_level: 1.0,
			filter_band: 220.0,
			filter_width: 100.0,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimescaleFilter {
	pub speed: f64,
	pub pitch: f64,
	pub rate: f64,
}

impl Default for TimescaleFilter {
	fn default() -> Self {
		Self {
			speed: 1.0,
			pitch: 1.0,
			rate: 1.0,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TremoloFilter {
	pub frequency: f32,
	pub depth: f32,
}

impl Default for TremoloFilter {
	fn default() -> Self {
		Self {
			frequency: 2.0,
			depth: 0.5,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VibratoFilter {
	pub frequency: f32,
	pub depth: f32,
}

impl Default for VibratoFilter {
	fn default() -> Self {
		Self {
			frequency: 2.0,
			depth: 0.5,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RotationFilter {
	pub rotation_hz: f64,
}

impl Default for RotationFilter {
	fn default() -> Self {
		Self { rotation_hz: 0.0 }
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DistortionFilter {
	pub sin_offset: f32,
	pub sin_scale: f32,
	pub cos_offset: f32,
	pub cos_scale: f32,
	pub tan_offset: f32,
	pub tan_scale: f32,
	pub offset: f32,
	pub scale: f32,
}

impl Default for DistortionFilter {
	fn default() -> Self {
		Self {
			sin_offset: 0.0,
			sin_scale: 1.0,
			cos_offset: 0.0,
			cos_scale: 1.0,
			tan_offset: 0.0,
			tan_scale: 1.0,
			offset: 0.0,
			scale: 1.0,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelMixFilter {
	pub left_to_left: f32,
	pub left_to_right: f32,
	pub right_to_left: f32,
	pub right_to_right: f32,
}

impl Default for ChannelMixFilter {
	fn default() -> Self {
		Self {
			left_to_left: 1.0,
			left_to_right: 0.0,
			right_to_left: 0.0,
			right_to_right: 1.0,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LowPassFilter {
	pub smoothing: f32,
}

impl Default for LowPassFilter {
	fn default() -> Self {
		Self { smoothing: 20.0 }
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lavalink_filter_object_deserializes_with_wire_names() {
		let filters: Filters = serde_json::from_str(
			r#"{
				"volume": 1.0,
				"equalizer": [{"band": 0, "gain": 0.2}],
				"karaoke": {"level": 1.0, "monoLevel": 1.0, "filterBand": 220.0, "filterWidth": 100.0},
				"timescale": {"speed": 1.0, "pitch": 1.0, "rate": 1.0},
				"tremolo": {"frequency": 2.0, "depth": 0.5},
				"vibrato": {"frequency": 2.0, "depth": 0.5},
				"rotation": {"rotationHz": 0.0},
				"distortion": {
					"sinOffset": 0.0, "sinScale": 1.0, "cosOffset": 0.0, "cosScale": 1.0,
					"tanOffset": 0.0, "tanScale": 1.0, "offset": 0.0, "scale": 1.0
				},
				"channelMix": {"leftToLeft": 1.0, "leftToRight": 0.0, "rightToLeft": 0.0, "rightToRight": 1.0},
				"lowPass": {"smoothing": 20.0}
			}"#,
		)
		.expect("vendor filter object");

		assert_eq!(filters.volume, Some(1.0));
		assert_eq!(filters.equalizer.as_ref().expect("eq").len(), 1);

		let tremolo = filters.tremolo.as_ref().expect("tremolo");

		assert_eq!(tremolo.frequency, 2.0);
		assert_eq!(tremolo.depth, 0.5);

		let karaoke = filters.karaoke.as_ref().expect("karaoke");
		assert_eq!(karaoke.filter_band, 220.0);
		assert_eq!(karaoke.filter_width, 100.0);

		assert_eq!(filters.rotation.as_ref().expect("rotation").rotation_hz, 0.0);

		let distortion = filters.distortion.as_ref().expect("distortion");
		assert_eq!(distortion.sin_scale, 1.0);
		assert_eq!(distortion.tan_offset, 0.0);

		let mix = filters.channel_mix.as_ref().expect("channelMix");
		assert_eq!(mix.left_to_left, 1.0);
		assert_eq!(mix.right_to_right, 1.0);

		assert_eq!(filters.low_pass.as_ref().expect("lowPass").smoothing, 20.0);

		let round = serde_json::to_value(&filters).expect("round trip");

		assert_eq!(round["rotation"]["rotationHz"], 0.0);
		assert_eq!(round["karaoke"]["filterBand"], 220.0);
		assert_eq!(round["karaoke"]["filterWidth"], 100.0);
		assert_eq!(round["distortion"]["sinOffset"], 0.0);
		assert_eq!(round["channelMix"]["leftToLeft"], 1.0);
		assert_eq!(round["lowPass"]["smoothing"], 20.0);
	}

	#[test]
	fn null_stage_disables_and_missing_stage_stays_absent() {
		let filters: Filters =
			serde_json::from_str(r#"{"karaoke": null, "tremolo": {"depth": 0.0}}"#).expect("partial filters");

		assert!(filters.karaoke.is_none());
		assert!(filters.timescale.is_none());

		let tremolo = filters.tremolo.as_ref().expect("tremolo");
		assert_eq!(tremolo.depth, 0.0);
		assert_eq!(tremolo.frequency, 2.0);
	}
}
