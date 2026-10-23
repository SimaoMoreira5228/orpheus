import type { WsPlayerUpdateData, WsPong, WsServerEvent, WsVoiceData } from "./types.js";

export interface WsConfig {
	host: string;
	port: number;
	password: string;
	userId: string | number;
	clientName?: string;
	sessionId?: string;
	tls?: boolean;
}

export type WsHandler = (event: WsServerEvent | WsPong) => void;

export interface WsSocket {
	send(data: string): void;
	close(): void;
	onmessage: ((event: { data: unknown }) => void) | null;
	onopen: (() => void) | null;
	onerror: ((event: unknown) => void) | null;
	onclose: (() => void) | null;
}

export type SocketFactory = (url: string, headers: Record<string, string>) => WsSocket;

function defaultSocket(url: string, headers: Record<string, string>): WsSocket {
	const socket = new (WebSocket as unknown as new (url: string, options?: { headers: Record<string, string> }) => WebSocket)(
		url,
		{ headers }
	);

	return socket as unknown as WsSocket;
}

const SERVER_OPS = new Set(["ready", "playerUpdate", "stats", "event", "loadtracks"]);

export function parseServerFrame(data: unknown): WsServerEvent | WsPong | undefined {
	let parsed: unknown;

	try {
		parsed = JSON.parse(String(data));
	} catch {
		return undefined;
	}

	if (typeof parsed !== "object" || parsed === null) {
		return undefined;
	}

	const op = (parsed as { op?: unknown }).op;

	if (op === "pong") {
		return parsed as WsPong;
	}

	if (typeof op === "string" && SERVER_OPS.has(op)) {
		return parsed as WsServerEvent;
	}

	return undefined;
}

export class OrpheusWsClient {
	private socket: WsSocket | null = null;

	constructor(
		private config: WsConfig,
		private handler: WsHandler,
		private factory: SocketFactory = defaultSocket
	) {}

	url(): string {
		const scheme = this.config.tls === true ? "wss" : "ws";

		return `${scheme}://${this.config.host}:${this.config.port}/v4/websocket`;
	}

	headers(): Record<string, string> {
		const headers: Record<string, string> = {
			Authorization: this.config.password,
			"User-Id": String(this.config.userId),
			"Client-Name": this.config.clientName ?? "orpheus-sdk",
		};

		if (this.config.sessionId !== undefined) {
			headers["Session-Id"] = this.config.sessionId;
		}

		return headers;
	}

	connect(): Promise<void> {
		return new Promise((resolve, reject) => {
			const socket = this.factory(this.url(), this.headers());

			socket.onopen = () => {
				this.socket = socket;
				resolve();
			};

			socket.onerror = (event: unknown) => {
				if (this.socket === null) {
					reject(new Error(`Orpheus websocket failed to open: ${String(event)}`));
				}
			};

			socket.onclose = () => {
				this.socket = null;
			};

			socket.onmessage = (event: { data: unknown }) => {
				const parsed = parseServerFrame(event.data);

				if (parsed !== undefined) {
					this.handler(parsed);
				}
			};
		});
	}

	private send(message: unknown): void {
		const socket = this.socket;

		if (socket === null) {
			throw new Error("Orpheus websocket is not connected: call connect() first");
		}

		socket.send(JSON.stringify(message));
	}

	ping(timestamp: number): void {
		this.send({ op: "ping", timestamp });
	}

	destroy(guildId: string | number): void {
		this.send({ op: "destroy", guildId: String(guildId) });
	}

	configureResuming(key?: string, timeout?: number): void {
		const message: Record<string, unknown> = { op: "configureResuming" };

		if (key !== undefined) {
			message.key = key;
		}

		if (timeout !== undefined) {
			message.timeout = timeout;
		}

		this.send(message);
	}

	playerUpdate(guildId: string | number, data: WsPlayerUpdateData): void {
		this.send({ op: "playerUpdate", guildId: String(guildId), data });
	}

	voiceUpdate(guildId: string | number, data: WsVoiceData): void {
		this.send({ op: "voiceUpdate", guildId: String(guildId), data });
	}

	loadTracks(identifier: string): void {
		this.send({ op: "loadtracks", identifier });
	}

	close(): void {
		const socket = this.socket;

		this.socket = null;
		socket?.close();
	}
}
