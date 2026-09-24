/**
 * `/canvas` — the interactive surface for canvas extensions.
 *
 * Design: `docs/canvas-extensions-design.md` §11. Deliberately thin: every decision
 * lives in `core/canvas/session.ts`, which is testable without a terminal, so this
 * file only renders and supplies an `AbortSignal`.
 *
 * The signal is the point of the loader. `BorderedLoader` already gives Esc-to-cancel
 * and exposes an `AbortSignal`, and `registry.open` accepts one — so a person's Esc
 * reaches the abandon path (§11.6) and the extension is told to release the port it
 * may already have bound, rather than the spinner merely disappearing.
 *
 * `/new-canvas` is registered here rather than beside `/new-skill` and friends
 * because it is not a file-writing command any more: it opens what it scaffolds
 * and hands the agent a brief to build it, which needs this file's session and
 * `hoo.sendUserMessage`. Its decisions live in `core/canvas/scaffold.ts`.
 *
 * The agent tools register on the first successful open and stay for the session:
 * `registerTool` has no counterpart to remove a tool. So a session that never opens a
 * canvas pays nothing for them, which is the case that matters (§11.5); after the
 * first open they cost ~235 tokens and answer honestly when nothing is open.
 */

import { homedir } from "node:os";
import { type Component, hyperlink, truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import { getAgentDir } from "../../config.js";
import { CATEGORY_GLYPH } from "../../core/brand.js";
import { canvasDesignGuidePath } from "../../core/builtin-skills.js";

/** Canvas extensions are extensions, so they wear the extension glyph. */
const GLYPH = CATEGORY_GLYPH.extensions;

import { CanvasEventSource } from "../../core/canvas/events.js";
import { CanvasInbox } from "../../core/canvas/inbox.js";
import { isCanvasRefusal } from "../../core/canvas/lifecycle.js";
import type { CanvasInstance } from "../../core/canvas/registry.js";
import { CANVAS_HOMES, canvasBuildBrief, parseCanvasRequest, scaffoldCanvas } from "../../core/canvas/scaffold.js";
import {
	type CanvasOverview,
	CanvasSession,
	type CanvasSessionOptions,
	parseCanvasRef,
} from "../../core/canvas/session.js";
import { getWorkspacePlatforms } from "../../core/extensions/plugins/formats/platform-targets.js";
import { isWorkspaceTrusted, trustWorkspace } from "../../core/extensions/plugins/trust.js";
import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext } from "../../core/extensions/types.js";
import { taskStore } from "../../core/task-store.js";
import { createCanvasToolDefinitions } from "../../core/tools/canvas.js";
import { BorderedLoader } from "../../modes/interactive/components/bordered-loader.js";
import type { Theme } from "../../modes/interactive/theme/theme.js";
import { openUrl } from "../../utils/open-url.js";

const SUBCOMMANDS = ["list", "open", "close", "reload", "rename", "remove"] as const;

/** How an open attempt ended. `custom()` resolves with exactly one of these. */
type OpenOutcome =
	| { kind: "opened"; instance: CanvasInstance }
	| { kind: "failed"; message: string }
	| { kind: "cancelled" };

/**
 * A canvas url as something a click opens.
 *
 * The label is the url itself, so it reads the same as before and can still be
 * selected and copied; the OSC 8 wrapper is what makes the whole of it one
 * click even when the band truncates the row it sits on. Only on a terminal:
 * `--print` and RPC get the plain url, not escape codes in their text.
 */
function linkUrl(url: string, linked: boolean): string {
	return linked ? hyperlink(url, url) : url;
}

function describeInstance(instance: CanvasInstance, linked = false): string {
	const title = instance.title ?? instance.canvasId;
	return `${instance.instanceId}  ${title}${instance.url ? `  ${linkUrl(instance.url, linked)}` : ""}`;
}

/** Widget key for the pinned canvas links above the prompt. */
export const CANVAS_LINKS_WIDGET = "canvas-links";

/** Rows the pinned band will spend before it summarises the rest. */
const MAX_PINNED_ROWS = 4;

/**
 * The open canvases' urls, pinned directly above the prompt.
 *
 * The open itself sends the url to the browser, and says so on the
 * notification band — but the band fades, and a tab closed by accident a
 * minute later left nothing on screen to get it back but `/canvas list`. So
 * every open instance keeps one row here for as long as it is open: one click
 * reopens it.
 *
 * It reads the session live rather than holding a snapshot, because the url is
 * not stable: a reload (the agent's `reload_canvas` as much as `/canvas
 * reload`) binds a new port and mints a new token, and a pinned link to the
 * old one would be a link to a closed port.
 */
export class CanvasLinksBand implements Component {
	constructor(
		private readonly instances: () => readonly CanvasInstance[],
		private readonly theme: Theme,
	) {}

	invalidate(): void {}

	render(width: number): string[] {
		const pinned = this.instances().filter((instance) => instance.url);
		if (pinned.length === 0 || width < 8) return [];
		const t = this.theme;
		const hint = "click to reopen";
		const rows = pinned.slice(0, MAX_PINNED_ROWS).map((instance) => {
			const url = instance.url as string;
			const title = instance.title ?? instance.canvasId;
			const lead = ` ${t.fg("accent", GLYPH)} ${t.fg("text", title)}  `;
			const leadWidth = visibleWidth(lead);
			// The hint goes first when room runs out; the url is the row's point.
			const fitsHint = leadWidth + url.length + 2 + hint.length + 1 <= width;
			const tail = fitsHint ? `  ${t.fg("dim", hint)}` : "";
			const urlRoom = Math.max(8, width - leadWidth - 1);
			const label = t.fg("accent", t.underline(truncateToWidth(url, urlRoom)));
			return truncateToWidth(`${lead}${hyperlink(label, url)}${tail}`, width);
		});
		if (pinned.length > MAX_PINNED_ROWS) {
			rows.push(t.fg("dim", `   +${pinned.length - MAX_PINNED_ROWS} more open — /canvas list`));
		}
		return rows;
	}
}

/**
 * What an open says about its url.
 *
 * In a terminal the browser has already been sent there and the link is pinned
 * above the prompt, so this says both — and the link here is clickable too, for
 * the seconds the band is up. Elsewhere it is the plain url, because that is the
 * only way in.
 */
function openedUrlLines(url: string | undefined, interactive: boolean): string[] {
	if (!url) return [];
	if (!interactive) return [`Open in a browser: ${url}`];
	return [
		`Opened in your browser: ${linkUrl(url, true)}`,
		"Pinned above the prompt — click it to reopen a closed tab.",
	];
}

function renderOverview(overview: CanvasOverview, linked = false): string {
	const lines: string[] = [];
	if (!overview.availability.available) {
		lines.push(`Canvases are unavailable: ${overview.availability.reason}`, "");
	}
	if (overview.listings.length === 0) {
		lines.push(
			"No canvas extensions found in .agents/extensions, .github/extensions, ~/.copilot/extensions,",
			"or any installed plugin. Create one with /new-canvas <what it should do>, or install one with /plugin.",
		);
		return lines.join("\n");
	}
	for (const listing of overview.listings) {
		const name = listing.canvasId ? `${listing.extensionId}:${listing.canvasId}` : listing.extensionId;
		const label = listing.displayName ? `  ${listing.displayName}` : "";
		if (listing.withheld === "untrusted-workspace") {
			lines.push(`${GLYPH} ${name}${label}  [withheld: untrusted workspace]`);
			continue;
		}
		lines.push(`${GLYPH} ${name}${label}  (${listing.scope})`);
		for (const instance of listing.open) {
			lines.push(`    open  ${describeInstance(instance, linked)}`);
			// What a canvas can do is otherwise visible only to the model, through
			// `list_canvas_capabilities` — so the person driving the session could not
			// see the surface they were being asked about. Only for open instances,
			// because actions come from running the code (§5.1).
			const actions = overview.actionsByInstance.get(instance.instanceId) ?? [];
			if (actions.length > 0) lines.push(`          actions  ${actions.join(", ")}`);
		}
	}
	if (overview.withheldCount > 0) {
		lines.push(
			"",
			`${overview.withheldCount} extension(s) came with this repository and are withheld. Run /plugin trust to allow this directory to run code it ships.`,
		);
	}
	return lines.join("\n");
}

/**
 * Test seams, and only that.
 *
 * `/new-canvas` opens what it writes, so driving it without a terminal needs a
 * runtime that does not depend on hoocode having been built, and a home
 * directory that is not the developer's. Everything else this file does is
 * decided in `core/canvas/`, where it is testable without any of this.
 */
export interface CanvasSetupOverrides {
	homeDir?: string;
	resolveRuntime?: CanvasSessionOptions["resolveRuntime"];
	/** Stands in for the desktop browser, so a test can see what would open. */
	openUrl?: (url: string) => void;
}

export function setupCanvas(hoo: ExtensionAPI, overrides?: CanvasSetupOverrides): void {
	let session: CanvasSession | undefined;
	let toolsRegistered = false;
	/**
	 * Points at the most recent command's UI.
	 *
	 * A canvas keeps talking after the command that opened it has returned — logs,
	 * stray stdout, a leaked-port warning — so the callbacks cannot close over one
	 * invocation's `ctx`.
	 */
	let notify: (message: string, type?: "info" | "warning" | "error") => void = () => {};
	/** The UI the pinned links live on; only a terminal has one worth pinning to. */
	let pinUi: ExtensionCommandContext["ui"] | undefined;
	const browse = overrides?.openUrl ?? openUrl;

	/**
	 * Whether the agent is idle, read through the most recent context any handler
	 * or command was given — the extension API has no session-wide handle for it.
	 */
	let isIdle: () => boolean = () => true;
	const track = (ctx: ExtensionContext): void => {
		isIdle = () => ctx.isIdle();
	};

	// A canvas talking back: `session.send` lands here, and the inbox decides when
	// the agent sees it (see core/canvas/inbox.ts for why it is not simply "now").
	const inbox = new CanvasInbox({
		isIdle: () => isIdle(),
		deliver: (text, deliverAs) => {
			try {
				void Promise.resolve(hoo.sendUserMessage(text, { deliverAs })).catch((error: unknown) =>
					notify(
						`A canvas message could not be delivered: ${error instanceof Error ? error.message : String(error)}`,
						"warning",
					),
				);
			} catch (error) {
				notify(
					`A canvas message could not be delivered: ${error instanceof Error ? error.message : String(error)}`,
					"warning",
				);
			}
		},
		notify: (message) => notify(message, "info"),
	});

	// What the agent is doing, for canvases that listen (`session.on`). Emitting
	// costs nothing until a canvas is open and has subscribed.
	const events = new CanvasEventSource((event) => session?.emit(event));

	const ensureSession = (ctx: ExtensionCommandContext): CanvasSession => {
		notify = (message, type) => ctx.ui.notify(message, type);
		if (ctx.hasUI) pinUi = ctx.ui;
		session ??= new CanvasSession({
			cwd: ctx.cwd,
			homeDir: overrides?.homeDir ?? homedir(),
			agentDir: getAgentDir(),
			resolveRuntime: overrides?.resolveRuntime,
			// A canvas's own diagnostics are the user's business: a stray stdout line means
			// its author reached for console.log, and a possible leaked port is worth saying.
			onLog: (id, message, level) =>
				notify(`[canvas ${id}] ${message}`, level === "warning" || level === "error" ? level : "info"),
			onSend: (id, message) => {
				inbox.submit(id, message.prompt, message.mode);
			},
			onStray: (id, line) => notify(`[canvas ${id}] non-protocol stdout (use session.log): ${line}`, "warning"),
			onDiagnostic: (id, message) => notify(`[canvas ${id}] ${message}`, "warning"),
		});
		return session;
	};

	/**
	 * Put the pinned links up, or take them down when nothing is open.
	 *
	 * Called after everything that can change what is open. The band reads the
	 * session live, so this only has to get the widget's presence right — a
	 * reload that moves a url repaints on its own.
	 */
	const refreshPins = (): void => {
		const ui = pinUi;
		if (!ui) return;
		const open = session?.instances().some((instance) => instance.url) ?? false;
		ui.setWidget(
			CANVAS_LINKS_WIDGET,
			open ? (_tui, theme) => new CanvasLinksBand(() => session?.instances() ?? [], theme) : undefined,
		);
	};

	/**
	 * Send a freshly opened canvas to the browser.
	 *
	 * The url is the whole reason to open a canvas, and copying a tokenised
	 * localhost address out of a notification was a chore every single time. Only
	 * from a terminal: `--print` and RPC have no person at a desktop to show it to,
	 * and a browser appearing on a CI runner is nobody's intent.
	 */
	const reveal = (ctx: ExtensionCommandContext, url: string | undefined): void => {
		if (ctx.hasUI && url) browse(url);
	};

	const registerToolsOnce = (canvas: CanvasSession): void => {
		if (toolsRegistered) return;
		const registry = canvas.registryOrUndefined();
		if (!registry) return;
		toolsRegistered = true;
		for (const definition of createCanvasToolDefinitions(registry)) hoo.registerTool(definition);
	};

	/**
	 * Open a canvas behind a cancellable loader.
	 *
	 * Shared by `/canvas open` and `/new-canvas`, which want identical behaviour:
	 * opening forks a process and binds a port, so it can be slow and must be
	 * interruptible. The loader's signal is what makes Esc mean something — it
	 * reaches the registry's abandon path, which tells the extension to release a
	 * port it may already have bound. Outside a terminal (--print, RPC) there is
	 * nothing to draw and nothing to press, so the open simply runs.
	 */
	const openWithLoader = async (
		canvas: CanvasSession,
		ref: { extensionId: string; canvasId?: string },
		ctx: ExtensionCommandContext,
	): Promise<OpenOutcome | undefined> => {
		const openPlain = (): Promise<OpenOutcome> =>
			canvas
				.open(ref)
				.then((instance): OpenOutcome => ({ kind: "opened", instance }))
				.catch(
					(error: unknown): OpenOutcome => ({
						kind: "failed",
						message: error instanceof Error ? error.message : String(error),
					}),
				);
		if (!ctx.hasUI) return openPlain();
		// RPC reports a UI but cannot draw a custom component: its `custom()` settles
		// at once without calling the factory. Read that as "nothing to draw" and open
		// directly — reading it as a cancel made every RPC host's open a no-op.
		let drawn = false;
		const outcome = await ctx.ui.custom<OpenOutcome | undefined>((tui, theme, _keybindings, done) => {
			drawn = true;
			const loader = new BorderedLoader(tui, theme, `Opening ${ref.extensionId}…`);
			loader.onAbort = () => done({ kind: "cancelled" });
			void canvas
				.open(ref, { signal: loader.signal })
				.then((instance) => done({ kind: "opened", instance }))
				// Cancelling races: the signal rejects the pending call at the same moment
				// onAbort fires, and whichever lands first resolves `custom`. Deciding from
				// the signal rather than from who won means a cancel always reads as a
				// cancel instead of surfacing as an error.
				.catch((error: unknown) =>
					done(
						loader.signal.aborted
							? { kind: "cancelled" }
							: { kind: "failed", message: error instanceof Error ? error.message : String(error) },
					),
				);
			return loader;
		});
		return drawn ? outcome : openPlain();
	};

	hoo.registerCommand("canvas", {
		description:
			"Work with canvas extensions. /canvas list | open <extension>[:<canvas>] | reload [extension] | close <instanceId> | rename <extension> <new-name> | remove <extension>",
		getArgumentCompletions: (prefix: string) =>
			SUBCOMMANDS.filter((name) => name.startsWith(prefix)).map((name) => ({ value: name, label: name })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const trimmed = args.trim();
			const canvas = ensureSession(ctx);

			if (trimmed.length === 0 || trimmed === "list") {
				ctx.ui.notify(renderOverview(await canvas.list(), ctx.hasUI), "info");
				return;
			}

			if (trimmed.startsWith("close")) {
				const instanceId = trimmed.slice("close".length).trim();
				if (!instanceId) {
					ctx.ui.notify("Usage: /canvas close <instanceId>  (see /canvas list)", "warning");
					return;
				}
				const closed = await canvas.close(instanceId);
				refreshPins();
				if (!closed) ctx.ui.notify(`No open canvas instance "${instanceId}".`, "warning");
				else ctx.ui.notify(`Closed ${closed.canvasId} (${closed.instanceId}).`, "info");
				return;
			}

			if (trimmed.startsWith("reload")) {
				// A reload re-forks the extension, so it can take as long as an open and is
				// worth being able to abandon — but unlike an open there is nothing to
				// abandon *to*: the registry keeps the old child serving until the new one
				// answers, so a cancel here just stops waiting.
				const requested = trimmed.slice("reload".length).trim();
				const running = canvas.runningExtensionIds();
				const extensionId = requested || (running.length === 1 ? running[0] : undefined);
				if (!extensionId) {
					ctx.ui.notify(
						running.length === 0
							? "Nothing is open, so there is nothing to reload. Open a canvas first with /canvas open <extension>."
							: `Several extensions are open; name one: ${running.join(", ")}.`,
						"warning",
					);
					return;
				}
				try {
					const result = await canvas.reload(extensionId);
					const lines = [`Reloaded ${result.extensionId} from disk.`];
					// Same reason the tool reports it: an edit to `actions: [...]` is
					// otherwise invisible, and a typo there fails by the action simply
					// not being there.
					const { added, removed, changed } = result.actions;
					if (added.length > 0) lines.push(`  + ${added.join(", ")}`);
					if (removed.length > 0) lines.push(`  - ${removed.join(", ")}`);
					if (changed.length > 0) lines.push(`  ~ ${changed.join(", ")} (description or schema)`);
					for (const instance of result.reopened) {
						// The url is the point of saying anything: the extension binds a new
						// port and mints a new token on every open, so the tab the person has
						// in front of them is now pointing at a closed port.
						lines.push(`  ${describeInstance(instance, ctx.hasUI)}`);
						reveal(ctx, instance.url);
					}
					if (result.reopened.length > 0) {
						lines.push(
							"",
							ctx.hasUI
								? "Opened the new url(s) in your browser — the previous tab is dead."
								: "Open the new url(s) — the previous tab is dead.",
						);
					}
					for (const drop of result.dropped) {
						lines.push(`  ${drop.canvasId} (${drop.instanceId}) did not come back: ${drop.reason}`);
					}
					refreshPins();
					ctx.ui.notify(lines.join("\n"), result.dropped.length > 0 ? "warning" : "info");
				} catch (error) {
					// The registry only swaps children once the new one is ready, so the
					// canvas the person is looking at survived this. Say so, or they will
					// think they just lost it.
					ctx.ui.notify(
						`Reloading ${extensionId} failed, so it is still running the code it was started with: ` +
							`${error instanceof Error ? error.message : String(error)}`,
						"error",
					);
				}
				return;
			}

			if (trimmed.startsWith("rename")) {
				const [from, to, ...rest] = trimmed.slice("rename".length).trim().split(/\s+/).filter(Boolean);
				if (!from || !to || rest.length > 0) {
					ctx.ui.notify(`Usage: /canvas rename <extension> <new-name>  (see /canvas list)`, "warning");
					return;
				}
				// Renaming closes whatever the extension had open, because the directory
				// is about to move out from under it. Say so before doing it, not after.
				const wasOpen = canvas.instances().filter((instance) => instance.extensionId === from);
				const result = await canvas.rename(from, to);
				refreshPins();
				if (isCanvasRefusal(result)) {
					ctx.ui.notify(`/canvas rename: ${result.detail}`, "warning");
					return;
				}
				const lines = [`Renamed ${result.from} → ${result.to}.`, `  ${result.dir}`];
				if (result.rewrites.length > 0) {
					lines.push("", "Rewrote in extension.mjs:");
					for (const rewrite of result.rewrites) lines.push(`  line ${rewrite.line}: ${rewrite.after.trim()}`);
				}
				if (result.leftovers.length > 0) {
					// Prose is not identity, so it is reported rather than edited — a rename
					// that silently rewrote a description would be worse than one that
					// admits what it left.
					lines.push(
						"",
						`"${result.from}" still appears on line(s) ${result.leftovers.join(", ")}; those look like prose, so they were left alone.`,
					);
				}
				if (wasOpen.length > 0) lines.push("", `Closed ${wasOpen.length} open instance(s) to move the directory.`);
				lines.push("", `Open it with /canvas open ${result.to}.`);
				ctx.ui.notify(lines.join("\n"), "info");
				return;
			}

			if (trimmed.startsWith("remove")) {
				const target = trimmed.slice("remove".length).trim();
				if (!target) {
					ctx.ui.notify("Usage: /canvas remove <extension>  (see /canvas list)", "warning");
					return;
				}
				const known = canvas.knownExtensionIds();
				if (!known.includes(target)) {
					ctx.ui.notify(`No canvas extension "${target}" (found: ${known.join(", ") || "none"}).`, "warning");
					return;
				}
				// Deleting source is not undoable from here, so it is confirmed — and
				// outside a terminal there is nobody to ask, so it is refused rather than
				// assumed. `--print` and RPC should not be able to delete a directory
				// because a command happened to be piped in.
				if (!ctx.hasUI) {
					ctx.ui.notify(
						`/canvas remove needs to ask before deleting ${target}, and there is no interactive surface here. Delete the directory yourself, or run this in a terminal.`,
						"warning",
					);
					return;
				}
				const confirmed = await ctx.ui.confirm(
					`Delete canvas "${target}"?`,
					"This deletes the extension directory and everything in it. It is not undoable from here.",
				);
				if (!confirmed) {
					ctx.ui.notify(`Left ${target} alone.`, "info");
					return;
				}
				const openCount = canvas.instances().filter((instance) => instance.extensionId === target).length;
				const result = await canvas.remove(target);
				refreshPins();
				if (isCanvasRefusal(result)) {
					ctx.ui.notify(`/canvas remove: ${result.detail}`, "warning");
					return;
				}
				ctx.ui.notify(
					[
						`Removed ${result.id}.`,
						`  ${result.dir}`,
						openCount > 0 ? `Closed ${openCount} open instance(s) first.` : "",
					]
						.filter((line) => line.length > 0)
						.join("\n"),
					"info",
				);
				return;
			}

			if (!trimmed.startsWith("open")) {
				ctx.ui.notify(`Unknown subcommand. Use ${SUBCOMMANDS.join(", ")}.`, "warning");
				return;
			}

			const ref = parseCanvasRef(trimmed.slice("open".length));
			if (!ref) {
				ctx.ui.notify("Usage: /canvas open <extension>[:<canvas>]  (see /canvas list)", "warning");
				return;
			}

			const outcome = await openWithLoader(canvas, ref, ctx);

			// `custom` can also settle on its own when the overlay is dismissed, without
			// our `done` ever running — an escape that closes the surface leaves no
			// outcome. Treat that as the cancel it is rather than reading `.kind` off
			// undefined and failing silently.
			if (!outcome || outcome.kind === "cancelled") {
				// KNOWN ISSUE: this confirmation does not render when the cancel came from the
				// loader's own escape handling, though every effect of cancelling is correct
				// and verified (the open rejects, no instance is registered, and the extension
				// is told to close the instance it never finished opening). Ruled out: the
				// continuation does run and `ctx.ui.notify` works here — the failure path
				// through the same lines renders its error, and the canvas's own diagnostic
				// arrives moments later through this very function. Deferring a tick did not
				// help either. Left as an unexplained cosmetic gap rather than papered over
				// with a sleep; the loader disappearing is itself the signal.
				ctx.ui.notify("Canvas open cancelled.", "info");
				return;
			}
			if (outcome.kind === "failed") {
				ctx.ui.notify(outcome.message, "error");
				return;
			}

			registerToolsOnce(canvas);
			reveal(ctx, outcome.instance.url);
			refreshPins();
			ctx.ui.notify(
				[
					`Opened ${outcome.instance.canvasId} (${outcome.instance.instanceId}).`,
					...openedUrlLines(outcome.instance.url, ctx.hasUI),
					"The agent can now read and drive it; close it with /canvas close <instanceId>.",
				]
					.filter((line) => line.length > 0)
					.join("\n"),
				"info",
			);
		},
	});

	// ── /new-canvas <name> | <description> ────────────────────────────────────
	// Authoring, in Copilot's `/create-canvas` shape: describe what you want, the
	// agent writes it, and it is already open while it does. Design:
	// `docs/canvas-extensions-design.md` §9 Phase 3, §13.
	//
	// Unlike the other `/new-*` scaffolds this one needs no /reload: canvases are
	// discovered when /canvas runs, not loaded at session start.

	hoo.registerCommand("new-canvas", {
		description:
			"Create a canvas extension. Usage: /new-canvas <what it should do> | /new-canvas <name> | /new-canvas <name>: <what it should do>",
		getArgumentCompletions: () => [],
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const request = parseCanvasRequest(args);
			if (typeof request === "string") {
				ctx.ui.notify(`/new-canvas: ${request}`, "warning");
				return;
			}

			// `--platform claude` is dropped rather than redirected: Claude has no
			// canvas convention, and silently writing into someone else's marker
			// directory would put an extension where that vendor will never look.
			const requested = getWorkspacePlatforms() ?? ["agents"];
			const targets = requested.filter((platform) => CANVAS_HOMES[platform] !== undefined);
			if (targets.length === 0) {
				ctx.ui.notify(
					`/new-canvas: no canvas home for platform "${requested.join(", ")}". ` +
						"Canvas extensions exist under .agents/extensions (agents) and .github/extensions (github) only.",
					"warning",
				);
				return;
			}

			const { created, skipped } = scaffoldCanvas(ctx.cwd, request.name, targets);

			// A canvas lands in the working tree, which is where the trust gate looks
			// (`core/canvas/trust.ts`) — so without this the canvas you just asked for
			// is withheld the moment you try to open it, and refused with "came with
			// this repository", which is not true of a file created seconds ago.
			//
			// Granting is the same call `/plugin install --scope project` makes, for
			// the same stated reason: a person typing this command in this directory
			// is the human act workspace trust asks for. It is deliberately wider than
			// this one canvas — it also lets plugins already committed here run their
			// hooks and MCP servers — so it is said out loud and pointed at its
			// reverse, never done silently.
			let trustNote = "";
			if (created.length > 0 && !isWorkspaceTrusted(ctx.cwd, getAgentDir())) {
				trustWorkspace(ctx.cwd, getAgentDir());
				trustNote =
					`Trusted this workspace so the canvas can run. Plugins committed here may now run hooks ` +
					`and MCP servers too; \`/plugin untrust\` reverses it.`;
			}

			if (created.length === 0) {
				ctx.ui.notify(
					[
						"Nothing created — these already exist:",
						...skipped.map((file) => `  ${file}`),
						"",
						`Open the existing one with /canvas open ${request.name}, or pick another name.`,
					].join("\n"),
					"warning",
				);
				return;
			}

			// Open before saying anything. Copilot's `/create-canvas` puts the canvas
			// in front of the person and *then* builds it, and the order is the point:
			// a build brief that can name a live url and instance id is a different
			// instruction than one that cannot, and the person watches it change.
			const canvas = ensureSession(ctx);
			const outcome = await openWithLoader(canvas, { extensionId: request.name }, ctx);
			const opened = outcome?.kind === "opened" ? outcome.instance : undefined;
			if (opened) {
				registerToolsOnce(canvas);
				reveal(ctx, opened.url);
				refreshPins();
			}

			const lines = ["Canvas created:", ...created.map((file) => `  ${file}`)];
			if (skipped.length > 0) lines.push("Skipped (already exist):", ...skipped.map((file) => `  ${file}`));
			lines.push("");
			if (opened) {
				lines.push(`Opened ${opened.canvasId} (${opened.instanceId}).`);
				lines.push(...openedUrlLines(opened.url, ctx.hasUI));
			} else if (outcome?.kind === "failed") {
				// Not fatal: the file is written and discoverable, so say what broke and
				// leave them a way in rather than making it look like nothing happened.
				lines.push(
					`Created, but opening it failed: ${outcome.message}`,
					`Retry with /canvas open ${request.name}.`,
				);
			} else {
				lines.push(`Opening cancelled. Open it when you want with /canvas open ${request.name}.`);
			}
			if (trustNote) lines.push("", trustNote);

			if (request.description) {
				lines.push(
					"",
					`Building it now from: "${request.description}"`,
					"Steer it like any other turn, or interrupt to take over the file yourself.",
				);
			} else {
				lines.push("", `Edit ${created[0]} and run /canvas reload ${request.name} to see each change.`);
			}
			ctx.ui.notify(lines.join("\n"), "info");

			// The half that makes this `/create-canvas` rather than a scaffold. Queued
			// as a follow-up so it lands on the next turn whether or not the agent is
			// mid-stream, and only when a description was given: `/new-canvas my-board`
			// still means "give me the template", and starting a build nobody asked for
			// would burn a turn and overwrite the file they meant to edit.
			if (request.description) {
				await hoo.sendUserMessage(
					canvasBuildBrief(
						request.name,
						request.description,
						created[0] as string,
						opened ? { instanceId: opened.instanceId, url: opened.url } : undefined,
						canvasDesignGuidePath(),
					),
					{ deliverAs: "followUp" },
				);
			}
		},
	});

	// The agent's lifecycle, for canvases that listen, and for the inbox, which
	// holds a canvas's message until the agent is free.
	hoo.on("agent_start", (_event, ctx) => {
		track(ctx);
		events.turnStart();
	});
	hoo.on("agent_end", (_event, ctx) => {
		track(ctx);
		events.idle();
		inbox.agentEnded();
	});
	hoo.on("tool_execution_start", (event, ctx) => {
		track(ctx);
		events.toolStart(event.toolCallId, event.toolName, event.args);
	});
	hoo.on("tool_execution_end", (event) => {
		events.toolEnd(event.toolCallId, event.isError);
	});
	// Only the person resets the loop guard; other extensions' messages come in
	// as "extension", like the canvases' own.
	hoo.on("input", (event, ctx) => {
		track(ctx);
		if (event.source !== "extension") inbox.personSpoke();
		return { action: "continue" };
	});
	// The main agent's todos (no source: not a subagent's or an MCP call's).
	// Skipped while nothing is open, since the store changes on every subagent step.
	const stopTodos = taskStore.subscribe(() => {
		if (!session || session.instances().length === 0) return;
		events.todos(
			taskStore
				.list()
				.filter((task) => task.source === undefined)
				.map((task) => ({ id: task.id, title: task.title, status: task.status })),
		);
	});

	// Teardown on shutdown (§6): a browser tab gives no close signal, so without this
	// every child and loopback port outlives the session. `session_shutdown` is where
	// loop.ts stops its scheduler, and it is synchronous, so the dispose is fired and
	// not awaited.
	hoo.on("session_shutdown", () => {
		stopTodos();
		const closing = session;
		session = undefined;
		// Nothing is left to click through to, so nothing stays pinned. The UI may
		// already be coming down around us, and a stale band is not worth a throw.
		try {
			refreshPins();
		} catch {}
		void closing?.dispose();
	});
}
