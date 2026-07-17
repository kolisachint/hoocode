/**
 * GitHub Copilot format.
 *
 * hoocode's preferred Copilot home is the `.github/plugin/` marker directory —
 * the convention used by the real-world plugins indexed by
 * github/copilot-plugins (e.g. microsoft/work-iq) and the documented location
 * for GitHub-hosted marketplaces. The official Copilot CLI plugin reference
 * (docs.github.com copilot/reference/copilot-cli-reference/cli-plugin-reference,
 * "File locations"; verified 2026-07) accepts several locations for both files
 * (`.plugin/`, plugin root, `.github/plugin/`, `.claude-plugin/`), so this
 * adapter reads them all — `.github/plugin/` first, then root, then `.plugin/`,
 * then the legacy hoocode layout — and writes `.github/plugin/`.
 * (`.claude-plugin/` belongs to the Claude adapter, which wins precedence, so a
 * `.claude-plugin`-only directory correctly parses as a Claude plugin; Copilot
 * reads those natively anyway.) The capability tree matches the Claude layout:
 *
 *   skills            skills/<name>/SKILL.md            (or the manifest's `skills` path override)
 *   commands          commands/<name>.md
 *   subagents         agents/<name>.agent.md            (Copilot's custom-agent convention — the
 *                                                        `.agent.md` suffix and a YAML-list `tools`
 *                                                        frontmatter are what Copilot recognizes; the
 *                                                        reader still accepts bare `.md` names)
 *   hooks             hooks.json or hooks/hooks.json    ({ description?, hooks: { Event: [...] } })
 *   MCP servers       .mcp.json                         ({ mcpServers } | { servers })
 *
 * Workspace-level (non-plugin) conventions, per the current GitHub Copilot
 * customization docs (docs.github.com — agent skills, custom agents, prompt
 * files; verified 2026-07):
 *
 *   skills            .github/skills/<name>/SKILL.md    (agentskills.io open standard)
 *   custom agents     .github/agents/<name>.agent.md    (frontmatter: name, description,
 *                                                        tools as a YAML list, model, ...)
 *   prompt files      .github/prompts/<name>.prompt.md  (VS Code / Copilot prompt files)
 *
 * Earlier hoocode releases authored a different Copilot mapping
 * (`.github/copilot-plugin.json` + `.github/prompts/*.prompt.md` +
 * `.github/chatmodes/*.chatmode.md`); the reader still accepts that layout as a
 * legacy fallback so previously authored plugins keep loading. Copilot
 * conventions move quickly; this file is the single place that encodes them,
 * so tracking an upstream change never reaches beyond this adapter.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type { NormalizedPlugin } from "../manifest.js";
import {
	authoredHooksToConfig,
	dirIfExists,
	emitJson,
	emitMarkdown,
	normalizeHooks,
	normalizeMcp,
	parseAuthor,
	type RawManifest,
	readJson,
	resolveCapabilityDir,
	slug,
	toolsYamlList,
} from "./shared.js";
import type { EmittedFile, PluginDraft, PluginFormatAdapter } from "./types.js";

const MARKER_DIR = ".github";
/**
 * Manifest probe order: `.github/plugin/plugin.json` first — hoocode's
 * preferred Copilot home, matching the real-world plugins indexed by
 * github/copilot-plugins (e.g. microsoft/work-iq) — then the Copilot CLI
 * reference's other locations (root `plugin.json`, `.plugin/plugin.json`) and
 * hoocode's legacy `.github/copilot-plugin.json`. (`.claude-plugin/plugin.json`,
 * also probed by the CLI, belongs to the Claude adapter.)
 */
const MANIFEST_REL_PATHS = [
	path.join(MARKER_DIR, "plugin", "plugin.json"),
	"plugin.json",
	path.join(".plugin", "plugin.json"),
	path.join(MARKER_DIR, "copilot-plugin.json"),
] as const;
/** Authored manifest location: the preferred `.github/plugin/` home. */
const emitManifestRelPath = path.join(MARKER_DIR, "plugin", "plugin.json");

// Legacy authored layout (read-only fallbacks).
const LEGACY_PROMPTS_DIR = path.join(MARKER_DIR, "prompts");
const LEGACY_CHATMODES_DIR = path.join(MARKER_DIR, "chatmodes");
const LEGACY_MCP_FILE = path.join(MARKER_DIR, "mcp.json");
const LEGACY_HOOKS_FILE = path.join(MARKER_DIR, "hooks", "hooks.json");

/** Read a hooks JSON file (either `{ hooks: {...} }` or a bare event map). */
function readHooksFile(root: string, rel: string): NormalizedPlugin["hooks"] {
	const file = path.join(root, rel);
	if (!fs.existsSync(file)) return undefined;
	const raw = readJson<{ hooks?: Record<string, unknown> } | Record<string, unknown>>(file);
	if (!raw || typeof raw !== "object") return undefined;
	const config = ("hooks" in raw && raw.hooks ? raw.hooks : raw) as NormalizedPlugin["hooks"];
	return config && Object.keys(config).length > 0 ? config : undefined;
}

function manifestPathFor(root: string): string | undefined {
	for (const rel of MANIFEST_REL_PATHS) {
		if (readJson(path.join(root, rel)) != null) return path.join(root, rel);
	}
	return undefined;
}

export const copilotFormat: PluginFormatAdapter = {
	id: "copilot",
	platform: "github",
	precedence: 2,
	label: "GitHub Copilot (.github)",
	// .github/plugin/ first (hoocode's preferred Copilot home and the documented
	// location for GitHub-hosted marketplaces), then the legacy .github/ spot,
	// then the Copilot CLI's other probe locations (root, .plugin/).
	marketplaceFiles: [
		path.join(MARKER_DIR, "plugin", "marketplace.json"),
		path.join(MARKER_DIR, "marketplace.json"),
		"marketplace.json",
		path.join(".plugin", "marketplace.json"),
	],

	workspace: {
		root: MARKER_DIR,
		// Copilot reads repo skills from .github/skills (also .claude/skills and
		// .agents/skills, which the sibling adapters cover).
		emitSkill: (s) => ({
			path: path.join(MARKER_DIR, "skills", slug(s.name), "SKILL.md"),
			content: emitMarkdown({ name: s.name, description: s.description }, s.body),
		}),
		// Custom agents take `tools` as a YAML list (not the Claude comma string).
		emitAgent: (a) => ({
			path: path.join(MARKER_DIR, "agents", `${slug(a.name)}.agent.md`),
			content: emitMarkdown(
				{ name: a.name, description: a.description, tools: toolsYamlList(a.tools), model: a.model },
				a.body,
			),
		}),
		// The closest Copilot equivalent of a slash command is a prompt file.
		emitCommand: (c) => ({
			path: path.join(MARKER_DIR, "prompts", `${slug(c.name)}.prompt.md`),
			content: emitMarkdown({ description: c.description }, c.body),
		}),
	},

	detectPlugin(root: string): boolean {
		return manifestPathFor(root) !== undefined;
	},

	parsePlugin(root: string): NormalizedPlugin | null {
		const manifestPath = manifestPathFor(root);
		if (!manifestPath) return null;
		const raw = readJson<RawManifest>(manifestPath);
		if (!raw) return null;

		const id = (raw.name ?? path.basename(root)).trim();
		if (!id) return null;

		return {
			id,
			version: raw.version,
			description: raw.description,
			author: parseAuthor(raw.author),
			root,
			manifestPath,
			format: "copilot",
			// Single-format view; the registry widens this to every format present.
			supportPlatform: ["github"],
			// Claude-mirror layout (manifest overrides honored), with the legacy
			// prompts/chatmodes locations as read-only fallbacks.
			skillsDir: resolveCapabilityDir(root, raw.skills, "skills"),
			commandsDir: resolveCapabilityDir(root, raw.commands, "commands") ?? dirIfExists(root, LEGACY_PROMPTS_DIR),
			agentsDir: resolveCapabilityDir(root, raw.agents, "agents") ?? dirIfExists(root, LEGACY_CHATMODES_DIR),
			themesDir: resolveCapabilityDir(root, raw.themes, "themes"),
			// Copilot CLI plugins put hooks config at root `hooks.json`; normalizeHooks
			// covers the Claude-mirror `hooks/hooks.json`, then the legacy location.
			hooks:
				normalizeHooks(raw.hooks, root) ??
				readHooksFile(root, "hooks.json") ??
				readHooksFile(root, LEGACY_HOOKS_FILE),
			mcpServers: normalizeMcp(raw.mcpServers, root) ?? normalizeMcp(undefined, root, LEGACY_MCP_FILE),
			// Providers are a native-only concept.
			providers: undefined,
		};
	},

	emit(draft: PluginDraft): EmittedFile[] {
		// Only the manifest is Copilot-specific — the canonical location is root
		// `plugin.json` (Copilot CLI spec) — and the capability tree mirrors the
		// Claude layout, so a plugin authored for both platforms is one tree with a
		// root manifest for Copilot and a .claude-plugin/ manifest for Claude.
		const files: EmittedFile[] = [];

		files.push({
			path: emitManifestRelPath,
			content: emitJson({
				name: draft.id,
				...(draft.version ? { version: draft.version } : {}),
				...(draft.description ? { description: draft.description } : {}),
				// Spec: `author` is an object with a required `name`.
				...(draft.author ? { author: { name: draft.author } } : {}),
			}),
		});

		for (const s of draft.skills ?? []) {
			files.push({
				path: path.join("skills", slug(s.name), "SKILL.md"),
				content: emitMarkdown({ name: s.name, description: s.description }, s.body),
			});
		}
		for (const c of draft.commands ?? []) {
			files.push({
				path: path.join("commands", `${slug(c.name)}.md`),
				content: emitMarkdown({ description: c.description }, c.body),
			});
		}
		for (const a of draft.agents ?? []) {
			// Copilot recognizes custom agents by the `.agent.md` suffix, and takes
			// `tools` as a YAML list (not the Claude comma string) — matching this
			// adapter's workspace.emitAgent and the current GitHub Copilot docs
			// (docs.github.com custom agents / plugins-creating; verified 2026-07).
			files.push({
				path: path.join("agents", `${slug(a.name)}.agent.md`),
				content: emitMarkdown(
					{ name: a.name, description: a.description, tools: toolsYamlList(a.tools), model: a.model },
					a.body,
				),
			});
		}
		if (draft.mcpServers?.length) {
			files.push({
				path: ".mcp.json",
				content: emitJson({
					mcpServers: Object.fromEntries(
						draft.mcpServers.map((s) => [
							s.name,
							{ command: s.command, ...(s.args ? { args: s.args } : {}), ...(s.env ? { env: s.env } : {}) },
						]),
					),
				}),
			});
		}
		if (draft.hooks?.length) {
			files.push({
				path: path.join("hooks", "hooks.json"),
				content: emitJson({ hooks: authoredHooksToConfig(draft.hooks) }),
			});
		}
		return files;
	},
};
