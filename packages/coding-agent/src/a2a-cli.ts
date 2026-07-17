/**
 * `hoocode a2a` — A2A (Agent2Agent) discovery subcommand.
 *
 * Builds an AgentCard from the tools and skills active for the current working
 * directory, then either prints it (`--print`, the default) or serves it over
 * HTTP at `/.well-known/agent.json` (`--serve`) so other agents can discover
 * this instance.
 *
 * Read-only with respect to the model: it makes no LLM call. It mirrors the
 * structure of `resources-cli.ts`, which discovers the same skills.
 */

import chalk from "chalk";
import { APP_NAME, getAgentDir, VERSION } from "./config.js";
import {
	buildAgentCard,
	type DiscoverableSkill,
	resolveActiveTools,
	startA2ADiscoveryServer,
} from "./core/a2a/index.js";
import { DefaultResourceLoader } from "./core/resource-loader.js";
import { SettingsManager } from "./core/settings-manager.js";

interface A2AFlags {
	serve: boolean;
	port?: number;
	host?: string;
	help: boolean;
}

function parseA2AFlags(args: string[]): { flags: A2AFlags; error?: string } {
	const flags: A2AFlags = { serve: false, help: false };
	for (let i = 0; i < args.length; i++) {
		const arg = args[i];
		switch (arg) {
			case "-h":
			case "--help":
				flags.help = true;
				break;
			case "--serve":
				flags.serve = true;
				break;
			case "--print":
				flags.serve = false;
				break;
			case "--port": {
				const value = args[++i];
				const port = Number(value);
				if (!Number.isInteger(port) || port < 0 || port > 65535) {
					return { flags, error: `Invalid --port value: ${value ?? "(missing)"}` };
				}
				flags.port = port;
				flags.serve = true;
				break;
			}
			case "--host":
				flags.host = args[++i];
				flags.serve = true;
				break;
			default:
				return { flags, error: `Unknown argument: ${arg}` };
		}
	}
	return { flags };
}

function printHelp(): void {
	console.log(`${chalk.bold("Usage:")}
  ${APP_NAME} a2a [--print | --serve] [--port <n>] [--host <addr>]

Publish an A2A (Agent2Agent) AgentCard describing the tools and skills active
for the current working directory, so other agents can discover this instance.

${chalk.bold("Options:")}
  --print              Print the AgentCard as JSON and exit (default)
  --serve              Serve the card over HTTP until interrupted
  --port <n>           Port to serve on (default 41411; implies --serve)
  --host <addr>        Interface to bind (default 127.0.0.1; implies --serve)
  -h, --help           Show this help

The card is served at ${chalk.cyan("/.well-known/agent.json")} (JSON-RPC task execution is
not implemented; the card advertises discovery only). Makes no LLM call.
`);
}

/** Resolve the AgentCard for the current working directory. */
async function buildCardForCwd(url: string) {
	const cwd = process.cwd();
	const agentDir = getAgentDir();
	const settingsManager = SettingsManager.create(cwd, agentDir);
	const resourceLoader = new DefaultResourceLoader({ cwd, agentDir, settingsManager });
	await resourceLoader.reload();

	const skills: DiscoverableSkill[] = resourceLoader.getSkills().skills.map((skill) => ({
		name: skill.name,
		description: skill.description,
	}));

	const activeTools = resolveActiveTools({
		enableWebTools: settingsManager.getEnableWebTools(),
		enableBrowserTools: settingsManager.getEnableBrowserTools(),
		enableFileTools: settingsManager.getEnableFileTools(),
	});

	return buildAgentCard({
		url,
		activeTools,
		skills,
		version: VERSION,
		provider: { organization: "HooCode", url: "https://github.com/kolisachint/hoocode" },
		documentationUrl: "https://github.com/kolisachint/hoocode",
	});
}

export async function handleA2ACommand(args: string[]): Promise<boolean> {
	if (args[0] !== "a2a") {
		return false;
	}

	const { flags, error } = parseA2AFlags(args.slice(1));
	if (error) {
		console.error(chalk.red(`Error: ${error}`));
		printHelp();
		process.exit(1);
	}
	if (flags.help) {
		printHelp();
		return true;
	}

	if (!flags.serve) {
		// The endpoint URL is only meaningful when serving; for --print use a
		// placeholder base so the card is still valid and self-describing.
		const card = await buildCardForCwd(`http://${flags.host ?? "127.0.0.1"}:${flags.port ?? 41411}`);
		console.log(JSON.stringify(card, null, 2));
		return true;
	}

	// Serve. Build the card first, then re-point its `url` at the address we
	// actually bound to (important when --port 0 lets the OS choose the port).
	// The server's request handler closes over this same card object, so the
	// in-place update is reflected in what it serves.
	const card = await buildCardForCwd("http://127.0.0.1:0");
	const server = await startA2ADiscoveryServer(card, { port: flags.port, host: flags.host });
	card.url = server.url;

	console.log(chalk.green(`A2A discovery server listening on ${server.url}`));
	console.log(chalk.dim(`AgentCard: ${server.cardUrl}`));
	console.log(chalk.dim(`Skills advertised: ${card.skills.length}`));
	console.log(chalk.dim("Press Ctrl+C to stop."));

	await new Promise<void>((resolve) => {
		const shutdown = () => {
			void server.close().then(resolve);
		};
		process.once("SIGINT", shutdown);
		process.once("SIGTERM", shutdown);
	});

	return true;
}
