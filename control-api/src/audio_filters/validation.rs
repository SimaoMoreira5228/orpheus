use orpheus_core::Filters;

fn finite(name: &str, value: f64) -> Result<(), String> {
	if value.is_finite() {
		Ok(())
	} else {
		Err(format!("{name} must be finite"))
	}
}

fn positive(name: &str, value: f64) -> Result<(), String> {
	if value.is_finite() && value > 0.0 {
		Ok(())
	} else {
		Err(format!("{name} must be a finite number greater than 0"))
	}
}

pub fn validate_filters(filters: &Filters) -> Result<(), String> {
	if let Some(volume) = filters.volume {
		finite("volume", volume as f64)?;

		if volume < 0.0 {
			return Err("volume must be a gain greater than or equal to 0".to_string());
		}
	}

	if let Some(bands) = filters.equalizer.as_ref() {
		for band in bands {
			if !(0..15).contains(&band.band) {
				return Err(format!("equalizer band {} out of range 0-14", band.band));
			}

			finite("equalizer gain", band.gain as f64)?;
		}
	}

	if let Some(karaoke) = filters.karaoke.as_ref() {
		finite("karaoke level", karaoke.level as f64)?;
		finite("karaoke mono_level", karaoke.mono_level as f64)?;
		positive("karaoke filter_band", karaoke.filter_band as f64)?;
		positive("karaoke filter_width", karaoke.filter_width as f64)?;
	}

	if let Some(timescale) = filters.timescale.as_ref() {
		positive("timescale speed", timescale.speed)?;
		positive("timescale pitch", timescale.pitch)?;
		positive("timescale rate", timescale.rate)?;
	}

	if let Some(tremolo) = filters.tremolo.as_ref() {
		if !(0.1..=20000.0).contains(&f64::from(tremolo.frequency)) {
			return Err("tremolo frequency must be within 0.1-20000".to_string());
		}

		if !(0.0..=1.0).contains(&f64::from(tremolo.depth)) {
			return Err("tremolo depth must be within 0-1".to_string());
		}
	}

	if let Some(vibrato) = filters.vibrato.as_ref() {
		if !(0.1..=20000.0).contains(&f64::from(vibrato.frequency)) {
			return Err("vibrato frequency must be within 0.1-20000".to_string());
		}

		if !(0.0..=1.0).contains(&f64::from(vibrato.depth)) {
			return Err("vibrato depth must be within 0-1".to_string());
		}
	}

	if let Some(rotation) = filters.rotation.as_ref() {
		let hz = rotation.rotation_hz;

		if !hz.is_finite() || hz < 0.0 {
			return Err("rotation rotation_hz must be a finite number greater than or equal to 0".to_string());
		}
	}

	if let Some(distortion) = filters.distortion.as_ref() {
		for (name, value) in [
			("sin_offset", distortion.sin_offset as f64),
			("sin_scale", distortion.sin_scale as f64),
			("cos_offset", distortion.cos_offset as f64),
			("cos_scale", distortion.cos_scale as f64),
			("tan_offset", distortion.tan_offset as f64),
			("tan_scale", distortion.tan_scale as f64),
			("offset", distortion.offset as f64),
			("scale", distortion.scale as f64),
		] {
			finite(&format!("distortion {name}"), value)?;
		}
	}

	if let Some(mix) = filters.channel_mix.as_ref() {
		for (name, value) in [
			("left_to_left", mix.left_to_left as f64),
			("left_to_right", mix.left_to_right as f64),
			("right_to_left", mix.right_to_left as f64),
			("right_to_right", mix.right_to_right as f64),
		] {
			finite(&format!("channel_mix {name}"), value)?;
		}
	}

	if let Some(low_pass) = filters.low_pass.as_ref() {
		positive("low_pass smoothing", low_pass.smoothing as f64)?;
	}

	Ok(())
}
