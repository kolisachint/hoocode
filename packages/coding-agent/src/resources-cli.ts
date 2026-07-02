import { existsSync, readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import chalk from "chalk";
import { APP_NAME, CONFIG_DIR_NAME, getAgentDir, getHooCodeDir } from "./config.js";
import { loadAgentRegistry } from "./core/agent-registry.js";
import { DefaultResourceLoader } from "./core/resource-loader.js";
import { SettingsManager } from "./core/settings-manager.js";
import type { SourceInfo } from "./core/source-info.js";

interface ResourceRow {
	name: string;
	detail?: string;
	origin: string;
	path: string;
}

/**
 * Derive a short origin label from a resource path and scope. Distinguishes the
 * four discovery surfaces a user cares about when debugging: project vs user, and
 * the cross-vendor `.agents/` vs Claude Code `.claude/` directories.
 */
function classifyOrigin(path: string, scope: string): string {
	if (path.includes(`${join("", ".agents", "")}`) || path.includes("/.agents/")) return `${scope} (.agents)`;
	if (path.includes("/.claude/")) return `${scope} (.claude)`;
	if (path.includes(`${CONFIG_DIR_NAME}/`)) return `${scope} (${CONFIG_DIR_NAME})`;
	return scope;
}

function originFromSourceInfo(sourceInfo: SourceInfo): string {
	return classifyOrigin(sourceInfo.path, sourceInfo.scope);
}

/** Map an AgentSource value to the same origin vocabulary used elsewhere. */
function originFromAgentSource(source: string, path: string | undefined): string {
	switch (source) {
		case "builtin":
			return "builtin";
		case "user":
			return path ? classifyOrigin(path, "user") : "user";
		case "project":
			return path ? classifyOrigin(path, "project") : "project";
		case "claude-user":
			return "user (.claude)";
		case "claude-project":
			return "project (.claude)";
		default:
			return source;
	}
}

interface DiscoveredMcpServer {
	name: string;
	path: string;
	scope: "user" | "project";
}

/**
 * Read-only replica of the MCP discovery surfaces in
 * `extensions/core/mcp-loader.ts` (setupMcpLoader). First-wins by name across all
 * sources. Kept in sync with that loader; it is the source of truth.
 */
function discoverMcpServers(cwd: string): DiscoveredMcpServer[] {
	const servers: DiscoveredMcpServer[] = [];
	const seen = new Set<string>();
	const add = (name: string, path: string, scope: "user" | "project"): void => {
		if (!name || seen.has(name)) return;
		seen.add(name);
		servers.push({ name, path, scope });
	};

	const standardFiles: Array<{ path: string; scope: "user" | "project" }> = [
		{ path: join(homedir(), ".agents", "mcp.json"), scope: "user" },
		{ path: join(cwd, ".agents", "mcp.json"), scope: "project" },
		{ path: join(homedir(), ".config", "claude", "mcp.json"), scope: "user" },
	];
	for (const { path, scope } of standardFiles) {
		if (!existsSync(path)) continue;
		try {
			const config = JSON.parse(readFileSync(path, "utf8")) as { mcpServers?: Record<string, unknown> };
			for (const name of Object.keys(config.mcpServers ?? {})) {
				add(name, path, scope);
			}
		} catch {
			// Ignore malformed config; the loader does the same.
		}
	}

	const perServerDirs: Array<{ dir: string; scope: "user" | "project" }> = [
		{ dir: join(getHooCodeDir(), "mcp-servers"), scope: "user" },
		{ dir: join(cwd, CONFIG_DIR_NAME, "mcp-servers"), scope: "project" },
	];
	for (const { dir, scope } of perServerDirs) {
		if (!existsSync(dir)) continue;
		let files: string[];
		try {
			files = readdirSync(dir).filter((f) => f.endsWith(".json"));
		} catch {
			continue;
		}
		for (const file of files) {
			const path = join(dir, file);
			try {
				const config = JSON.parse(readFileSync(path, "utf8")) as { name?: string };
				if (config.name) add(config.name, path, scope);
			} catch {
				// Ignore malformed config.
			}
		}
	}

	return servers;
}

/** Collapse a description to a single short line for the table. */
function oneLine(text: string | undefined): string | undefined {
	if (!text) return undefined;
	const firstLine = text
		.split("\n")
		.map((l) => l.trim())
		.find((l) => l.length > 0);
	if (!firstLine) return undefined;
	return firstLine.length > 80 ? `${firstLine.slice(0, 77)}...` : firstLine;
}

function printSection(title: string, rows: ResourceRow[]): void {
	console.log(chalk.bold(`${title} (${rows.length})`));
	if (rows.length === 0) {
		console.log(chalk.dim("  (none)"));
		console.log();
		return;
	}
	for (const row of rows) {
		const summary = oneLine(row.detail);
		const detail = summary ? chalk.dim(` - ${summary}`) : "";
		console.log(`  ${row.name}${detail}  ${chalk.dim(`[${row.origin}]`)}`);
		console.log(chalk.dim(`    ${row.path}`));
	}
	console.log();
}

function printHelp(): void {
	console.log(`${chalk.bold("Usage:")}
  ${APP_NAME} resources

Print the skills, subagents, slash commands, and MCP servers discovered for the
current working directory, with their source path and origin. Read-only; makes
no LLM call.
`);
}

export async function handleResourcesCommand(args: string[]): Promise<boolean> {
	if (args[0] !== "resources") {
		return false;
	}

	if (args.includes("-h") || args.includes("--help")) {
		printHelp();
		return true;
	}

	const cwd = process.cwd();
	const agentDir = getAgentDir();
	const settingsManager = SettingsManager.create(cwd, agentDir);
	const resourceLoader = new DefaultResourceLoader({ cwd, agentDir, settingsManager });
	await resourceLoader.reload();

	const skills = resourceLoader.getSkills().skills.map<ResourceRow>((skill) => ({
		name: skill.name,
		detail: skill.description,
		origin: originFromSourceInfo(skill.sourceInfo),
		path: skill.sourceInfo.path,
	}));

	const agents = loadAgentRegistry({ cwd, agentDir })
		.list()
		.map<ResourceRow>((agent) => ({
			name: agent.name,
			detail: agent.description,
			origin: originFromAgentSource(agent.source, agent.filePath),
			path: agent.filePath ?? "<builtin>",
		}));

	const slashCommands = resourceLoader.getPrompts().prompts.map<ResourceRow>((prompt) => ({
		name: `/${prompt.name}`,
		detail: prompt.description,
		origin: originFromSourceInfo(prompt.sourceInfo),
		path: prompt.sourceInfo.path,
	}));

	const mcpServers = discoverMcpServers(cwd).map<ResourceRow>((server) => ({
		name: server.name,
		origin: classifyOrigin(server.path, server.scope),
		path: server.path,
	}));

	console.log(chalk.dim(`Resources discovered for ${cwd}\n`));
	printSection("Skills", skills);
	printSection("Subagents", agents);
	printSection("Slash commands", slashCommands);
	printSection("MCP servers", mcpServers);

	return true;
}
