FROM rust:bookworm AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y pkg-config libopus0 libopus-dev cmake g++ python3-dev python3-pip && rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build --release --bin orpheus

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y libopus0 ca-certificates ffmpeg python3 libpython3.11 python3-pip && rm -rf /var/lib/apt/lists/* \
	&& pip install --break-system-packages --no-cache-dir yt-dlp==2026.10.21

RUN groupadd -r orpheus && useradd -r -g orpheus orpheus

WORKDIR /app

COPY --from=builder /build/target/release/orpheus /app/orpheus

RUN chown -R orpheus:orpheus /app

USER orpheus

EXPOSE 2333 2334

ENTRYPOINT ["/app/orpheus"]
