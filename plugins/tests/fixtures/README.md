# Test fixture: `example-plugin.wasm`

Built from `plugins/examples/rust-plugin` (which generates its bindings from the
canonical `plugins/wit/plugin.wit`, so this fixture always matches the host API).

Rebuild after changing the guest or the WIT world:

```bash
cd plugins/examples/rust-plugin
cargo component build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/orpheus_example_plugin.wasm \
  ../../tests/fixtures/example-plugin.wasm
```

Do not check in a fixture built from a modified guest without updating
`plugins/tests/guest_roundtrip.rs` and `plugins/tests/plugin_p2.rs`
expectations to match.
