import { strict as assert } from "node:assert";
import { test } from "node:test";
import type { WsPong, WsServerEvent } from "../src/types.js";
import { OrpheusWsClient, parseServerFrame } from "../src/ws.js";
import type { WsHandler, WsSocket } from "../src/ws.js";

class FakeSocket implements WsSocket {
	sent: string[] = [];
	closed = false;
	onmessage: ((event: { data: unknown }) => void) | null = null;
	onopen: (() => void) | null = null;
	onerror: ((event: unknown) => void) | null = null;
	onclose: (() => void) | null = null;

	constructor(
		public url: string,
		public headers: Record<string, string>
	) {}

	send(data: string): void {
		this.sent.push(data);
	}

	close(): void {
		this.closed = true;
	}

	open(): void {
		this.onopen?.();
	}

	receive(frame: unknown): void {
		this.onmessage?.({ data: JSON.stringify(frame) });
	}
}

function connectedClient(
	config: Record<string, unknown> = {},
	events: Array<WsServerEvent | WsPong> = []
): Promise<{ client: OrpheusWsClient; socket: FakeSocket }> {
	const sockets: FakeSocket[] = [];
	const handler: WsHandler = (event) => {
		events.push(event);
	};
	const client = new OrpheusWsClient(
		{ host: "127.0.0.1", port: 2333, password: "secret", userId: 7, ...config },
		handler,
		(url, headers) => {
			const socket = new FakeSocket(url, headers);

			sockets.push(socket);

			return socket;
		}
	);
	const connected = client.connect();

	assert.equal(sockets.length, 1);

	sockets[0].open();

	return connected.then(() => ({ client, socket: sockets[0] }));
}

test("connect uses the websocket url and auth headers", async () => {
	const { socket } = await connectedClient({ sessionId: "abc", tls: true });

	assert.equal(socket.url, "wss://127.0.0.1:2333/v4/websocket");
	assert.equal(socket.headers.Authorization, "secret");
	assert.equal(socket.headers["User-Id"], "7");
	assert.equal(socket.headers["Client-Name"], "orpheus-sdk");
	assert.equal(socket.headers["Session-Id"], "abc");
});

test("commands serialize with lavalink ops", async () => {
	const { client, socket } = await connectedClient();

	client.ping(42);
	client.destroy(7);
	client.configureResuming("key", 60);
	client.playerUpdate(7, { paused: true });
	client.voiceUpdate(7, { token: "t", endpoint: "e", sessionId: "s" });
	client.loadTracks("ytsearch:test");

	const sent = socket.sent.map((body) => JSON.parse(body));

	assert.deepEqual(sent[0], { op: "ping", timestamp: 42 });
	assert.deepEqual(sent[1], { op: "destroy", guildId: "7" });
	assert.deepEqual(sent[2], { op: "configureResuming", key: "key", timeout: 60 });
	assert.deepEqual(sent[3], { op: "playerUpdate", guildId: "7", data: { paused: true } });
	assert.deepEqual(sent[4], { op: "voiceUpdate", guildId: "7", data: { token: "t", endpoint: "e", sessionId: "s" } });
	assert.deepEqual(sent[5], { op: "loadtracks", identifier: "ytsearch:test" });
});

test("configureResuming omits absent fields", async () => {
	const { client, socket } = await connectedClient();

	client.configureResuming();

	assert.deepEqual(JSON.parse(socket.sent[0]), { op: "configureResuming" });
});

test("incoming frames dispatch by op, unknown frames are ignored", async () => {
	const events: Array<WsServerEvent | WsPong> = [];
	const { socket } = await connectedClient({}, events);

	socket.receive({ op: "ready", sessionId: "abc", resumed: false, sourceManagers: ["youtube"] });
	socket.receive({ op: "playerUpdate", guildId: "7", state: { time: 1, position: 0, connected: true, ping: -1 } });
	socket.receive({ op: "pong", timestamp: 42 });
	socket.receive({ op: "futureOp", x: 1 });
	socket.onmessage?.({ data: "not json" });

	assert.equal(events.length, 3);
	assert.equal(events[0].op, "ready");
	assert.equal(events[1].op, "playerUpdate");
	assert.deepEqual(events[2], { op: "pong", timestamp: 42 });
});

test("sending before connect throws, close drops the socket", async () => {
	const events: Array<WsServerEvent | WsPong> = [];
	const { client, socket } = await connectedClient({}, events);

	assert.throws(
		() =>
			new OrpheusWsClient(
				{ host: "127.0.0.1", port: 2333, password: "secret", userId: 7 },
				() => {},
				() => socket
			).ping(1),
		/not connected/
	);

	client.close();

	assert.equal(socket.closed, true);
	assert.throws(() => client.ping(1), /not connected/);
});

test("parseServerFrame rejects garbage", () => {
	assert.equal(parseServerFrame("not json"), undefined);
	assert.equal(parseServerFrame("[1,2]"), undefined);
	assert.equal(parseServerFrame('"just a string"'), undefined);
	assert.equal(parseServerFrame(null), undefined);
});
