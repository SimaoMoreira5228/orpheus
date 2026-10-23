# orpheus-sdk

TypeScript client library for Orpheus. Parity with `../rust`: same wire
types, same methods, same camelCase names. Zero runtime dependencies
(`fetch` only, Node 22+).

## Use

```ts
import { OrpheusClient } from "orpheus-sdk";

const client = new OrpheusClient({ host: "127.0.0.1", port: 2333, password: "youshallnotpass" });
const result = await client.loadTracks("ytsearch:never gonna give you up");
const track = result.data;
```

Pass a custom `fetch` as the third constructor argument to stub the transport
in tests.

## Develop

```sh
pnpm install
pnpm typecheck
pnpm test
pnpm build
```
