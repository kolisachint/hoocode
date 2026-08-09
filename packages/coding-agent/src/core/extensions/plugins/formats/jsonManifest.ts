/**
 * Factory for the `plugin.json`-style formats (native `.agents-plugin` and
 * Claude Code `.claude-plugin`). They share one on-disk layout — a JSON manifest
 * under a marker directory plus `skills/`, `commands/`, `agents/`, `themes/`,
 * `hooks/hooks.json` — and differ only in the marker directory, the format id,
 * and whether `providers` are honored (native-only). Both adapters are produced
 * from this factory so the shared reader/writer lives in one place.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type { NormalizedPlugin, PluginProvider } from "../manifest.js";
import {
	authoredHooksToConfig,
	claudeStyleWorkspace,
	detectUnsupportedSurfaces,
	emitJson,
	emitMarkdown,
	normalizeHooks,
	normalizeMcp,
	parseAuthor,
	type RawManifest,
	readJson,
	resolveCapabilityDir,
	slug,
	unknownManifestFields,
} from "./shared.js";
import type { EmittedFile, PluginDraft, PluginFormatAdapter, PluginFormatId } from "./types.js";

const MANIFEST_FILE = "plugin.json";

/** Component locations that make a directory a plugin even with no manifest. */
export const CONVENTIONAL_COMPONENTS = ["skills", "commands", "agents", "hooks", ".mcp.json", "SKILL.md"];

/**
 * Everything this adapter resolves, including surfaces that are read from the
 * manifest rather than auto-discovered. Distinct from
 * {@link CONVENTIONAL_COMPONENTS}, which drives *detection* — `themes` is parsed
 * but has never made a directory a plugin on its own.
 */
export const JSON_MANIFEST_READ_PATHS = [...CONVENTIONAL_COMPONENTS, "themes", "hooks/hooks.json"];

/** True when `root` carries any component in its default location. */
function hasConventionalComponents(root: string): boolean {
	return CONVENTIONAL_COMPONENTS.some((rel) => fs.existsSync(path.join(root, rel)));
}

interface JsonManifestOptions {
	id: Extract<PluginFormatId, "agents" | "claude">;
	/**
	 * Honor the vendor's manifest-optional rule. True for Claude, whose reference
	 * states the manifest is optional for marketplace and `--plugin-dir` plugins.
	 */
	allowManifestless?: boolean;
	/** Marker subdirectory, e.g. ".agents-plugin". */
	manifestDir: string;
	/** Workspace-level artifact root, e.g. ".agents" / ".claude" (skills/, agents/, commands/ live under it). */
	workspaceRoot: string;
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
		workspace: claudeStyleWorkspace(opts.workspaceRoot),

		hasManifest(root: string): boolean {
			return readJson(path.join(root, manifestRelPath)) != null;
		},

		detectPlugin(root: string): boolean {
			if (this.hasManifest(root)) return true;
			// Manifest-optional: a directory whose components sit in the default
			// locations is a plugin, with the name derived from the directory.
			//
			// Deliberately NOT applied inside a skills directory: there the manifest
			// is precisely what promotes a folder from plain skill to plugin, so
			// `discoverPlugins` passes `requireManifest` for those roots. Two vendor
			// rules that look contradictory until you notice they describe different
			// containers. See docs/plugin-system-architecture.md §1.6 and §2.1.
			return opts.allowManifestless === true && hasConventionalComponents(root);
		},

		parsePlugin(root: string): NormalizedPlugin | null {
			const manifestPath = path.join(root, manifestRelPath);
			const raw = readJson<RawManifest>(manifestPath) ?? (this.detectPlugin(root) ? {} : null);
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
				// A plugin shipping exactly one skill may put SKILL.md at its root
				// instead of creating skills/ (Claude reference, "Skills").
				skillsDir:
					resolveCapabilityDir(root, raw.skills, "skills") ??
					(fs.existsSync(path.join(root, "SKILL.md")) ? root : undefined),
				commandsDir: resolveCapabilityDir(root, raw.commands, "commands"),
				agentsDir: resolveCapabilityDir(root, raw.agents, "agents"),
				themesDir: resolveCapabilityDir(root, raw.themes, "themes"),
				hooks: normalizeHooks(raw.hooks, root),
				mcpServers: normalizeMcp(raw.mcpServers, root),
				providers,
				unknownFields: unknownManifestFields(raw),
				unsupportedSurfaces: detectUnsupportedSurfaces(root),
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
					...(draft.unknownFields ?? {}),
					name: draft.id,
					...(draft.version ? { version: draft.version } : {}),
					...(draft.description ? { description: draft.description } : {}),
					// Claude Code's schema documents `author` as an object with `name`.
					...(draft.author ? { author: { name: draft.author } } : {}),
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
