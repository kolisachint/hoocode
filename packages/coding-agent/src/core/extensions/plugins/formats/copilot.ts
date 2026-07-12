/**
 * GitHub Copilot `.github` format.
 *
 * Copilot's customization surface differs from Claude Code's: it lives under
 * `.github/` and uses prompt files and chat modes rather than a `skills/` /
 * `agents/` tree. This adapter maps between that layout and hoocode's
 * {@link NormalizedPlugin} model:
 *
 *   marker/manifest   .github/copilot-plugin.json      { name, version, description }
 *   skills + commands .github/prompts/<name>.prompt.md   → commandsDir
 *   subagents         .github/chatmodes/<name>.chatmode.md → agentsDir
 *   MCP servers       .github/mcp.json ({ servers } | { mcpServers })
 *   hooks             .github/hooks/hooks.json           (hoocode extension; Copilot has no hooks)
 *
 * Copilot conventions move quickly; this file is the single place that encodes
 * them, so tracking an upstream change never reaches beyond this adapter. The
 * capability files carry frontmatter that is a superset valid for both Copilot
 * and hoocode's loaders (e.g. `name` + comma-separated `tools`), so an authored
 * Copilot plugin both round-trips here and loads natively.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type { NormalizedPlugin } from "../manifest.js";
import { authoredHooksToConfig, dirIfExists, emitJson, emitMarkdown, normalizeMcp, readJson, slug } from "./shared.js";
import type { EmittedFile, PluginDraft, PluginFormatAdapter } from "./types.js";

const MARKER_DIR = ".github";
const MANIFEST_FILE = "copilot-plugin.json";
const PROMPTS_DIR = path.join(".github", "prompts");
const CHATMODES_DIR = path.join(".github", "chatmodes");
const MCP_FILE = path.join(".github", "mcp.json");
const HOOKS_FILE = path.join(".github", "hooks", "hooks.json");

const manifestRelPath = path.join(MARKER_DIR, MANIFEST_FILE);

interface CopilotManifest {
	name?: string;
	version?: string;
	description?: string;
	author?: string | { name?: string };
}

/** Read `.github/hooks/hooks.json` into the shared hooks event-map shape. */
function readCopilotHooks(root: string): NormalizedPlugin["hooks"] {
	const file = path.join(root, HOOKS_FILE);
	if (!fs.existsSync(file)) return undefined;
	const raw = readJson<{ hooks?: Record<string, unknown> } | Record<string, unknown>>(file);
	if (!raw || typeof raw !== "object") return undefined;
	const config = ("hooks" in raw && raw.hooks ? raw.hooks : raw) as NormalizedPlugin["hooks"];
	return config && Object.keys(config).length > 0 ? config : undefined;
}

export const copilotFormat: PluginFormatAdapter = {
	id: "copilot",
	platform: "github",
	precedence: 2,
	label: "GitHub Copilot (.github)",
	marketplaceFile: path.join(MARKER_DIR, "marketplace.json"),

	detectPlugin(root: string): boolean {
		return fs.existsSync(path.join(root, manifestRelPath));
	},

	parsePlugin(root: string): NormalizedPlugin | null {
		const manifestPath = path.join(root, manifestRelPath);
		const raw = readJson<CopilotManifest>(manifestPath);
		if (!raw) return null;

		const id = (raw.name ?? path.basename(root)).trim();
		if (!id) return null;

		const author = typeof raw.author === "string" ? raw.author : raw.author?.name;

		return {
			id,
			version: raw.version,
			description: raw.description,
			author,
			root,
			manifestPath,
			format: "copilot",
			// Single-format view; the registry widens this to every format present.
			supportPlatform: ["github"],
			// Copilot has no skills dir; prompts are command-shaped and map to commandsDir.
			commandsDir: dirIfExists(root, PROMPTS_DIR),
			agentsDir: dirIfExists(root, CHATMODES_DIR),
			hooks: readCopilotHooks(root),
			// `.github/mcp.json` only (no inline manifest MCP in the Copilot layout).
			mcpServers: normalizeMcp(undefined, root, MCP_FILE),
			// Providers are a native-only concept.
			providers: undefined,
		};
	},

	emit(draft: PluginDraft): EmittedFile[] {
		const files: EmittedFile[] = [];

		files.push({
			path: manifestRelPath,
			content: emitJson({
				name: draft.id,
				...(draft.version ? { version: draft.version } : {}),
				...(draft.description ? { description: draft.description } : {}),
				...(draft.author ? { author: draft.author } : {}),
			}),
		});

		// Skills and commands are both authored as Copilot prompt files.
		for (const s of draft.skills ?? []) {
			files.push({
				path: path.join(PROMPTS_DIR, `${slug(s.name)}.prompt.md`),
				content: emitMarkdown({ name: s.name, description: s.description }, s.body),
			});
		}
		for (const c of draft.commands ?? []) {
			files.push({
				path: path.join(PROMPTS_DIR, `${slug(c.name)}.prompt.md`),
				content: emitMarkdown({ name: c.name, description: c.description }, c.body),
			});
		}
		// Subagents become chat modes. Frontmatter is a superset valid for both
		// Copilot (description/tools/model) and hoocode's agent loader (name +
		// comma-separated tools).
		for (const a of draft.agents ?? []) {
			files.push({
				path: path.join(CHATMODES_DIR, `${slug(a.name)}.chatmode.md`),
				content: emitMarkdown({ name: a.name, description: a.description, tools: a.tools, model: a.model }, a.body),
			});
		}
		if (draft.mcpServers?.length) {
			files.push({
				path: MCP_FILE,
				content: emitJson({
					servers: Object.fromEntries(
						draft.mcpServers.map((s) => [
							s.name,
							{ command: s.command, ...(s.args ? { args: s.args } : {}), ...(s.env ? { env: s.env } : {}) },
						]),
					),
				}),
			});
		}
		if (draft.hooks?.length) {
			files.push({ path: HOOKS_FILE, content: emitJson({ hooks: authoredHooksToConfig(draft.hooks) }) });
		}
		return files;
	},
};
