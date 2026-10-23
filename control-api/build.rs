use std::process::Command;

fn git_output(args: &[&str]) -> Option<String> {
	let output = Command::new("git").args(args).output().ok()?;

	if !output.status.success() {
		return None;
	}

	let text = String::from_utf8(output.stdout).ok()?.trim().to_string();

	if text.is_empty() {
		return None;
	}

	Some(text)
}

fn emit(name: &str, value: &str) {
	println!("cargo:rustc-env={name}={value}");
}

fn main() {
	let build_time_ms = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_millis().to_string())
		.unwrap_or_else(|_| "0".to_string());
	emit("ORPHEUS_BUILD_TIME_MS", &build_time_ms);

	emit(
		"ORPHEUS_GIT_BRANCH",
		&git_output(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "unknown".to_string()),
	);
	emit(
		"ORPHEUS_GIT_COMMIT",
		&git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string()),
	);

	let commit_time_ms = git_output(&["log", "-1", "--format=%ct"])
		.and_then(|s| s.parse::<u64>().ok())
		.map(|s| s.saturating_mul(1000).to_string())
		.unwrap_or_else(|| "0".to_string());
	emit("ORPHEUS_GIT_COMMIT_TIME_MS", &commit_time_ms);

	let rustc_version = Command::new("rustc")
		.arg("-V")
		.output()
		.ok()
		.and_then(|o| String::from_utf8(o.stdout).ok())
		.map(|s| s.trim().to_string())
		.unwrap_or_else(|| "unknown".to_string());
	emit("ORPHEUS_RUSTC_VERSION", &rustc_version);
}
