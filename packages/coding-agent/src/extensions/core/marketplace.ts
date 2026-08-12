/**
 * /plugin — marketplace add/list + plugin install/list/remove (the human path).
 *
 * `/plugin` installs plugins from marketplaces (a git repo or local dir with a
 * native `.agents-plugin/marketplace.json`, Claude `.claude-plugin/marketplace.json`,
 * or Copilot-style `.github/marketplace.json` index). Installed plugins are placed
 * in the global consumption home (`~/.agents/plugins/<name>`) and loaded by the
 * plugin loader after a reload.
 *
 * Adding a marketplace is the human trust boundary and stays here; the shared
 * mechanics (discovery, install, remove, the bundled default marketplace) live in
 * `core/extensions/plugins/install.ts` so this command and the model-facing
 * lifecycle tools never drift.
 *
 *   /plugin marketplace add <git-url|path>
 *   /plugin marketplace list
 *   /plugin marketplace refresh     re-fetch every cached index now
 *   /plugin list                     list available plugins across marketplaces
 *   /plugin install <name>
 *   /plugin remove <name>
 *   /plugin publish <name> [--to <marketplace-dir>]
 *
 * `publish` is the human end of the publish lane (§3.2). The model can package a
 * plugin — gates, README, index entry — but never publish it, because an agent
 * that can push executable code into a marketplace other agents install from
 * unattended is a supply-chain compromise primitive. So the last step is a
 * command a person types, and even then it stops at the machine boundary: with
 * `--to` it copies the plugin into a local marketplace checkout and updates that
 * index, leaving an uncommitted diff to review. Committing and opening the PR
 * stay manual.
 */

import { existsSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { CATEGORY_GLYPH, SEGMENT_SEP } from "../../core/brand.js";
import { getPlugin } from "../../core/extensions/plugins/authoring.js";
import {
	findAvailablePlugin,
	installAvailablePlugin,
	listAvailablePlugins,
	listInstalledPlugins,
	readMarketplaceRecords,
	refreshMarketplaces,
	uninstallPlugin,
} from "../../core/extensions/plugins/install.js";
import { availablePluginGroups } from "../../core/extensions/plugins/listing.js";
import {
	marketplaceCacheDir,
	marketplaceCacheRoot,
	marketplaceStorePath,
} from "../../core/extensions/plugins/locations.js";
import {
	parseMarketplaceDir,
	readMarketplaceStore,
	resolvePluginSource,
	writeMarketplaceStore,
} from "../../core/extensions/plugins/marketplace.js";
import { entryNeedsSource, packagePlugin, stageIntoMarketplace } from "../../core/extensions/plugins/packaging.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";
import { type ListStyle, plural, renderList } from "../../core/format-list.js";

function isGitSource(loc: string): boolean {
	return /^https?:\/\//.test(loc) || loc.startsWith("git@") || loc.endsWith(".git");
}

/**
 * Chat styling for a listing.
 *
 * The name stays in the terminal's default foreground and everything else steps
 * down from it, so the column you scan is the brightest thing on the row. Note
 * this only reaches the screen because `showStatus` passes pre-styled messages
 * through instead of wrapping them in a blanket dim.
 */
function listStyle(theme: ExtensionCommandContext["ui"]["theme"]): ListStyle {
	return {
		marker: (text) => theme.fg("success", text),
		facts: (text) => theme.fg("muted", text),
		detail: (text) => theme.fg("dim", text),
		trailer: (text) => theme.fg("dim", text),
		groupTitle: (text) => theme.fg("mdLink", text),
	};
}

/** `⬡ 4 plugins …` — the counted header every listing opens with. */
function listHeader(
	theme: ExtensionCommandContext["ui"]["theme"],
	glyph: string,
	summary: string,
	hint?: string,
): string {
	const head = `${theme.fg("accent", glyph)} ${theme.fg("muted", summary)}`;
	return hint ? `${head}\n${theme.fg("dim", `  ${hint}`)}` : head;
}

export function setupMarketplace(pi: ExtensionAPI): void {
	pi.registerCommand("plugin", {
		description:
			"Manage plugin marketplaces. /plugin marketplace add <git-url|path> | /plugin marketplace list | /plugin marketplace refresh | /plugin list | /plugin install <name> | /plugin remove <name> | /plugin publish <name> [--to <dir>]",
		getArgumentCompletions: (prefix: string) =>
			["marketplace", "list", "install", "remove", "refresh", "publish"]
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
					const theme = ctx.ui.theme;
					const rows = records.map((r) => {
						const market = parseMarketplaceDir(r.dir);
						return {
							name: market?.name ?? r.location,
							facts: [plural(market?.plugins.length ?? 0, "plugin"), (market?.supportPlatform ?? []).join(", ")],
							// The location is the longest and least-scanned token on the row;
							// on its own line it stops forcing the wrap.
							trailer: r.location,
						};
					});
					const body = renderList([{ rows }], {
						columns: ctx.ui.columns,
						indent: 2,
						style: listStyle(theme),
					});
					const header = listHeader(theme, CATEGORY_GLYPH.marketplaces, plural(records.length, "marketplace"));
					ctx.ui.notify(
						`${header}\n${body}\n\n${theme.fg("dim", "  /plugin list to see what they offer")}`,
						"info",
					);
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
						dir = marketplaceCacheDir(loc);
						rmSync(dir, { recursive: true, force: true });
						mkdirSync(marketplaceCacheRoot(), { recursive: true });
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

					const records = readMarketplaceStore(marketplaceStorePath()).filter((r) => r.location !== loc);
					records.push({ location: loc, dir });
					writeMarketplaceStore(marketplaceStorePath(), records);
					ctx.ui.notify(`Added marketplace "${market.name}" (${market.plugins.length} plugin(s)).`, "info");
					return;
				}

				if (sub === "refresh") {
					ctx.ui.notify("Refreshing marketplace indices…", "info");
					const { refreshed, errors } = await refreshMarketplaces(cwd);
					const lines: string[] = [];
					if (refreshed.length > 0) lines.push(`Refreshed: ${refreshed.join(", ")}`);
					if (errors.length > 0) lines.push(`Could not refresh: ${errors.join("; ")}`);
					ctx.ui.notify(lines.join("\n") || "Nothing to refresh.", errors.length > 0 ? "warning" : "info");
					return;
				}

				ctx.ui.notify(
					"Usage: /plugin marketplace add <git-url|path> | /plugin marketplace list | /plugin marketplace refresh",
					"warning",
				);
				return;
			}

			// ── list available plugins ──────────────────────────────────────────
			if (trimmed === "list" || trimmed === "") {
				const available = listAvailablePlugins(cwd);
				if (available.length === 0) {
					ctx.ui.notify("No plugins available. Add a marketplace first.", "info");
					return;
				}
				const theme = ctx.ui.theme;
				// Installed state is the fact the old listing could not answer: without
				// it, /plugin install on something already present silently re-clones
				// over it, discarding any local edits to the plugin directory.
				const installed = new Set(listInstalledPlugins(cwd).map((p) => p.id));
				const groups = availablePluginGroups(available, { installed });
				const body = renderList(groups, {
					columns: ctx.ui.columns,
					indent: 2,
					style: listStyle(theme),
				});
				const installedCount = available.filter((p) => installed.has(p.name)).length;
				const summary =
					`${plural(available.length, "plugin")} across ${plural(groups.length, "marketplace")}` +
					(installedCount > 0 ? ` ${SEGMENT_SEP} ${installedCount} installed` : "");
				const header = listHeader(
					theme,
					CATEGORY_GLYPH.plugins,
					summary,
					installedCount > 0 ? "✓ already installed" : undefined,
				);
				ctx.ui.notify(`${header}\n${body}\n\n${theme.fg("dim", "  /plugin install <name> to add one")}`, "info");
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

			// ── publish <name> [--to <marketplace-dir>] ─────────────────────────
			if (trimmed.startsWith("publish")) {
				const rest = trimmed.slice("publish".length).trim();
				const toMatch = /(?:^|\s)--to\s+(\S+)/.exec(rest);
				const name = rest.replace(/(?:^|\s)--to\s+\S+/, "").trim();
				if (!name) {
					ctx.ui.notify("Usage: /plugin publish <name> [--to <marketplace-dir>]", "warning");
					return;
				}
				const plugin = getPlugin(cwd, name);
				if (!plugin) {
					ctx.ui.notify(`No plugin named "${name}" is installed or authored here.`, "error");
					return;
				}

				ctx.ui.notify(`Packaging "${name}" — running publish checks…`, "info");
				const result = await packagePlugin(plugin);
				if (!result.ok) {
					// A red gate blocks staging too: the point of the strict run is that a
					// plugin other people install has passed it, and staging is the step
					// that puts it on the path to them.
					ctx.ui.notify(`Not publishable yet.\n${result.instructions}`, "error");
					return;
				}

				if (!toMatch) {
					const hint = entryNeedsSource(result.entry)
						? "\nRe-run with --to <marketplace-dir> to vendor it into a local marketplace checkout (which fills `source` in)."
						: "";
					ctx.ui.notify(`${result.instructions}${hint}`, "info");
					return;
				}

				const staged = stageIntoMarketplace(plugin, result.platform, toMatch[1]);
				if (!staged.ok) {
					ctx.ui.notify(staged.message, "error");
					return;
				}
				ctx.ui.notify(
					`${staged.message}\n\nReview the diff, then commit and open the pull request yourself — ` +
						"hoocode deliberately stops short of pushing a plugin into a marketplace.",
					"info",
				);
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
				"Usage: /plugin marketplace add|list|refresh | /plugin list | /plugin install <name> | " +
					"/plugin remove <name> | /plugin publish <name> [--to <marketplace-dir>]",
				"warning",
			);
		},
	});
}
