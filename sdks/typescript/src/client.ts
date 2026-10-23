import type {
	AddToQueueRequest,
	DryRunItem,
	Filters,
	LoadResult,
	NodeConfig,
	NodeInfo,
	PlayerSnapshot,
	PlayerState,
	PlayerTrackUpdate,
	PlayerUpdate,
	QueuePositionResponse,
	QueueResponse,
	QueueUpdateRequest,
	RichSearchResponse,
	SessionState,
	SessionUpdate,
	Stats,
	Track,
	Version,
} from "./types.js";

export type FetchFn = typeof fetch;

export class OrpheusError extends Error {
	status: number;
	code: string | undefined;
	body: string;

	constructor(status: number, body: string, path: string) {
		let envelope: { code?: unknown; message?: unknown } | undefined;

		try {
			envelope = JSON.parse(body) as { code?: unknown; message?: unknown };
		} catch {
			envelope = undefined;
		}

		const message = typeof envelope?.message === "string" ? envelope.message : path;

		super(`Orpheus request failed: ${status} ${message}`);

		this.name = "OrpheusError";
		this.status = status;
		this.code = typeof envelope?.code === "string" ? envelope.code : undefined;
		this.body = body;
	}
}

function defaultSession(): string {
	return "default";
}

export class OrpheusClient {
	private node: NodeConfig;
	private sessionId: string;
	private fetchFn: FetchFn;

	constructor(node: NodeConfig, sessionId: string = defaultSession(), fetchFn: FetchFn = fetch) {
		this.node = node;
		this.sessionId = sessionId;
		this.fetchFn = fetchFn;
	}

	withSession(sessionId: string): OrpheusClient {
		return new OrpheusClient(this.node, sessionId, this.fetchFn);
	}

	private baseUrl(): string {
		return `http://${this.node.host}:${this.node.port}`;
	}

	private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
		const response = await this.fetchFn(`${this.baseUrl()}${path}`, {
			method,
			headers: { Authorization: this.node.password, ...(body !== undefined ? { "Content-Type": "application/json" } : {}) },
			...(body !== undefined ? { body: JSON.stringify(body) } : {}),
		});

		if (!response.ok) {
			throw new OrpheusError(response.status, await response.text(), path);
		}

		if (response.status === 204) {
			return undefined as T;
		}

		return (await response.json()) as T;
	}

	async loadTracks(identifier: string): Promise<LoadResult> {
		const path = `/v4/loadtracks?identifier=${encodeURIComponent(identifier)}`;

		return this.request<LoadResult>("GET", path);
	}

	async decodeTrack(encoded: string): Promise<Track> {
		const path = `/v4/decodetrack?encodedTrack=${encodeURIComponent(encoded)}`;

		return this.request<Track>("GET", path);
	}

	async decodeTracks(tracks: string[]): Promise<Track[]> {
		return this.request<Track[]>("POST", "/v4/decodetracks", { tracks });
	}

	async loadTracksBatch(identifiers: string[]): Promise<LoadResult[]> {
		return this.request<LoadResult[]>("POST", "/v4/loadtracks", { identifiers });
	}

	async loadSearch(query: string): Promise<RichSearchResponse> {
		return this.request<RichSearchResponse>("GET", `/v4/loadsearch?query=${encodeURIComponent(query)}`);
	}

	async exportPlayer(guildId: string | number): Promise<PlayerSnapshot> {
		return this.request<PlayerSnapshot>("GET", `/v4/sessions/${this.sessionId}/players/${guildId}/export`);
	}

	async importPlayer(guildId: string | number, snapshot: PlayerSnapshot): Promise<PlayerState> {
		return this.request<PlayerState>("POST", `/v4/sessions/${this.sessionId}/players/${guildId}/import`, snapshot);
	}

	async loadTracksDryRun(identifiers: string[]): Promise<DryRunItem[]> {
		return this.request<DryRunItem[]>("POST", "/v4/loadtracks/dry-run", { identifiers });
	}

	async getAllPlayers(): Promise<PlayerState[]> {
		return this.request<PlayerState[]>("GET", `/v4/sessions/${this.sessionId}/players`);
	}

	async getPlayer(guildId: string | number): Promise<PlayerState> {
		return this.request<PlayerState>("GET", `/v4/sessions/${this.sessionId}/players/${guildId}`);
	}

	async updatePlayer(guildId: string | number, update: PlayerUpdate): Promise<PlayerState> {
		return this.request<PlayerState>("PATCH", `/v4/sessions/${this.sessionId}/players/${guildId}`, update);
	}

	async deletePlayer(guildId: string | number): Promise<void> {
		await this.request<void>("DELETE", `/v4/sessions/${this.sessionId}/players/${guildId}`);
	}

	async play(guildId: string | number, track: Track): Promise<PlayerState> {
		const trackUpdate: PlayerTrackUpdate =
			track.encoded !== undefined ? { encoded: track.encoded } : { identifier: track.info.uri ?? undefined };

		return this.updatePlayer(guildId, { track: trackUpdate, position: 0, paused: false });
	}

	async pause(guildId: string | number, paused: boolean): Promise<PlayerState> {
		return this.updatePlayer(guildId, { paused });
	}

	async setVolume(guildId: string | number, volume: number): Promise<PlayerState> {
		return this.updatePlayer(guildId, { volume });
	}

	async seek(guildId: string | number, position: number): Promise<PlayerState> {
		return this.updatePlayer(guildId, { position });
	}

	async getQueue(guildId: string | number): Promise<QueueResponse> {
		return this.request<QueueResponse>("GET", `/v4/sessions/${this.sessionId}/players/${guildId}/queue`);
	}

	async addToQueue(guildId: string | number, request: AddToQueueRequest): Promise<QueueResponse> {
		return this.request<QueueResponse>("POST", `/v4/sessions/${this.sessionId}/players/${guildId}/queue`, request);
	}

	async updateQueue(guildId: string | number, request: QueueUpdateRequest): Promise<QueueResponse> {
		return this.request<QueueResponse>("PATCH", `/v4/sessions/${this.sessionId}/players/${guildId}/queue`, request);
	}

	async removeFromQueue(guildId: string | number, position: number): Promise<QueuePositionResponse> {
		return this.request<QueuePositionResponse>(
			"DELETE",
			`/v4/sessions/${this.sessionId}/players/${guildId}/queue/${position}`
		);
	}

	async setVoiceChannel(guildId: string | number, channelId: string | number): Promise<void> {
		await this.request<void>("PUT", `/v4/sessions/${this.sessionId}/players/${guildId}/voice-channel`, { channelId });
	}

	async updateSession(update: SessionUpdate): Promise<SessionState> {
		return this.request<SessionState>("PATCH", `/v4/sessions/${this.sessionId}`, update);
	}

	async listFilterPresets(): Promise<Record<string, Filters>> {
		return this.request<Record<string, Filters>>("GET", "/v4/filters/presets");
	}

	async getFilterPreset(name: string): Promise<Filters> {
		return this.request<Filters>("GET", `/v4/filters/presets/${encodeURIComponent(name)}`);
	}

	async putFilterPreset(name: string, filters: Filters): Promise<Filters> {
		return this.request<Filters>("PUT", `/v4/filters/presets/${encodeURIComponent(name)}`, filters);
	}

	async deleteFilterPreset(name: string): Promise<void> {
		await this.request<void>("DELETE", `/v4/filters/presets/${encodeURIComponent(name)}`);
	}

	async getInfo(): Promise<NodeInfo> {
		return this.request<NodeInfo>("GET", "/v4/info");
	}

	async getStats(): Promise<Stats> {
		return this.request<Stats>("GET", "/v4/stats");
	}

	async getVersion(): Promise<Version> {
		return this.request<Version>("GET", "/version");
	}
}
