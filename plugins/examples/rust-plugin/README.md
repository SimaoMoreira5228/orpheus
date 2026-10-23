# Example Rust Plugin

This is an example of an Orpheus plugin written in Rust that uses wit-bindgen.

## Building

Requires `cargo-component` and the `wasm32-wasip2` target:

```bash
cargo install cargo-component
rustup target add wasm32-wasip2

cargo component build --release --target wasm32-wasip2
```

The output is a component at
`target/wasm32-wasip2/release/orpheus_example_plugin.wasm`. A plain
`cargo build` produces a core module, not a component, and the host will
refuse to load it.

The guest generates its bindings from the single canonical
`plugins/wit/plugin.wit` (see the `path` in `src/lib.rs`); edit the
interface there, never here. `track_info` must report the WIT package
version in `api_version` or the host rejects the guest at load (currently
`0.2.0`; guests built against `0.1.0` are refused at instantiate time
because they lack the newer exports).

The guest also demonstrates the P2 surface: a `GET /hello` route declared
in `track-info` and served by `handle-route`, and a `configure` export that
stores the `name` key from the plugin config object. Host network access stays
deny-by-default: the `fetch` import only succeeds for hosts listed in the
plugin's `allow_hosts` config.

## Plugin Structure

The plugin uses wit-bindgen against the canonical `plugins/wit/plugin.wit`:

```wit
package orpheus:plugin@0.2.0;

interface host {
    log: func(msg: string);
    get-api-version: func() -> string;
}

interface fetch {
    fetch: func(url: string, opts: string) -> result<string, string>;
}

world orpheus-plugin {
    import host;
    import fetch;

    export init: func();
    export shutdown: func();
    export resolve: func(query: string) -> string;
    export filter: func(frame-ptr: u32, frame-len: u32);
    export track-info: func() -> string;
    export configure: func(json: string);
    export handle-route: func(
        method: string,
        path: string,
        query: string,
        headers-json: string,
        body-b64: string,
    ) -> result<string, string>;
}
```

## Implementation

The Rust code implements the `Guest` trait:

```rust
impl Guest for Plugin {
    fn init() {}
    fn shutdown() {}
    fn resolve(query: String) -> String { ... }
    fn filter(_frame_ptr: u32, _frame_len: u32) {}
    fn track_info() -> String { ... }
    fn configure(json: String) { ... }
    fn handle_route(method: String, path: String, query: String, headers_json: String, body_b64: String) -> Result<String, String> { ... }
}
```

The host provides:

- `log(msg: string)` - Log a message
- `get-api-version() -> string` - Get the API version
- `fetch(url: string, opts: string) -> result<string, string>` - Fetch a URL (JSON-in/stringly; allowed hosts only)
