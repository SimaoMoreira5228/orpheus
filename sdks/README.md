# sdks

Client libraries for the Orpheus control plane.

- `rust/` — Rust client (`orpheus-client-sdk` crate, workspace member).
- `typescript/` — TypeScript client (`orpheus-sdk` package, pnpm, zero runtime deps).

Parity rule: both SDKs expose the same surface (types, methods, wire names).
Adding an endpoint means adding it in both, with tests, in the same commit.

Repository-wide checks run from the root with `./tools/fmt` and `./tools/lint`.
They use Rust nightly, plus the pinned Oxfmt and Oxlint tools installed by
`pnpm install` in `sdks/typescript`.
