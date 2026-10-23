import "dotenv/config";
import { Client, EmbedBuilder, GatewayIntentBits } from "discord.js";
import { LavalinkManager } from "lavalink-client";

const config = {
	discordToken: process.env.DISCORD_TOKEN || "YOUR_DISCORD_TOKEN",
	lavalink: {
		host: process.env.ORPHEUS_HOST || "127.0.0.1",
		port: parseInt(process.env.ORPHEUS_PORT || "2333"),
		password: process.env.ORPHEUS_PASSWORD || "youshallnotpass",
		wsPort: parseInt(process.env.ORPHEUS_WS_PORT || "2333"),
	},
};

const client = new Client({
	intents: [
		GatewayIntentBits.Guilds,
		GatewayIntentBits.GuildVoiceStates,
		GatewayIntentBits.GuildMessages,
		GatewayIntentBits.MessageContent,
	],
});

const manager = new LavalinkManager({
	nodes: [
		{
			id: "orpheus-node",
			host: config.lavalink.host,
			port: config.lavalink.port,
			wsPort: config.lavalink.wsPort,
			authorization: config.lavalink.password,
			secure: false,
		},
	],
	sendToShard: (guildId, payload) => {
		const guild = client.guilds.cache.get(guildId);
		if (guild && guild.shard) {
			guild.shard.send(payload);
		}
	},
});

client.once("clientReady", async () => {
	console.log(`Logged in as ${client.user.tag}`);
	try {
		await manager.init({ id: client.user.id, username: client.user.username });
		console.log("Lavalink manager ready");
	} catch (err) {
		console.error("Failed to init lavalink manager:", err);
	}

	manager.nodeManager.on("create", (node) => {
		console.log(`Node created: ${node.id}`);
	});

	manager.nodeManager.on("remove", (node) => {
		console.log(`Node removed: ${node.id}`);
	});

	manager.nodeManager.on("error", (node, error) => {
		console.error(`Node ${node.id} error:`, error);
	});

	manager.nodeManager.on("connect", (node) => {
		console.log(`Node connected: ${node.id}`);
	});

	manager.nodeManager.on("disconnect", (node, code, reason) => {
		console.log(`Node disconnected: ${node.id}, code: ${code}, reason: ${reason}`);
	});

	console.log("Available nodes:", manager.nodeManager.nodes.size);
	manager.nodeManager.nodes.forEach((node) => {
		console.log(`Node: ${node.id}, connected: ${node.connected}, sessionId: ${node.sessionId}`);
	});
});

client.on("raw", (d) => {
	manager.sendRawData(d);
});

client.on("messageCreate", async (message) => {
	if (message.author.bot) return;

	const args = message.content.slice(1).split(" ");
	const command = args.shift().toLowerCase();
	console.log(`Received command: ${command}, args: ${args.join(" ")}`);

	if (!message.content.startsWith("!")) return;

	const member = message.member;
	if (!member) return;

	const voiceChannel = member.voice.channel;

	switch (command) {
		case "play": {
			if (!voiceChannel) {
				return message.reply("You must be in a voice channel to play music!");
			}

			const query = args.join(" ");
			if (!query) {
				return message.reply("Please provide a search query!");
			}

			try {
				const hasConnected = Array.from(manager.nodeManager.nodes.values()).some((n) => n.connected);
				if (!hasConnected) {
					return message.reply("No Orpheus node connected yet, please try again later.");
				}

				const player = manager.createPlayer({
					guildId: message.guildId,
					voiceChannelId: voiceChannel.id,
					textChannelId: message.channelId,
					node: "orpheus-node",
					volume: 80,
					selfDeaf: true,
					selfMute: false,
				});

				if (!player.connected) {
					console.log(`[play] Connecting player to voice channel: ${voiceChannel.name} (${voiceChannel.id})`);
					await player.connect();
					console.log(`[play] Player connected to voice channel`);
					// Tell orpheus the voice channel id so its DAVE MLS group matches the call.
					try {
						const node = manager.nodeManager.nodes.get("orpheus-node");
						if (node?.sessionId) {
							await fetch(
								`http://${config.lavalink.host}:${config.lavalink.port}/v4/sessions/${node.sessionId}/players/${message.guildId}/voice-channel`,
								{
									method: "PUT",
									headers: { "Content-Type": "application/json", Authorization: config.lavalink.password },
									body: JSON.stringify({ channelId: voiceChannel.id }),
								}
							);
						}
					} catch (error) {
						console.error(`[play] Failed to report voice channel: ${error?.message ?? error}`);
					}
				}

				const results = await player.search({ query: query, source: "ytsearch" }, message.author.id);
				console.log(`[play] Search results: loadType=${results.loadType}, tracks=${results.tracks.length}`);

				if (results.loadType === "error") {
					return message.reply("Could not load that track: the source blocked playback.");
				}

				if (!results.tracks.length) {
					return message.reply("No tracks found!");
				}

				if (!player.queue.current) {
					player.queue.add(results.tracks[0]);
					console.log(`[play] Starting playback: ${results.tracks[0].info.title}`);
					await player.play({ track: results.tracks[0] });
					const embed = new EmbedBuilder()
						.setColor(0x00ff00)
						.setTitle("Now Playing")
						.setDescription(results.tracks[0].info.title);
					message.reply({ embeds: [embed] });
				} else {
					player.queue.add(results.tracks[0]);
					console.log(`[play] Added to queue: ${results.tracks[0].info.title}`);
					const embed = new EmbedBuilder()
						.setColor(0x00ff00)
						.setTitle("Added to Queue")
						.setDescription(results.tracks[0].info.title);
					message.reply({ embeds: [embed] });
				}
			} catch (error) {
				console.error("Play error:", error);
				console.error(error.stack);
				message.reply(`Error: ${error.message}`);
			}
			break;
		}

		case "pause": {
			const player = manager.getPlayer(message.guildId);
			if (!player) {
				return message.reply("No music playing!");
			}

			await player.pause(true);
			message.reply("Paused ⏸️");
			break;
		}

		case "resume": {
			const player = manager.getPlayer(message.guildId);
			if (!player) {
				return message.reply("No music playing!");
			}

			await player.pause(false);
			message.reply("Resumed ▶️");
			break;
		}

		case "stop": {
			const player = manager.getPlayer(message.guildId);
			if (!player) {
				return message.reply("No music playing!");
			}

			await player.destroy("StopCommand");
			message.reply("Stopped ⏹️");
			break;
		}

		case "skip": {
			const player = manager.getPlayer(message.guildId);
			if (!player) {
				return message.reply("No music playing!");
			}

			if (player.queue.tracks.length === 0) {
				await player.stopPlaying(false);
			} else {
				await player.skip();
			}

			message.reply("Skipped ⏭️");
			break;
		}

		case "queue": {
			const player = manager.getPlayer(message.guildId);
			if (!player || !player.queue.tracks.length) {
				return message.reply("Queue is empty!");
			}

			const queueList = player.queue.tracks
				.slice(0, 10)
				.map((track, i) => `${i + 1}. ${track.info.title}`)
				.join("\n");

			const embed = new EmbedBuilder().setColor(0x00ff00).setTitle("Queue").setDescription(queueList);

			message.reply({ embeds: [embed] });
			break;
		}

		case "volume": {
			const player = manager.getPlayer(message.guildId);
			if (!player) {
				return message.reply("No music playing!");
			}

			const volume = parseInt(args[0]);
			if (isNaN(volume)) {
				return message.reply(`Current volume: ${player.volume}%`);
			}

			await player.setVolume(volume);
			message.reply(`Volume set to ${volume}%`);
			break;
		}

		case "help": {
			const embed = new EmbedBuilder().setColor(0x00ff00).setTitle("Orpheus Bot Commands").setDescription(`
!play <query> - Play a track
!pause - Pause playback
!resume - Resume playback  
!stop - Stop and disconnect
!skip - Skip current track
!volume [0-1000] - Set or show volume
!queue - Show queue
!help - Show this help
                `);

			message.reply({ embeds: [embed] });
			break;
		}
	}
});

client.login(config.discordToken);
