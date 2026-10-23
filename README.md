# Orpheus

Orpheus is a self-hosted audio server written in Rust. It exposes a Lavalink-style v4 REST and WebSocket API for controlling players, queues, filters, and Discord voice connections. You can connect with the Rust or TypeScript client, or use the API directly.

## What it does

- Resolves tracks from YouTube, Twitch, Spotify, SoundCloud, Bandcamp, playlists, local files, and direct HTTP URLs. yt-dlp provides additional source handling.
- Manages players, queues, audio filters, filter presets, and Discord voice connections.
- Serves control and playback events over REST and WebSocket, with health, stats, and Prometheus metrics endpoints.
- Loads WebAssembly plugins that can add track resolvers, audio filters, and API routes.
- Ships Rust and TypeScript client libraries, plus Discord bot examples.

Spotify URL and search support needs Spotify Web API credentials. Add `SPOTIFY_CLIENT_ID` and `SPOTIFY_CLIENT_SECRET` to your environment. Other sources do not need those credentials.

## Run it

Building from source needs Rust, CMake, a C++23 compiler, Python development headers, `uv`, and Opus development libraries. On Debian or Ubuntu, install the system packages with:

```sh
sudo apt-get update
sudo apt-get install -y cmake g++ libopus-dev pkg-config python3-dev
```

Install `uv` using the instructions at [docs.astral.sh/uv](https://docs.astral.sh/uv/getting-started/installation/), then build and start the server:

```sh
cargo build --release --bin orpheus
ORPHEUS_PASSWORD='choose-a-password' cargo run --release --bin orpheus
```

Orpheus reads `config.toml` from the current directory. The checked-in file listens on `0.0.0.0:2333` and contains the default resolver order and plugin settings. Change `host` and `http_port` there if you need a different address. The server reads `ORPHEUS_PASSWORD` from the environment or `.env`; set your own value before exposing the API.

The health endpoint is public at `http://localhost:2333/health`. Most API routes require the password in the `Authorization` header. The WebSocket endpoint is `/v4/websocket` and performs its own authentication.

To build and run the container image instead:

```sh
docker build -t orpheus .
docker run --rm -p 2333:2333 \
  -e ORPHEUS_PASSWORD='choose-a-password' \
  -v "$PWD/config.toml:/app/config.toml:ro" \
  orpheus
```

## Clients and examples

The Rust client crate is in [`sdks/rust`](sdks/rust). The TypeScript package is in [`sdks/typescript`](sdks/typescript); it needs Node.js 22 or newer.

```ts
import { OrpheusClient } from "orpheus-sdk";

const client = new OrpheusClient({
  host: "127.0.0.1",
  port: 2333,
  password: "choose-a-password",
});

const tracks = await client.loadTracks("ytsearch:never gonna give you up");
```

Use the same password you set in `ORPHEUS_PASSWORD`.

See [`sdks/README.md`](sdks/README.md) for SDK details and [`examples/README.md`](examples/README.md) for the Rust and JavaScript Discord bots.

## Plugins

Orpheus can load WebAssembly component plugins from the directory configured by `[plugins].dir` in `config.toml`. The example plugin and build instructions are in [`plugins/examples/rust-plugin`](plugins/examples/rust-plugin). A plugin's outbound fetch access is denied unless its configuration lists allowed hosts.

## Development

Run the Rust test suite and repository checks from the project root:

```sh
cargo test --workspace --all-features
./tools/fmt --check
./tools/lint
```

To work on the TypeScript SDK, install its development dependencies with pnpm:

```sh
cd sdks/typescript
pnpm install --frozen-lockfile
pnpm typecheck
pnpm test
```

## License

Orpheus is licensed under [AGPL-3.0-or-later](LICENSE).
