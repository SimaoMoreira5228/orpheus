import { strict as assert } from "node:assert";
import { test } from "node:test";
import { OrpheusClient, OrpheusError } from "../src/client.js";
import { firstTrack } from "../src/types.js";
import type { LoadResult, Track } from "../src/types.js";

interface CapturedCall {
	url: string;
	method: string;
	headers: Record<string, string>;
	body?: string;
}

function stubFetch(handler: (call: CapturedCall) => Response) {
	const calls: CapturedCall[] = [];

	const fetchFn = (async (url: unknown, init?: { method?: string; headers?: Record<string, string>; body?: string }) => {
		const call: CapturedCall = {
			url: String(url),
			method: init?.method ?? "GET",
			headers: init?.headers ?? {},
			body: init?.body,
		};

		calls.push(call);

		return handler(call);
	}) as unknown as typeof fetch;

	return { calls, fetchFn };
}

function jsonResponse(value: unknown, status = 200): Response {
	return new Response(JSON.stringify(value), { status, headers: { "Content-Type": "application/json" } });
}

function trackWith(encoded: string | undefined, uri: string | null): Track {
	return {
		...(encoded !== undefined ? { encoded } : {}),
		info: {
			identifier: "https://example.com/watch?v=abc",
			isSeekable: true,
			author: "author",
			length: 1000,
			isStream: false,
			position: 0,
			title: "title",
			uri,
			artworkUrl: null,
			isrc: null,
			sourceName: "youtube",
		},
	};
}

function clientWith(handler: (call: CapturedCall) => Response) {
	const { calls, fetchFn } = stubFetch(handler);
	const client = new OrpheusClient({ host: "127.0.0.1", port: 2333, password: "secret" }, "session-1", fetchFn);

	return { calls, client };
}

test("requests carry the base url and auth header", async () => {
	const { calls, client } = clientWith(() => jsonResponse({ version: "0.1.0" }));

	await client.getVersion();

	assert.equal(calls.length, 1);
	assert.equal(calls[0].url, "http://127.0.0.1:2333/version");
	assert.equal(calls[0].method, "GET");
	assert.equal(calls[0].headers.Authorization, "secret");
});

test("loadTracks encodes the identifier as a query param", async () => {
	const { calls, client } = clientWith(() => jsonResponse({ loadType: "Empty" }));

	await client.loadTracks("ytsearch:never gonna give you up");

	assert.equal(calls[0].url, "http://127.0.0.1:2333/v4/loadtracks?identifier=ytsearch%3Anever%20gonna%20give%20you%20up");
});

test("play prefers the encoded track, falls back to the uri identifier", async () => {
	const seen: unknown[] = [];
	const { client } = clientWith((call) => {
		seen.push(call.body !== undefined ? JSON.parse(call.body) : undefined);

		return jsonResponse({});
	});

	await client.play(7, trackWith("encoded-abc", "https://example.com/watch?v=abc"));
	await client.play(7, trackWith(undefined, "https://example.com/watch?v=abc"));

	assert.deepEqual(seen[0], { track: { encoded: "encoded-abc" }, position: 0, paused: false });
	assert.deepEqual(seen[1], { track: { identifier: "https://example.com/watch?v=abc" }, position: 0, paused: false });
});

test("updatePlayer keeps camelCase names and explicit nulls", async () => {
	const seen: unknown[] = [];
	const { client } = clientWith((call) => {
		seen.push(call.body !== undefined ? JSON.parse(call.body) : undefined);

		return jsonResponse({});
	});

	await client.updatePlayer(7, { noReplace: true, endTime: null, filters: { volume: 0.5 } });

	assert.deepEqual(seen[0], { noReplace: true, endTime: null, filters: { volume: 0.5 } });
});

test("failed requests throw OrpheusError with status and body", async () => {
	const { client } = clientWith(() =>
		jsonResponse(
			{
				timestamp: 1,
				status: 404,
				error: "Not Found",
				message: "Session not found",
				path: "/v4/info",
				code: "SESSION_NOT_FOUND",
			},
			404
		)
	);

	await assert.rejects(
		() => client.getInfo(),
		(error: unknown) =>
			error instanceof OrpheusError &&
			error.status === 404 &&
			error.code === "SESSION_NOT_FOUND" &&
			error.body.includes("/v4/info") &&
			error.message.includes("Session not found")
	);
});

test("deletePlayer resolves on 204 with no body", async () => {
	const { client } = clientWith(() => new Response(null, { status: 204 }));

	await client.deletePlayer(7);
});

test("withSession scopes subsequent paths", async () => {
	const { calls, client } = clientWith(() => jsonResponse({}));

	await client.withSession("other").getPlayer(7);

	assert.equal(calls[0].url, "http://127.0.0.1:2333/v4/sessions/other/players/7");
});

test("setVoiceChannel puts the channel id", async () => {
	const seen: unknown[] = [];
	const { calls, client } = clientWith((call) => {
		seen.push(call.body !== undefined ? JSON.parse(call.body) : undefined);

		return new Response(null, { status: 204 });
	});

	await client.setVoiceChannel(7, 123);

	assert.equal(calls[0].method, "PUT");
	assert.ok(calls[0].url.endsWith("/v4/sessions/session-1/players/7/voice-channel"));
	assert.deepEqual(seen[0], { channelId: 123 });
});

test("batch posts identifiers and keeps per-item results", async () => {
	const seen: unknown[] = [];
	const { client } = clientWith((call) => {
		seen.push(call.body !== undefined ? JSON.parse(call.body) : undefined);

		return jsonResponse([
			{ loadType: "error", data: { message: "No matches", severity: "COMMON", code: "NO_MATCHES" } },
			{
				loadType: "track",
				data: trackWith("encoded-abc", null),
			},
		]);
	});

	const results = await client.loadTracksBatch(["spsearch:nope", "file:///song.mp3"]);

	assert.deepEqual(seen[0], { identifiers: ["spsearch:nope", "file:///song.mp3"] });
	assert.equal(results.length, 2);
	assert.equal(firstTrack(results[0]), undefined);
	assert.equal(firstTrack(results[1])?.info.title, "title");
	assert.equal((results[0].data as { code?: string }).code, "NO_MATCHES");
});

test("firstTrack covers track, playlist and search shapes", () => {
	const track = trackWith("encoded-abc", null);

	const single: LoadResult = { loadType: "Track", data: track };
	const playlist: LoadResult = { loadType: "Playlist", data: { info: {}, tracks: [track] } };
	const search: LoadResult = { loadType: "Search", data: [track] };
	const empty: LoadResult = { loadType: "Empty", data: [] };
	const failed: LoadResult = { loadType: "Error", data: { message: "No matches", severity: "COMMON", code: "NO_MATCHES" } };

	assert.equal(firstTrack(single), track);
	assert.equal(firstTrack(playlist), track);
	assert.equal(firstTrack(search), track);
	assert.equal(firstTrack(empty), undefined);
	assert.equal(firstTrack(failed), undefined);
});

test("loadSearch hits the rich search endpoint and parses all types", async () => {
	const seen: string[] = [];
	const { client } = clientWith((call) => {
		seen.push(call.url);

		return jsonResponse({
			tracks: [],
			albums: [{ id: "a", name: "Album", artist: "Art", uri: "https://open.spotify.com/album/a" }],
			artists: [{ id: "r", name: "Art", uri: "https://open.spotify.com/artist/r" }],
			playlists: [{ id: "p", name: "List", author: "Someone", uri: "https://open.spotify.com/playlist/p", trackCount: 10 }],
		});
	});

	const result = await client.loadSearch("never gonna give you up");

	assert.ok(seen[0].startsWith("http://127.0.0.1:2333/v4/loadsearch?query="));
	assert.ok(seen[0].includes("never%20gonna"));
	assert.equal(result.albums[0].artist, "Art");
	assert.equal(result.playlists[0].trackCount, 10);
});

test("dry-run posts identifiers and keeps playback urls", async () => {
	const seen: unknown[] = [];
	const { client } = clientWith((call) => {
		seen.push(call.body !== undefined ? JSON.parse(call.body) : undefined);

		return jsonResponse([
			{
				loadType: "error",
				data: { message: "No matches", severity: "COMMON", code: "NO_MATCHES" },
				playbackUrl: null,
			},
			{ loadType: "track", data: trackWith("encoded-abc", "file:///song.mp3"), playbackUrl: "file:///song.mp3" },
		]);
	});

	const results = await client.loadTracksDryRun(["spsearch:nope", "file:///song.mp3"]);

	assert.deepEqual(seen[0], { identifiers: ["spsearch:nope", "file:///song.mp3"] });
	assert.equal(results.length, 2);
	assert.equal(results[0].playbackUrl, null);
	assert.equal(results[1].playbackUrl, "file:///song.mp3");
});

test("export and import round-trip a player snapshot", async () => {
	const seen: Array<{ url: string; method: string; body?: string }> = [];
	const { client } = clientWith((call) => {
		seen.push({ url: call.url, method: call.method, body: call.body && JSON.parse(call.body) });

		if (call.method === "POST") {
			return jsonResponse({
				guildId: "7",
				track: null,
				volume: 250,
				paused: true,
				state: { time: 1, position: 65000, connected: false, ping: -1 },
				filters: {},
			});
		}

		return jsonResponse({
			version: 1,
			current: null,
			queue: [trackWith("encoded-abc", "file:///song.mp3")],
			history: [],
			position: 65000,
			volume: 250,
			filters: {},
			repeat: "Off",
			shuffle: false,
			shufflePositions: [],
			autoplay: false,
		});
	});

	const exported = await client.exportPlayer(7);

	assert.ok(seen[0].url.endsWith("/v4/sessions/session-1/players/7/export"));
	assert.equal(exported.version, 1);
	assert.equal(exported.queue.length, 1);

	const imported = await client.importPlayer(7, exported);

	assert.ok(seen[1].url.endsWith("/v4/sessions/session-1/players/7/import"));
	assert.equal(seen[1].method, "POST");
	assert.equal(imported.volume, 250);
	assert.equal(imported.paused, true);
});

test("preset crud hits the namespaced routes", async () => {
	const seen: Array<{ url: string; method: string; body?: string }> = [];
	const { client } = clientWith((call) => {
		seen.push({ url: call.url, method: call.method, body: call.body });

		if (call.method === "DELETE") {
			return new Response(null, { status: 204 });
		}

		return jsonResponse({ equalizer: [{ band: 0, gain: 0.8 }] });
	});

	await client.putFilterPreset("bass", { equalizer: [{ band: 0, gain: 0.8 }] });
	await client.getFilterPreset("bass");
	await client.listFilterPresets();
	await client.deleteFilterPreset("bass");

	assert.deepEqual(
		seen.map(({ url, method }) => `${method} ${new URL(url).pathname}`),
		[
			"PUT /v4/filters/presets/bass",
			"GET /v4/filters/presets/bass",
			"GET /v4/filters/presets",
			"DELETE /v4/filters/presets/bass",
		]
	);
	assert.equal(JSON.parse(seen[0].body ?? "{}").equalizer[0].gain, 0.8);
});

test("queue responses carry autoplay and repeat", async () => {
	const { client } = clientWith(() =>
		jsonResponse({ current: null, queue: [], history: [], position: 0, repeat: "Track", autoplay: true })
	);

	const queue = await client.getQueue(7);

	assert.equal(queue.autoplay, true);
	assert.equal(queue.repeat, "Track");
});
