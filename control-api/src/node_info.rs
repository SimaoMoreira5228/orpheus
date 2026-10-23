use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct LavalinkInfo {
	pub version: VersionInfo,
	#[serde(rename = "buildTime")]
	pub build_time: u64,
	pub git: GitInfo,
	#[serde(rename = "jvm")]
	pub jvm: String,
	pub lavaplayer: String,
	#[serde(rename = "sourceManagers", default)]
	pub source_managers: Vec<String>,
	#[serde(default)]
	pub filters: Vec<String>,
	#[serde(default)]
	pub plugins: Vec<PluginWireInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
	pub semver: String,
	pub major: i32,
	pub minor: i32,
	pub patch: i32,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pre_release: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
	pub branch: String,
	pub commit: String,
	pub commit_time: u64,
}

#[derive(Debug, Serialize)]
pub struct PluginWireInfo {
	pub name: String,
	pub version: String,
}

pub fn version_info() -> VersionInfo {
	parse_semver(env!("CARGO_PKG_VERSION"))
}

fn parse_semver(semver: &str) -> VersionInfo {
	let without_build = semver.split('+').next().unwrap_or(semver);

	let (core, pre_release) = match without_build.split_once('-') {
		Some((core, pre)) => (core, (!pre.is_empty()).then(|| pre.to_string())),
		None => (without_build, None),
	};

	let mut parts = core.split('.').map(|part| part.parse::<i32>().unwrap_or(0));

	VersionInfo {
		semver: semver.to_string(),
		major: parts.next().unwrap_or(0),
		minor: parts.next().unwrap_or(0),
		patch: parts.next().unwrap_or(0),
		pre_release,
	}
}

pub fn build_time_ms() -> u64 {
	option_env!("ORPHEUS_BUILD_TIME_MS")
		.and_then(|value| value.parse().ok())
		.unwrap_or(0)
}

pub fn git_info() -> GitInfo {
	GitInfo {
		branch: option_env!("ORPHEUS_GIT_BRANCH").unwrap_or("unknown").to_string(),
		commit: option_env!("ORPHEUS_GIT_COMMIT").unwrap_or("unknown").to_string(),
		commit_time: option_env!("ORPHEUS_GIT_COMMIT_TIME_MS")
			.and_then(|value| value.parse().ok())
			.unwrap_or(0),
	}
}

pub fn runtime_name() -> String {
	// NOTE: the protocol field is named `jvm` because Lavalink runs on Java; orpheus is Rust,
	// so the truthful value is the rustc release that built this binary.
	option_env!("ORPHEUS_RUSTC_VERSION").unwrap_or("rustc (unknown)").to_string()
}

pub fn lavaplayer_version() -> String {
	// NOTE: orpheus has no Lavaplayer; audio is ffmpeg decode plus audiopus encode,
	// so report the real backend instead of a Lavaplayer version.
	"ffmpeg + audiopus (no lavaplayer)".to_string()
}

pub fn enabled_filters() -> Vec<String> {
	[
		"volume",
		"equalizer",
		"karaoke",
		"timescale",
		"tremolo",
		"vibrato",
		"rotation",
		"distortion",
		"channelMix",
		"lowPass",
	]
	.iter()
	.map(|name| name.to_string())
	.collect()
}

pub fn plugin_wire_info(plugins: &[orpheus_plugins::PluginInfo]) -> Vec<PluginWireInfo> {
	plugins
		.iter()
		.map(|plugin| PluginWireInfo {
			name: plugin.name.clone(),
			version: plugin.version.clone(),
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn semver_parses_release() {
		let version = parse_semver("0.1.0");

		assert_eq!(version.major, 0);
		assert_eq!(version.minor, 1);
		assert_eq!(version.patch, 0);
		assert_eq!(version.pre_release, None);
	}

	#[test]
	fn semver_parses_prerelease() {
		let version = parse_semver("4.2.1-rc.1");

		assert_eq!((version.major, version.minor, version.patch), (4, 2, 1));
		assert_eq!(version.pre_release, Some("rc.1".to_string()));
	}

	#[test]
	fn info_matches_lavalink_shape() {
		let info = LavalinkInfo {
			version: parse_semver("0.1.0"),
			build_time: 1_700_000_000_000,
			git: GitInfo {
				branch: "main".to_string(),
				commit: "abc123".to_string(),
				commit_time: 1_700_000_000_000,
			},
			jvm: "rustc 1.0.0".to_string(),
			lavaplayer: "ffmpeg + audiopus (no lavaplayer)".to_string(),
			source_managers: vec!["youtube".to_string()],
			filters: Vec::new(),
			plugins: vec![PluginWireInfo {
				name: "demo".to_string(),
				version: "1.0".to_string(),
			}],
		};

		let json = serde_json::to_value(&info).unwrap();

		assert_eq!(json["version"]["semver"], "0.1.0");
		assert_eq!(json["version"]["major"], 0);
		assert_eq!(json["version"]["preRelease"], serde_json::Value::Null);
		assert!(json["version"].get("build").is_none());
		assert!(json["buildTime"].is_u64());
		assert_eq!(json["git"]["branch"], "main");
		assert!(json["git"]["commitTime"].is_u64());
		assert!(json["jvm"].is_string());
		assert_eq!(json["sourceManagers"][0], "youtube");
		assert!(json["filters"].as_array().unwrap().is_empty());

		let plugins = json["plugins"].as_array().unwrap();

		assert_eq!(plugins.len(), 1);
		assert_eq!(plugins[0], serde_json::json!({"name": "demo", "version": "1.0"}));
	}
}
