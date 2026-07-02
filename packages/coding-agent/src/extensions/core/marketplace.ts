/**
 * /plugin — marketplace add/list + plugin install/list/remove.
 *
 * `/plugin` installs plugins from marketplaces (a git repo or local dir with a
 * Claude `.claude-plugin/marketplace.json` or Copilot-style `.github/marketplace.json`
 * index). Installed plugins are placed in `.hoocode/plugins/<name>` and loaded by the
 * plugin loader after a reload.
 *
 *   /plugin marketplace add <git-url|path>
 *   /plugin marketplace list
 *   /plugin list                     list available plugins across marketplaces
 *   /plugin install <name>
 *   /plugin remove <name>
 */

import { cpSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
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
	const storePath = (cwd: string) => join(cwd, ".hoocode", "marketplaces.json");
	const pluginsDir = (cwd: string) => join(cwd, ".hoocode", "plugins");
	const cacheDir = (cwd: string) => join(cwd, ".hoocode", "marketplace-cache");

	const findPlugin = (cwd: string, name: string) => {
		for (const record of readMarketplaceStore(storePath(cwd))) {
			const market = parseMarketplaceDir(record.dir);
			const entry = market?.plugins.find((p) => p.name === name);
			if (market && entry) return { market, entry };
		}
		return undefined;
	};

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
					const records = readMarketplaceStore(storePath(cwd));
					if (records.length === 0) {
						ctx.ui.notify("No marketplaces. Add one with /plugin marketplace add <git-url|path>.", "info");
						return;
					}
					const lines = records.map((r) => {
						const market = parseMarketplaceDir(r.dir);
						return `${market?.name ?? r.location} — ${market?.plugins.length ?? 0} plugin(s) [${r.location}]`;
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
							"No marketplace manifest found (.claude-plugin/ or .github/marketplace.json).",
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
				const records = readMarketplaceStore(storePath(cwd));
				const lines: string[] = [];
				for (const record of records) {
					const market = parseMarketplaceDir(record.dir);
					for (const p of market?.plugins ?? []) {
						lines.push(`${p.name} — ${p.description ?? p.source}`);
					}
				}
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
				const found = findPlugin(cwd, name);
				if (!found) {
					ctx.ui.notify(`Plugin "${name}" not found in any marketplace.`, "error");
					return;
				}
				const resolved = resolvePluginSource(found.entry.source, found.market.root);
				const dest = join(pluginsDir(cwd), sanitizeForDir(name));
				rmSync(dest, { recursive: true, force: true });
				mkdirSync(pluginsDir(cwd), { recursive: true });

				if (resolved.kind === "local") {
					cpSync(resolved.path, dest, { recursive: true });
				} else if (resolved.kind === "git") {
					const res = await pi.exec("git", ["clone", "--depth", "1", resolved.url, dest]);
					if (res.code !== 0) {
						ctx.ui.notify(`Clone failed: ${res.stderr || res.stdout}`, "error");
						return;
					}
				} else {
					ctx.ui.notify(`npm plugin sources are not supported yet (${resolved.spec}).`, "warning");
					return;
				}

				ctx.ui.notify(`Installed "${name}" — reloading…`, "info");
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
				const dest = join(pluginsDir(cwd), sanitizeForDir(name));
				if (!existsSync(dest)) {
					ctx.ui.notify(`Plugin "${name}" is not installed.`, "info");
					return;
				}
				rmSync(dest, { recursive: true, force: true });
				ctx.ui.notify(`Removed "${name}" — reloading…`, "info");
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
