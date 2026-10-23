# Orpheus Plugin Examples

This directory contains examples demonstrating how to create plugins for Orpheus.

## Structure

- `rust-plugin/` - Example of a native Rust plugin (compiled to WASM)
- `wit/` - WIT interface definitions used by plugins

## Creating a Plugin

### 1. Define your plugin interface in WIT

See the main WIT file at `../wit/plugin.wit` for the interface definitions.

### 2. Implement the plugin

Your plugin must implement:

- `init()` - Called when the plugin is loaded
- `shutdown()` - Called when the plugin is unloaded
- `resolve(query: string) -> string` - Resolve a query to track info (for track resolver plugins)
- `filter(frame-ptr: u32, frame-len: u32)` - Process audio frames (for filter plugins)
- `track-info() -> string` - Return information about the plugin

### 3. Compile to WASM

Use `cargo wasi` or `wasm32-wasi` target to compile your plugin:

```bash
cargo build --target wasm32-wasi --release
```

## Example: Track Resolver Plugin

A track resolver plugin takes a search query and returns track information.

```rust
use serde::{Deserialize, Serialize;

#[derive(Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub source: String,
    pub url: String,
}

#[no_mangle]
pub extern "C" fn resolve(query: &str) -> *mut u8 {
    // Your resolution logic here
    let track = TrackInfo {
        id: "example-id".to_string(),
        title: "Example Track".to_string(),
        artist: "Example Artist".to_string(),
        album: None,
        duration_ms: 180000,
        source: "example".to_string(),
        url: "https://example.com/track.mp3".to_string(),
    };
    
    // Serialize and return
    let json = serde_json::to_string(&track).unwrap();
    // ... handle memory allocation
}
```

## Example: Audio Filter Plugin

An audio filter plugin processes audio frames in real-time.

```rust
#[no_mangle]
pub extern "C" fn filter(frame_ptr: u32, frame_len: u32) {
	// Process audio frame
	// frame_ptr points to i16 PCM data
	// frame_len is the number of samples
}
```
