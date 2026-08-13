/**
 * /plugin — marketplace add/list + plugin install/list/remove (the human path).
 *
 * `/plugin` installs plugins from marketplaces (a git repo or local dir with a
 * native `.agents-plugin/marketplace.json`, Claude `.claude-plugin/marketplace.json`,
 * or Copilot-style `.github/marketplace.json` index). Installed plugins are placed
 * at the chosen scope — `~/.agents/plugins/<name>` for `user`, or
 * `<cwd>/.agents/plugins/<name>` for `project` — and loaded by the plugin loader
 * after a reload. `install` asks which, since a human is right here to answer;
 * the autonomous tool reads the `pluginInstallScope` setting instead.
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
 *   /plugin install <name> [--scope user|project]
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
import { getAgentDir } from "../../config.js";
import { getPlugin } from "../../core/extensions/plugins/authoring.js";
import {
	findAvailablePlugin,
	installAvailablePlugin,
	listAvailablePlugins,
	readMarketplaceRecords,
	refreshMarketplaces,
	uninstallPlugin,
} from "../../core/extensions/plugins/install.js";
import {
	marketplaceCacheDir,
	marketplaceCacheRoot,
	marketplaceStorePath,
	type PluginInstallScope,
} from "../../core/extensions/plugins/locations.js";
import {
	parseMarketplaceDir,
	readMarketplaceStore,
	resolvePluginSource,
	writeMarketplaceStore,
} from "../../core/extensions/plugins/marketplace.js";
import { entryNeedsSource, packagePlugin, stageIntoMarketplace } from "../../core/extensions/plugins/packaging.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";
import { SettingsManager } from "../../core/settings-manager.js";

function isGitSource(loc: string): boolean {
	return /^https?:\/\//.test(loc) || loc.startsWith("git@") || loc.endsWith(".git");
}

function isInstallScope(value: string): value is PluginInstallScope {
	return value === "user" || value === "project";
}

/** Labels carry the consequence, since that is the whole content of the choice. */
const SCOPE_CHOICES: ReadonlyArray<{ scope: PluginInstallScope; label: string }> = [
	{ scope: "user", label: "User — ~/.agents/plugins, available in every project (recommended)" },
	{ scope: "project", label: "Project — <repo>/.agents/plugins, committed and shared with collaborators" },
];

/**
 * Ask where the plugin should land. Returns undefined when the user dismisses
 * the selector, which cancels the install rather than guessing.
 *
 * Headless (no UI to ask through) resolves to `fallback` — the standing setting
 * — so a scripted or `--print` run still installs somewhere predictable.
 */
async function promptForScope(
	ctx: ExtensionCommandContext,
	fallback: PluginInstallScope,
): Promise<PluginInstallScope | undefined> {
	if (!ctx.hasUI) return fallback;
	const labels = SCOPE_CHOICES.map((c) => c.label);
	const picked = await ctx.ui.select("Install scope", labels);
	if (picked === undefined) return undefined;
	return SCOPE_CHOICES.find((c) => c.label === picked)?.scope ?? fallback;
}

export function setupMarketplace(pi: ExtensionAPI): void {
	pi.registerCommand("plugin", {
		description:
			"Manage plugin marketplaces. /plugin marketplace add <git-url|path> | /plugin marketplace list | /plugin marketplace refresh | /plugin list | /plugin install <name> [--scope user|project] | /plugin remove <name> | /plugin publish <name> [--to <dir>]",
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
				const lines = available.map((p) => {
					const src =
						typeof p.source === "string"
							? p.source
							: p.source.source === "url"
								? p.source.url
								: `${p.source.url}/${p.source.path}`;
					return `${p.name} [${p.supportPlatform.join(", ")}] — ${p.description ?? src}`;
				});
				ctx.ui.notify(lines.length ? lines.join("\n") : "No plugins available. Add a marketplace first.", "info");
				return;
			}

			// ── install <name> [--scope user|project] ───────────────────────────
			if (trimmed.startsWith("install")) {
				const rest = trimmed.slice("install".length).trim();
				const scopeMatch = /(?:^|\s)--scope[= ]\s*(\S+)/.exec(rest);
				const name = rest.replace(/(?:^|\s)--scope[= ]\s*\S+/, "").trim();
				if (!name) {
					ctx.ui.notify("Usage: /plugin install <name> [--scope user|project]", "warning");
					return;
				}
				if (scopeMatch && !isInstallScope(scopeMatch[1])) {
					ctx.ui.notify(`Unknown scope "${scopeMatch[1]}". Use --scope user or --scope project.`, "warning");
					return;
				}
				if (!findAvailablePlugin(cwd, name)) {
					ctx.ui.notify(`Plugin "${name}" not found in any marketplace.`, "error");
					return;
				}

				// An explicit --scope wins; otherwise ask, because this is the human
				// path and the destination is a real choice (portable vs. committed to
				// the repo). With no UI to ask through there is nobody to ask, so it
				// falls back to the same setting the autonomous path uses.
				const scope = scopeMatch
					? (scopeMatch[1] as PluginInstallScope)
					: await promptForScope(ctx, SettingsManager.create(cwd, getAgentDir()).getPluginInstallScope());
				if (!scope) {
					ctx.ui.notify("Install cancelled.", "info");
					return;
				}

				const outcome = await installAvailablePlugin(cwd, name, getAgentDir(), { scope });
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
				"Usage: /plugin marketplace add|list|refresh | /plugin list | /plugin install <name> [--scope user|project] | " +
					"/plugin remove <name> | /plugin publish <name> [--to <marketplace-dir>]",
				"warning",
			);
		},
	});
}
