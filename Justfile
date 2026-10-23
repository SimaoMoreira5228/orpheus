set dotenv-load := true

fmt *args:
    ./tools/fmt {{ args }}

lint:
    ./tools/lint

test:
    cargo test --workspace --all-features

coverage output="":
    @if [ -n "{{ output }}" ]; then \
        cargo llvm-cov --workspace --all-features --lcov --output-path="{{ output }}"; \
    else \
        cargo llvm-cov --workspace --all-features; \
    fi

coverage-clean:
    cargo llvm-cov clean

build:
    cargo build --release --workspace

check:
    ./tools/fmt --check
    ./tools/lint
    cargo test --workspace --all-features
