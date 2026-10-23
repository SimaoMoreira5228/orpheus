FROM ghcr.io/cross-rs/aarch64-unknown-linux-gnu:main

RUN dpkg --add-architecture arm64 \
	&& apt-get update \
	&& apt-get install --assume-yes libopus-dev:arm64 python3 python3-pip libpython3-dev:arm64 \
	&& rm -rf /var/lib/apt/lists/*

RUN pip install --break-system-packages --no-cache-dir uv

COPY cross/aarch64-python.cmake /tmp/aarch64-python.cmake
COPY cross/libytdlp-python /usr/local/bin/libytdlp-python-aarch64

RUN cat /tmp/aarch64-python.cmake >> /opt/toolchain.cmake \
	&& chmod +x /usr/local/bin/libytdlp-python-aarch64 \
	&& rm /tmp/aarch64-python.cmake
