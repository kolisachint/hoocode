/**
 * /plugin — marketplace add/list + plugin install/list/remove (the human path).
 *
 * `/plugin` installs plugins from marketplaces (a git repo or local dir with a
 * native `.agents-plugin/marketplace.json`, Claude `.claude-plugin/marketplace.json`,
 * or Copilot-style `.github/marketplace.json` index). Installed plugins are placed in
 * `.agents/plugins/<name>` (the primary, cross-vendor home) and loaded by the plugin
 * loader after a reload.
 *
 * Adding a marketplace is the human trust boundary and stays here; the shared
 * mechanics (discovery, install, remove, the bundled default marketplace) live in
 * `core/extensions/plugins/install.ts` so this command and the model-facing
 * lifecycle tools never drift.
 *
 *   /plugin marketplace add <git-url|path>
 *   /plugin marketplace list
 *   /plugin list                     list available plugins across marketplaces
 *   /plugin install <name>
 *   /plugin remove <name>
 */

import { existsSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import {
	findAvailablePlugin,
	installAvailablePlugin,
	listAvailablePlugins,
	readMarketplaceRecords,
	uninstallPlugin,
} from "../../core/extensions/plugins/install.js";
import {
	parseMarketplaceDir,
	readMarketplaceStore,
	resolvePluginSource,
	writeMarketplaceStore,
} from "../../core/extensions/plugins/marketplace.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";

function sanitizeForDir(s: string): string {
	return s.replace(/[^a-zA-Z0-9._-]+/g, "_").slice(0, 80);
}

function isGitSource(loc: string): boolean {
	return /^https?:\/\//.test(loc) || loc.startsWith("git@") || loc.endsWith(".git");
}

export function setupMarketplace(pi: ExtensionAPI): void {
	// `.agents/` is the primary, cross-vendor home for the added-marketplace registry.
	const storePath = (cwd: string) => join(cwd, ".agents", "marketplaces.json");
	const cacheDir = (cwd: string) => join(cwd, ".agents", "marketplace-cache");

	pi.registerCommand("plugin", {
		description:
			"Manage plugin marketplaces. /plugin marketplace add <git-url|path> | /plugin marketplace list | /plugin list | /plugin install <name> | /plugin remove <name>",
		getArgumentCompletions: (prefix: string) =>
			["marketplace", "list", "install", "remove"]
				.filter((s) => s.startsWith(prefix))
				.map((s) => ({ value: s, label: s })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const trimmed = args.trim();
			const cwd = ctx.cwd;

			// ── marketplace add / list ──────────────────────────────────────────
			if (trimmed.startsWith("marketplace")) {
				const sub = trimmed.slice("marketplace".length).trim();

				if (sub === "list" || sub === "") {
					const records = readMarketplaceRecords(cwd);
					if (records.length === 0) {
						ctx.ui.notify("No marketplaces. Add one with /plugin marketplace add <git-url|path>.", "info");
						return;
					}
					const lines = records.map((r) => {
						const market = parseMarketplaceDir(r.dir);
						const platforms =
							market && market.supportPlatform.length > 1 ? ` · ${market.supportPlatform.join(", ")}` : "";
						return `${market?.name ?? r.location} — ${market?.plugins.length ?? 0} plugin(s)${platforms} [${r.location}]`;
					});
					ctx.ui.notify(lines.join("\n"), "info");
					return;
				}

				if (sub.startsWith("add")) {
					const loc = sub.slice("add".length).trim();
					if (!loc) {
						ctx.ui.notify("Usage: /plugin marketplace add <git-url|path>", "warning");
						return;
					}

					let dir: string;
					if (isGitSource(loc)) {
						dir = join(cacheDir(cwd), sanitizeForDir(loc));
						rmSync(dir, { recursive: true, force: true });
						mkdirSync(cacheDir(cwd), { recursive: true });
						const res = await pi.exec("git", ["clone", "--depth", "1", loc, dir]);
						if (res.code !== 0) {
							ctx.ui.notify(`Clone failed: ${res.stderr || res.stdout}`, "error");
							return;
						}
					} else {
						dir = resolvePluginSource(loc, cwd).kind === "local" ? join(cwd, loc) : loc;
						if (!existsSync(dir)) {
							ctx.ui.notify(`Path not found: ${dir}`, "error");
							return;
						}
					}

					const market = parseMarketplaceDir(dir);
					if (!market) {
						ctx.ui.notify(
							"No marketplace manifest found (.agents-plugin/, .claude-plugin/, or .github/marketplace.json).",
							"error",
						);
						return;
					}

					const records = readMarketplaceStore(storePath(cwd)).filter((r) => r.location !== loc);
					records.push({ location: loc, dir });
					writeMarketplaceStore(storePath(cwd), records);
					ctx.ui.notify(`Added marketplace "${market.name}" (${market.plugins.length} plugin(s)).`, "info");
					return;
				}

				ctx.ui.notify("Usage: /plugin marketplace add <git-url|path> | /plugin marketplace list", "warning");
				return;
			}

			// ── list available plugins ──────────────────────────────────────────
			if (trimmed === "list" || trimmed === "") {
				const available = listAvailablePlugins(cwd);
				const lines = available.map(
					(p) => `${p.name} [${p.supportPlatform.join(", ")}] — ${p.description ?? p.source}`,
				);
				ctx.ui.notify(lines.length ? lines.join("\n") : "No plugins available. Add a marketplace first.", "info");
				return;
			}

			// ── install <name> ──────────────────────────────────────────────────
			if (trimmed.startsWith("install")) {
				const name = trimmed.slice("install".length).trim();
				if (!name) {
					ctx.ui.notify("Usage: /plugin install <name>", "warning");
					return;
				}
				if (!findAvailablePlugin(cwd, name)) {
					ctx.ui.notify(`Plugin "${name}" not found in any marketplace.`, "error");
					return;
				}
				const outcome = await installAvailablePlugin(cwd, name);
				if (!outcome.installed) {
					ctx.ui.notify(outcome.message, "error");
					return;
				}
				ctx.ui.notify(`${outcome.message} Reloading…`, "info");
				await ctx.reload();
				return;
			}

			// ── remove <name> ───────────────────────────────────────────────────
			if (trimmed.startsWith("remove")) {
				const name = trimmed.slice("remove".length).trim();
				if (!name) {
					ctx.ui.notify("Usage: /plugin remove <name>", "warning");
					return;
				}
				const outcome = uninstallPlugin(cwd, name);
				if (!outcome.removed) {
					ctx.ui.notify(outcome.message, "info");
					return;
				}
				ctx.ui.notify(`${outcome.message} Reloading…`, "info");
				await ctx.reload();
				return;
			}

			ctx.ui.notify(
				"Usage: /plugin marketplace add|list | /plugin list | /plugin install <name> | /plugin remove <name>",
				"warning",
			);
		},
	});
}
