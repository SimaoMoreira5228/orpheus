mod validation;

use orpheus_core::{
	ChannelMixFilter, DistortionFilter, EqualizerBand, Filters, KaraokeFilter, LowPassFilter, RotationFilter,
	TimescaleFilter, TremoloFilter, VibratoFilter,
};
pub use validation::validate_filters;

pub const OUTPUT_SAMPLE_RATE: u32 = 48000;

pub(crate) const FFMPEG_USER_AGENT: &str =
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

const EQUALIZER_FREQUENCIES: [u32; 15] = [
	25, 40, 63, 100, 160, 250, 400, 630, 1000, 1600, 2500, 4000, 6300, 10000, 16000,
];

pub fn volume_fraction(volume: u32) -> f32 {
	volume.min(1000) as f32 / 1000.0
}

fn format_float(value: f32) -> String {
	format!("{value}")
}

fn format_double(value: f64) -> String {
	format!("{value}")
}

fn gain(value: f32) -> String {
	format_float(value)
}

pub fn volume_stage(volume: u32, filter_volume: Option<f32>) -> Option<String> {
	let effective = volume_fraction(volume) * filter_volume.unwrap_or(1.0);

	if (effective - 1.0).abs() <= f32::EPSILON {
		return None;
	}

	Some(format!("volume={}", format_float(effective)))
}

pub fn equalizer_stage(bands: &[EqualizerBand]) -> Option<String> {
	let mut sections = Vec::new();

	for band in bands {
		if band.gain == 0.0 {
			continue;
		}

		let Some(frequency) = EQUALIZER_FREQUENCIES.get(band.band as usize) else {
			continue;
		};

		for channel in ["c0", "c1"] {
			sections.push(format!("{channel} f={frequency} w=1 g={}", format_float(band.gain)));
		}
	}

	if sections.is_empty() {
		return None;
	}

	Some(format!("anequalizer={}", sections.join("|")))
}

pub fn karaoke_stage(filter: &KaraokeFilter) -> String {
	let own = gain(filter.level - filter.mono_level * 0.5);
	let other = gain(filter.mono_level * -0.5);

	let pan = format!("pan=stereo|c0={own}*c0+{other}*c1|c1={other}*c0+{own}*c1");

	let reject = format!(
		"bandreject=f={}:t=h:w={}",
		format_float(filter.filter_band),
		format_float(filter.filter_width),
	);

	format!("{pan},{reject}")
}

pub fn timescale_stage(filter: &TimescaleFilter) -> Option<String> {
	if filter.speed == 1.0 && filter.pitch == 1.0 && filter.rate == 1.0 {
		return None;
	}

	let mut stages = Vec::new();

	let pitch_rate = filter.pitch * filter.rate;

	if pitch_rate != 1.0 {
		let resampled = (f64::from(OUTPUT_SAMPLE_RATE) * pitch_rate).round().max(1.0) as u64;
		stages.push(format!("asetrate=r={resampled}"));
		stages.push(format!("aresample={OUTPUT_SAMPLE_RATE}"));
	}

	stages.extend(atempo_chain(filter.speed));

	if stages.is_empty() {
		return None;
	}

	Some(stages.join(","))
}

fn atempo_chain(speed: f64) -> Vec<String> {
	if speed == 1.0 || speed <= 0.0 {
		return Vec::new();
	}

	let mut factors = Vec::new();
	let mut remaining = speed;

	while remaining > 2.0 {
		factors.push(2.0);
		remaining /= 2.0;
	}

	while remaining < 0.5 {
		factors.push(0.5);
		remaining /= 0.5;
	}

	factors.push(remaining);

	factors
		.into_iter()
		.map(|factor| format!("atempo=tempo={}", format_double(factor)))
		.collect()
}

pub fn tremolo_stage(filter: &TremoloFilter) -> Option<String> {
	if filter.depth == 0.0 {
		return None;
	}

	Some(format!(
		"tremolo=f={}:d={}",
		format_float(filter.frequency.clamp(0.1, 20000.0)),
		format_float(filter.depth.clamp(0.0, 1.0)),
	))
}

pub fn vibrato_stage(filter: &VibratoFilter) -> Option<String> {
	if filter.depth == 0.0 {
		return None;
	}

	Some(format!(
		"vibrato=f={}:d={}",
		format_float(filter.frequency.clamp(0.1, 20000.0)),
		format_float(filter.depth.clamp(0.0, 1.0)),
	))
}

pub fn rotation_stage(filter: &RotationFilter) -> Option<String> {
	if filter.rotation_hz == 0.0 {
		return None;
	}

	Some(format!(
		"apulsator=hz={}",
		format_double(filter.rotation_hz.clamp(0.01, 100.0))
	))
}

// NOTE: ffmpeg has no per-sample sin/cos/tan waveshaper, so only the overall
// drive (scale) and bias (offset) ride acrusher; the trig stages shape the
// enable check but cannot be expressed in -af.
pub fn distortion_stage(filter: &DistortionFilter) -> Option<String> {
	let trig_active = filter.sin_offset != 0.0
		|| filter.sin_scale != 1.0
		|| filter.cos_offset != 0.0
		|| filter.cos_scale != 1.0
		|| filter.tan_offset != 0.0
		|| filter.tan_scale != 1.0;

	if !trig_active && filter.offset == 0.0 && filter.scale == 1.0 {
		return None;
	}

	let level_in = format_float(filter.scale.clamp(0.015625, 64.0));

	let dc = if filter.offset == 0.0 {
		1.0
	} else {
		filter.offset.clamp(0.25, 4.0)
	};

	Some(format!("acrusher=level_in={level_in}:dc={}:mix=1", format_float(dc)))
}

pub fn channel_mix_stage(filter: &ChannelMixFilter) -> Option<String> {
	if filter.left_to_left == 1.0
		&& filter.left_to_right == 0.0
		&& filter.right_to_left == 0.0
		&& filter.right_to_right == 1.0
	{
		return None;
	}

	let left_to_left = gain(filter.left_to_left);
	let left_to_right = gain(filter.left_to_right);
	let right_to_left = gain(filter.right_to_left);
	let right_to_right = gain(filter.right_to_right);

	Some(format!(
		"pan=stereo|c0={left_to_left}*c0+{right_to_left}*c1|c1={left_to_right}*c0+{right_to_right}*c1"
	))
}

pub fn low_pass_stage(filter: &LowPassFilter) -> Option<String> {
	if filter.smoothing <= 1.0 {
		return None;
	}

	Some(format!("lowpass=f={}", format_float(filter.smoothing)))
}

pub fn normalization_stage(enabled: bool) -> Option<String> {
	if !enabled {
		return None;
	}

	Some("loudnorm=I=-16:TP=-1.5:LRA=11".to_string())
}

pub fn build_filter_chain(volume: u32, filters: &Filters) -> Option<String> {
	let mut stages = Vec::new();

	if let Some(stage) = volume_stage(volume, filters.volume) {
		stages.push(stage);
	}

	if let Some(bands) = filters.equalizer.as_ref()
		&& let Some(stage) = equalizer_stage(bands)
	{
		stages.push(stage);
	}

	if let Some(filter) = filters.karaoke.as_ref() {
		stages.push(karaoke_stage(filter));
	}

	if let Some(filter) = filters.timescale.as_ref()
		&& let Some(stage) = timescale_stage(filter)
	{
		stages.push(stage);
	}

	if let Some(filter) = filters.tremolo.as_ref()
		&& let Some(stage) = tremolo_stage(filter)
	{
		stages.push(stage);
	}

	if let Some(filter) = filters.vibrato.as_ref()
		&& let Some(stage) = vibrato_stage(filter)
	{
		stages.push(stage);
	}

	if let Some(filter) = filters.rotation.as_ref()
		&& let Some(stage) = rotation_stage(filter)
	{
		stages.push(stage);
	}

	if let Some(filter) = filters.distortion.as_ref()
		&& let Some(stage) = distortion_stage(filter)
	{
		stages.push(stage);
	}

	if let Some(filter) = filters.channel_mix.as_ref()
		&& let Some(stage) = channel_mix_stage(filter)
	{
		stages.push(stage);
	}

	if let Some(filter) = filters.low_pass.as_ref()
		&& let Some(stage) = low_pass_stage(filter)
	{
		stages.push(stage);
	}

	if filters.normalization == Some(true)
		&& let Some(stage) = normalization_stage(true)
	{
		stages.push(stage);
	}

	if stages.is_empty() {
		return None;
	}

	Some(stages.join(","))
}

pub fn ffmpeg_input_args(track_uri: &str, seek_ms: u64, volume: u32, filters: &Filters, end_ms: Option<u64>) -> Vec<String> {
	let mut args: Vec<String> = vec!["-user_agent".into(), FFMPEG_USER_AGENT.into()];

	if seek_ms > 0 {
		args.push("-ss".into());
		args.push(format!("{}", seek_ms as f64 / 1000.0));
	}

	args.extend(["-i".into(), track_uri.to_string()]);

	if let Some(end) = end_ms {
		args.push("-to".into());
		args.push(format!("{}", end as f64 / 1000.0));
	}

	if let Some(chain) = build_filter_chain(volume, filters) {
		args.push("-af".into());
		args.push(chain);
	}

	args.extend([
		"-vn".into(),
		"-f".into(),
		"s16le".into(),
		"-ac".into(),
		"2".into(),
		"-ar".into(),
		OUTPUT_SAMPLE_RATE.to_string(),
		"-acodec".into(),
		"pcm_s16le".into(),
		"-".into(),
	]);

	args
}

#[cfg(test)]
mod tests {
	use super::*;

	fn empty_filters() -> Filters {
		Filters::default()
	}

	#[test]
	fn volume_fraction_maps_lavalink_range_to_gain() {
		assert_eq!(volume_fraction(0), 0.0);
		assert_eq!(volume_fraction(500), 0.5);
		assert_eq!(volume_fraction(1000), 1.0);
		assert_eq!(volume_fraction(1500), 1.0);
	}

	#[test]
	fn volume_stage_silences_at_zero_and_skips_unity() {
		assert_eq!(volume_stage(0, None).as_deref(), Some("volume=0"));
		assert_eq!(volume_stage(1000, None), None);
		assert_eq!(volume_stage(500, None).as_deref(), Some("volume=0.5"));
	}

	#[test]
	fn volume_stage_combines_player_volume_with_filter_volume() {
		assert_eq!(volume_stage(1000, Some(0.5)).as_deref(), Some("volume=0.5"));
		assert_eq!(volume_stage(500, Some(0.5)).as_deref(), Some("volume=0.25"));
		assert_eq!(volume_stage(1000, Some(1.0)), None);
	}

	#[test]
	fn null_filter_stages_disable_while_values_build_chain_in_order() {
		let mut filters = empty_filters();
		filters.volume = None;
		filters.tremolo = Some(TremoloFilter {
			frequency: 2.0,
			depth: 0.0,
		});
		filters.rotation = Some(RotationFilter { rotation_hz: 0.0 });
		filters.channel_mix = Some(ChannelMixFilter::default());
		filters.low_pass = Some(LowPassFilter { smoothing: 0.5 });

		assert_eq!(build_filter_chain(1000, &filters), None);

		filters.tremolo = Some(TremoloFilter {
			frequency: 4.0,
			depth: 0.5,
		});
		filters.low_pass = Some(LowPassFilter { smoothing: 4000.0 });

		let chain = build_filter_chain(500, &filters).expect("chain");

		assert_eq!(chain, "volume=0.5,tremolo=f=4:d=0.5,lowpass=f=4000");
	}

	#[test]
	fn full_filter_object_replaces_chain_in_fixed_order() {
		let filters = Filters {
			volume: Some(1.0),
			equalizer: Some(vec![EqualizerBand { band: 0, gain: 0.2 }]),
			karaoke: Some(KaraokeFilter::default()),
			timescale: Some(TimescaleFilter {
				speed: 1.2,
				pitch: 1.0,
				rate: 1.0,
			}),
			tremolo: Some(TremoloFilter {
				frequency: 2.0,
				depth: 0.5,
			}),
			vibrato: Some(VibratoFilter {
				frequency: 3.0,
				depth: 0.25,
			}),
			rotation: Some(RotationFilter { rotation_hz: 0.6 }),
			distortion: Some(DistortionFilter {
				scale: 1.5,
				..DistortionFilter::default()
			}),
			channel_mix: Some(ChannelMixFilter {
				left_to_left: 0.5,
				left_to_right: 0.5,
				right_to_left: 0.5,
				right_to_right: 0.5,
			}),
			low_pass: Some(LowPassFilter { smoothing: 18000.0 }),
			normalization: None,
			plugin_filters: None,
		};

		let chain = build_filter_chain(1000, &filters).expect("chain");

		assert!(chain.starts_with("anequalizer="));
		assert!(chain.contains(",atempo=tempo=1.2"));
		assert!(chain.contains("tremolo=f=2:d=0.5"));
		assert!(chain.contains("vibrato=f=3:d=0.25"));
		assert!(chain.contains("apulsator=hz=0.6"));
		assert!(chain.contains("acrusher=level_in=1.5:dc=1:mix=1"));
		assert!(chain.contains("pan=stereo|c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1"));
		assert!(chain.ends_with("lowpass=f=18000"));

		let order = [
			"anequalizer",
			"bandreject",
			"atempo",
			"tremolo",
			"vibrato",
			"apulsator",
			"acrusher",
			"pan=stereo|c0=0.5*c0+0.5*c1",
			"lowpass",
		];
		let mut positions = Vec::new();

		for marker in order {
			positions.push(chain.find(marker).expect(marker));
		}

		assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
	}

	#[test]
	fn timescale_combines_pitch_and_rate_into_one_resample() {
		let filter = TimescaleFilter {
			speed: 1.0,
			pitch: 1.5,
			rate: 2.0,
		};

		assert_eq!(timescale_stage(&filter).as_deref(), Some("asetrate=r=144000,aresample=48000"));
	}

	#[test]
	fn timescale_splits_large_speed_into_valid_atempo_factors() {
		let filter = TimescaleFilter {
			speed: 8.0,
			pitch: 1.0,
			rate: 1.0,
		};

		assert_eq!(
			timescale_stage(&filter).as_deref(),
			Some("atempo=tempo=2,atempo=tempo=2,atempo=tempo=2")
		);
	}

	#[test]
	fn identity_timescale_and_channel_mix_build_no_stage() {
		assert_eq!(timescale_stage(&TimescaleFilter::default()), None);
		assert_eq!(channel_mix_stage(&ChannelMixFilter::default()), None);
		assert_eq!(distortion_stage(&DistortionFilter::default()), None);
	}

	#[test]
	fn validation_rejects_garbage_while_accepting_working_values() {
		assert!(validate_filters(&empty_filters()).is_ok());

		let mut valid = Filters {
			volume: Some(0.5),
			equalizer: Some(vec![
				EqualizerBand { band: 0, gain: -0.25 },
				EqualizerBand { band: 14, gain: 1.0 },
			]),
			karaoke: Some(KaraokeFilter::default()),
			timescale: Some(TimescaleFilter {
				speed: 1.2,
				pitch: 1.0,
				rate: 1.0,
			}),
			tremolo: Some(TremoloFilter {
				frequency: 4.0,
				depth: 0.5,
			}),
			vibrato: Some(VibratoFilter {
				frequency: 3.0,
				depth: 0.25,
			}),
			rotation: Some(RotationFilter { rotation_hz: 0.6 }),
			distortion: Some(DistortionFilter::default()),
			channel_mix: Some(ChannelMixFilter::default()),
			low_pass: Some(LowPassFilter { smoothing: 4000.0 }),
			normalization: Some(true),
			plugin_filters: None,
		};

		assert!(validate_filters(&valid).is_ok());

		valid.volume = Some(f32::NAN);
		assert!(validate_filters(&valid).is_err());
		valid.volume = Some(-1.0);
		assert!(validate_filters(&valid).is_err());
		valid.volume = Some(0.0);
		assert!(validate_filters(&valid).is_ok());

		valid.equalizer = Some(vec![EqualizerBand { band: 15, gain: 0.5 }]);
		assert!(validate_filters(&valid).is_err());
		valid.equalizer = Some(vec![EqualizerBand {
			band: 0,
			gain: f32::INFINITY,
		}]);
		assert!(validate_filters(&valid).is_err());
		valid.equalizer = None;

		valid.timescale = Some(TimescaleFilter {
			speed: 0.0,
			pitch: 1.0,
			rate: 1.0,
		});
		assert!(validate_filters(&valid).is_err());
		valid.timescale = None;

		valid.tremolo = Some(TremoloFilter {
			frequency: 4.0,
			depth: 1.5,
		});
		assert!(validate_filters(&valid).is_err());
		valid.tremolo = None;

		valid.karaoke = Some(KaraokeFilter {
			filter_width: 0.0,
			..KaraokeFilter::default()
		});
		assert!(validate_filters(&valid).is_err());
		valid.karaoke = None;

		valid.low_pass = Some(LowPassFilter { smoothing: 0.0 });
		assert!(validate_filters(&valid).is_err());
		valid.low_pass = None;

		valid.rotation = Some(RotationFilter { rotation_hz: -1.0 });
		assert!(validate_filters(&valid).is_err());
		valid.rotation = None;

		assert!(validate_filters(&valid).is_ok());
	}

	#[test]
	fn normalization_stage_appends_loudnorm_last() {
		assert_eq!(normalization_stage(false), None);
		assert_eq!(normalization_stage(true).as_deref(), Some("loudnorm=I=-16:TP=-1.5:LRA=11"));

		let mut filters = empty_filters();
		filters.low_pass = Some(LowPassFilter { smoothing: 4000.0 });
		filters.normalization = Some(true);

		let chain = build_filter_chain(1000, &filters).expect("chain");

		assert!(chain.ends_with(",loudnorm=I=-16:TP=-1.5:LRA=11"));

		filters.normalization = Some(false);
		let chain = build_filter_chain(1000, &filters).expect("chain");

		assert!(!chain.contains("loudnorm"));

		filters.normalization = None;
		let chain = build_filter_chain(1000, &filters).expect("chain");

		assert!(!chain.contains("loudnorm"));
	}

	#[test]
	fn timescale_rejects_non_positive_speed_without_hanging() {
		assert!(atempo_chain(0.0).is_empty());
		assert!(atempo_chain(-2.0).is_empty());
		assert_eq!(
			timescale_stage(&TimescaleFilter {
				speed: 0.0,
				pitch: 1.0,
				rate: 1.0,
			}),
			None
		);
	}

	#[test]
	fn out_of_range_equalizer_band_skips_only_that_band() {
		let stages = equalizer_stage(&[
			EqualizerBand { band: -1, gain: 0.5 },
			EqualizerBand { band: 0, gain: 0.5 },
			EqualizerBand { band: 99, gain: 0.5 },
		])
		.expect("in-range band survives");

		assert!(stages.contains("c0 f=25"));
		assert!(!stages.contains("c0 f=40"));
	}

	#[test]
	fn ffmpeg_args_carry_seek_and_filter_chain() {
		let filters = Filters {
			tremolo: Some(TremoloFilter {
				frequency: 2.0,
				depth: 0.5,
			}),
			..Filters::default()
		};

		let args = ffmpeg_input_args("https://example.invalid/audio.opus", 65_000, 250, &filters, None);

		assert!(args.contains(&"-ss".to_string()));
		assert!(args.contains(&"65".to_string()));
		assert!(args.contains(&"-af".to_string()));
		assert!(args.contains(&"volume=0.25,tremolo=f=2:d=0.5".to_string()));
	}

	#[test]
	fn ffmpeg_args_omit_seek_and_filter_flags_when_idle() {
		let args = ffmpeg_input_args("https://example.invalid/audio.opus", 0, 1000, &empty_filters(), None);

		assert!(!args.contains(&"-ss".to_string()));
		assert!(!args.contains(&"-af".to_string()));
		assert!(!args.contains(&"-to".to_string()));
	}

	#[test]
	fn ffmpeg_args_stop_at_end_marker_after_input() {
		let args = ffmpeg_input_args(
			"https://example.invalid/audio.opus",
			10_000,
			1000,
			&empty_filters(),
			Some(45_000),
		);

		let input = args.iter().position(|arg| arg == "-i").expect("input flag");
		let stop = args.iter().position(|arg| arg == "-to").expect("stop flag");

		assert!(stop > input);
		assert_eq!(args[stop + 1], "45");
		assert!(!args.contains(&"-ss".to_string()) || args.iter().position(|arg| arg == "-ss").unwrap() < input);
	}
}
