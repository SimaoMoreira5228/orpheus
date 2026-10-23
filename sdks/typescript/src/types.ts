export interface NodeConfig {
	host: string;
	port: number;
	password: string;
	wsPort?: number;
}

export interface TrackInfo {
	identifier: string;
	isSeekable: boolean;
	author: string;
	length: number;
	isStream: boolean;
	position: number;
	title: string;
	uri: string | null;
	artworkUrl: string | null;
	isrc: string | null;
	sourceName: string;
}

export interface Track {
	encoded?: string;
	info: TrackInfo;
	startTime?: number;
	endTime?: number;
	pluginInfo?: unknown;
	userData?: unknown;
}

export interface PlaylistInfo {
	name?: string;
	selectedTrack?: number;
}

export type LoadResultType = "Track" | "Playlist" | "Search" | "Empty" | "Error";

export interface PlaylistData {
	info: PlaylistInfo;
	pluginInfo?: unknown;
	tracks: Track[];
}

export interface DryRunItem {
	loadType: LoadResultType;
	data: LoadData;
	playbackUrl: string | null;
}

export interface AlbumRef {
	id: string;
	name: string;
	artist: string;
	artworkUrl?: string;
	uri: string;
}

export interface ArtistRef {
	id: string;
	name: string;
	artworkUrl?: string;
	uri: string;
}

export interface PlaylistRef {
	id: string;
	name: string;
	author: string;
	artworkUrl?: string;
	uri: string;
	trackCount: number;
}

export interface RichSearchResponse {
	tracks: Track[];
	albums: AlbumRef[];
	artists: ArtistRef[];
	playlists: PlaylistRef[];
}

export interface PlayerSnapshot {
	version: number;
	current?: Track | null;
	queue: Track[];
	history: Track[];
	position: number;
	volume: number;
	filters?: Filters;
	repeat?: RepeatMode;
	shuffle?: boolean;
	shufflePositions?: number[];
	autoplay?: boolean;
}

export interface LoadFailure {
	message: string;
	severity: string;
	code: string;
}

export type LoadData = Track | PlaylistData | Track[] | LoadFailure;

export interface LoadResult {
	loadType: LoadResultType;
	data: LoadData;
}

export function firstTrack(result: LoadResult): Track | undefined {
	if (Array.isArray(result.data)) {
		return result.data[0];
	}

	if ("tracks" in result.data) {
		return result.data.tracks[0];
	}

	if ("message" in result.data) {
		return undefined;
	}

	return result.data;
}

export interface PlayerStateInfo {
	time: number;
	position: number;
	connected: boolean;
	ping: number;
}

export interface VoiceStateInfo {
	token: string;
	endpoint: string;
	sessionId: string;
}

export interface PlayerState {
	guildId: string;
	track?: Track;
	volume: number;
	paused: boolean;
	state: PlayerStateInfo;
	voice?: VoiceStateInfo;
	filters: Filters;
}

export interface PlayerTrackUpdate {
	encoded?: string;
	identifier?: string;
	userData?: unknown;
}

export interface VoiceStateUpdate {
	token: string;
	endpoint: string;
	sessionId: string;
	channelId?: string;
}

export interface PlayerUpdate {
	track?: PlayerTrackUpdate | null;
	position?: number;
	volume?: number;
	paused?: boolean;
	endTime?: number | null;
	filters?: Filters;
	filterPreset?: string;
	voice?: VoiceStateUpdate;
	noReplace?: boolean;
}

export interface QueueResponse {
	current?: Track;
	queue: Track[];
	history: Track[];
	position: number;
	repeat: RepeatMode;
	autoplay: boolean;
}

export interface AddToQueueRequest {
	tracks: Track[];
	position?: number;
}

export interface QueueUpdateRequest {
	shuffle?: boolean;
	repeat?: RepeatMode;
	position?: number;
	autoplay?: boolean;
}

export interface QueuePositionResponse {
	track?: Track;
	position: number;
}

export interface SessionUpdate {
	resuming?: boolean;
	timeout?: number;
}

export interface SessionState {
	resuming: boolean;
	timeout: number;
}

export interface NodeVersion {
	semver: string;
	major: number;
	minor: number;
	patch: number;
	preRelease?: string;
}

export interface GitInfo {
	branch: string;
	commit: string;
	commitTime: number;
}

export interface PluginInfo {
	name: string;
	version: string;
}

export interface NodeInfo {
	version: NodeVersion;
	buildTime: number;
	git: GitInfo;
	jvm: string;
	lavaplayer: string;
	sourceManagers: string[];
	filters: string[];
	plugins: PluginInfo[];
}

export interface MemoryInfo {
	free: number;
	used: number;
	allocated: number;
	reservable: number;
}

export interface CpuInfo {
	cores: number;
	systemLoad: number;
	lavalinkLoad: number;
}

export interface Stats {
	players: number;
	playingPlayers: number;
	uptime: number;
	memory: MemoryInfo;
	cpu: CpuInfo;
	frameStats?: unknown;
}

export interface Version {
	version: string;
}

export type RepeatMode = "Off" | "Track" | "Queue";

export interface EqualizerBand {
	band: number;
	gain: number;
}

export interface KaraokeFilter {
	level?: number;
	monoLevel?: number;
	filterBand?: number;
	filterWidth?: number;
}

export interface TimescaleFilter {
	speed?: number;
	pitch?: number;
	rate?: number;
}

export interface TremoloFilter {
	frequency?: number;
	depth?: number;
}

export interface VibratoFilter {
	frequency?: number;
	depth?: number;
}

export interface RotationFilter {
	rotationHz?: number;
}

export interface DistortionFilter {
	sinOffset?: number;
	sinScale?: number;
	cosOffset?: number;
	cosScale?: number;
	tanOffset?: number;
	tanScale?: number;
	offset?: number;
	scale?: number;
}

export interface ChannelMixFilter {
	leftToLeft?: number;
	leftToRight?: number;
	rightToLeft?: number;
	rightToRight?: number;
}

export interface LowPassFilter {
	smoothing?: number;
}

export type TrackEndReason = "Finished" | "LoadFailed" | "Stopped" | "Replaced" | "Cleanup";

export type WsExceptionSeverity = "COMMON" | "SUSPICIOUS" | "FAULT";

export interface WsException {
	message: string;
	severity: WsExceptionSeverity;
	cause: string;
	causeStackTrace?: string;
}

export type WsTrackEvent =
	| { type: "TrackStartEvent"; track: Track; guildId: string }
	| { type: "TrackEndEvent"; track: Track; reason: TrackEndReason; guildId: string }
	| { type: "TrackExceptionEvent"; track: Track; exception: WsException; guildId: string }
	| { type: "TrackStuckEvent"; track: Track; thresholdMs: number; guildId: string }
	| { type: "WebSocketClosedEvent"; code: number; reason: string; byRemote: boolean; guildId: string };

export interface WsPlayerState {
	time: number;
	position: number;
	connected: boolean;
	ping: number;
}

export type WsServerEvent =
	| { op: "ready"; sessionId: string; resumed: boolean; sourceManagers?: string[] }
	| { op: "playerUpdate"; guildId: string; state: WsPlayerState }
	| ({ op: "stats" } & Stats)
	| ({ op: "event" } & WsTrackEvent)
	| { op: "loadtracks"; loadType: string; data: LoadData };

export interface WsPong {
	op: "pong";
	timestamp: number;
}

export interface WsPlayerTrackUpdate {
	encoded?: string;
	identifier?: string;
	userData?: unknown;
}

export interface WsVoiceData {
	token: string;
	endpoint: string;
	sessionId: string;
}

export interface WsPlayerUpdateData {
	track?: WsPlayerTrackUpdate;
	position?: number;
	endTime?: number;
	volume?: number;
	paused?: boolean;
	voice?: WsVoiceData;
	filters?: Filters;
}

export interface Filters {
	volume?: number;
	equalizer?: EqualizerBand[];
	karaoke?: KaraokeFilter;
	timescale?: TimescaleFilter;
	tremolo?: TremoloFilter;
	vibrato?: VibratoFilter;
	rotation?: RotationFilter;
	distortion?: DistortionFilter;
	channelMix?: ChannelMixFilter;
	lowPass?: LowPassFilter;
	normalization?: boolean;
	pluginFilters?: unknown;
}
