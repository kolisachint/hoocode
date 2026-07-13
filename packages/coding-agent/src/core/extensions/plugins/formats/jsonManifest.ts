/**
 * Factory for the `plugin.json`-style formats (native `.agents-plugin` and
 * Claude Code `.claude-plugin`). They share one on-disk layout — a JSON manifest
 * under a marker directory plus `skills/`, `commands/`, `agents/`, `themes/`,
 * `hooks/hooks.json` — and differ only in the marker directory, the format id,
 * and whether `providers` are honored (native-only). Both adapters are produced
 * from this factory so the shared reader/writer lives in one place.
 */

import * as path from "node:path";
import type { NormalizedPlugin, PluginProvider } from "../manifest.js";
import {
	authoredHooksToConfig,
	emitJson,
	emitMarkdown,
	normalizeHooks,
	normalizeMcp,
	parseAuthor,
	type RawManifest,
	readJson,
	resolveCapabilityDir,
	slug,
} from "./shared.js";
import type { EmittedFile, PluginDraft, PluginFormatAdapter, PluginFormatId } from "./types.js";

const MANIFEST_FILE = "plugin.json";

interface JsonManifestOptions {
	id: Extract<PluginFormatId, "agents" | "claude">;
	/** Marker subdirectory, e.g. ".agents-plugin". */
	manifestDir: string;
	precedence: number;
	label: string;
	/** Native honors `providers`; the Claude-compat path ignores them. */
	supportsProviders: boolean;
}

export function createJsonManifestAdapter(opts: JsonManifestOptions): PluginFormatAdapter {
	const manifestRelPath = path.join(opts.manifestDir, MANIFEST_FILE);

	return {
		id: opts.id,
		platform: opts.id, // "agents" | "claude" map 1:1 to their platform token
		precedence: opts.precedence,
		label: opts.label,
		marketplaceFiles: [path.join(opts.manifestDir, "marketplace.json")],

		detectPlugin(root: string): boolean {
			return readJson(path.join(root, manifestRelPath)) != null;
		},

		parsePlugin(root: string): NormalizedPlugin | null {
			const manifestPath = path.join(root, manifestRelPath);
			const raw = readJson<RawManifest>(manifestPath);
			if (!raw) return null;

			const id = (raw.name ?? path.basename(root)).trim();
			if (!id) return null;

			const providers: PluginProvider[] | undefined =
				opts.supportsProviders && Array.isArray(raw.providers) ? (raw.providers as PluginProvider[]) : undefined;

			return {
				id,
				version: raw.version,
				description: raw.description,
				author: parseAuthor(raw.author),
				root,
				manifestPath,
				format: opts.id,
				// Single-format view; the registry widens this to every format present.
				supportPlatform: [opts.id],
				skillsDir: resolveCapabilityDir(root, raw.skills, "skills"),
				commandsDir: resolveCapabilityDir(root, raw.commands, "commands"),
				agentsDir: resolveCapabilityDir(root, raw.agents, "agents"),
				themesDir: resolveCapabilityDir(root, raw.themes, "themes"),
				hooks: normalizeHooks(raw.hooks, root),
				mcpServers: normalizeMcp(raw.mcpServers, root),
				providers,
			};
		},

		emit(draft: PluginDraft): EmittedFile[] {
			const files: EmittedFile[] = [];
			const mcpServers = draft.mcpServers?.length
				? Object.fromEntries(
						draft.mcpServers.map((s) => [
							s.name,
							{ command: s.command, ...(s.args ? { args: s.args } : {}), ...(s.env ? { env: s.env } : {}) },
						]),
					)
				: undefined;

			files.push({
				path: manifestRelPath,
				content: emitJson({
					name: draft.id,
					...(draft.version ? { version: draft.version } : {}),
					...(draft.description ? { description: draft.description } : {}),
					...(draft.author ? { author: draft.author } : {}),
					...(mcpServers ? { mcpServers } : {}),
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
				files.push({
					path: path.join("agents", `${slug(a.name)}.md`),
					content: emitMarkdown(
						{ name: a.name, description: a.description, tools: a.tools, model: a.model },
						a.body,
					),
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
}
