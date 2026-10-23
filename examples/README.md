# Orpheus Discord Bot Examples

This directory contains example Discord bots that integrate with Orpheus.

## Rust Bot

Location: `discord-bot-rust.rs`

A simple Rust bot using the [serenity](https://github.com/serenity-rs/serenity) crate.

### Setup

Add to your `Cargo.toml`:

```toml
[dependencies]
serenity = "0.12"
reqwest = { version = "0.12", features = ["json"] }
tokio = { version = "1", features = ["full"] }
```

### Run

```bash
export DISCORD_TOKEN=your_bot_token
export ORPHEUS_HOST=127.0.0.1
export ORPHEUS_PORT=2333
export ORPHEUS_PASSWORD=youshallnotpass

cargo run --example discord-bot-rust
```

### Commands

- `!play <query>` - Play a track
- `!pause` - Pause playback
- `!resume` - Resume playback
- `!stop` - Stop playback
- `!volume <0-1000>` - Set volume
- `!help` - Show help

---

## JavaScript Bot

Location: `discord-bot-js/`

A simple JS bot using [discord.js](https://discord.js.org/) and [lavalink.js](https://lavalink.js.org/).

### Setup

```bash
cd discord-bot-js
pnpm install
```

### Run

```bash
export DISCORD_TOKEN=your_bot_token
export ORPHEUS_HOST=127.0.0.1
export ORPHEUS_PORT=2333
export ORPHEUS_PASSWORD=youshallnotpass

pnpm start
```

### Commands

- `!play <query>` - Play a track
- `!pause` - Pause playback
- `!resume` - Resume playback
- `!stop` - Stop and disconnect
- `!skip` - Skip current track
- `!volume [0-1000]` - Set or show volume
- `!queue` - Show queue
- `!help` - Show help
