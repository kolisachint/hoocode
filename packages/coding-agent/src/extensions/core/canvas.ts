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
 * The two agent tools register on the first successful open and stay for the session:
 * `registerTool` has no counterpart to remove a tool. So a session that never opens a
 * canvas pays nothing for them, which is the case that matters (§11.5); after the
 * first open they cost ~235 tokens and answer honestly when nothing is open.
 */

import { homedir } from "node:os";
import { getAgentDir } from "../../config.js";
import { CATEGORY_GLYPH } from "../../core/brand.js";

/** Canvas extensions are extensions, so they wear the extension glyph. */
const GLYPH = CATEGORY_GLYPH.extensions;

import type { CanvasInstance } from "../../core/canvas/registry.js";
import { type CanvasOverview, CanvasSession, parseCanvasRef } from "../../core/canvas/session.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";
import { createCanvasToolDefinitions } from "../../core/tools/canvas.js";
import { BorderedLoader } from "../../modes/interactive/components/bordered-loader.js";

const SUBCOMMANDS = ["list", "open", "close"] as const;

/** How an open attempt ended. `custom()` resolves with exactly one of these. */
type OpenOutcome =
	| { kind: "opened"; instance: CanvasInstance }
	| { kind: "failed"; message: string }
	| { kind: "cancelled" };

function describeInstance(instance: CanvasInstance): string {
	const title = instance.title ?? instance.canvasId;
	return `${instance.instanceId}  ${title}${instance.url ? `  ${instance.url}` : ""}`;
}

function renderOverview(overview: CanvasOverview): string {
	const lines: string[] = [];
	if (!overview.availability.available) {
		lines.push(`Canvases are unavailable: ${overview.availability.reason}`, "");
	}
	if (overview.listings.length === 0) {
		lines.push(
			"No canvas extensions found in .agents/extensions, .github/extensions, ~/.copilot/extensions,",
			"or any installed plugin. Scaffold one with /create-canvas <name>, or install one with /plugin.",
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
		for (const instance of listing.open) lines.push(`    open  ${describeInstance(instance)}`);
	}
	if (overview.withheldCount > 0) {
		lines.push(
			"",
			`${overview.withheldCount} extension(s) came with this repository and are withheld. Run /plugin trust to allow this directory to run code it ships.`,
		);
	}
	return lines.join("\n");
}

export function setupCanvas(pi: ExtensionAPI): void {
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

	const ensureSession = (ctx: ExtensionCommandContext): CanvasSession => {
		notify = (message, type) => ctx.ui.notify(message, type);
		session ??= new CanvasSession({
			cwd: ctx.cwd,
			homeDir: homedir(),
			agentDir: getAgentDir(),
			// A canvas's own diagnostics are the user's business: a stray stdout line means
			// its author reached for console.log, and a possible leaked port is worth saying.
			onLog: (id, message) => notify(`[canvas ${id}] ${message}`, "info"),
			onStray: (id, line) => notify(`[canvas ${id}] non-protocol stdout (use session.log): ${line}`, "warning"),
			onDiagnostic: (id, message) => notify(`[canvas ${id}] ${message}`, "warning"),
		});
		return session;
	};

	const registerToolsOnce = (canvas: CanvasSession): void => {
		if (toolsRegistered) return;
		const registry = canvas.registryOrUndefined();
		if (!registry) return;
		toolsRegistered = true;
		for (const definition of createCanvasToolDefinitions(registry)) pi.registerTool(definition);
	};

	pi.registerCommand("canvas", {
		description:
			"Work with canvas extensions. /canvas list | /canvas open <extension>[:<canvas>] | /canvas close <instanceId>",
		getArgumentCompletions: (prefix: string) =>
			SUBCOMMANDS.filter((name) => name.startsWith(prefix)).map((name) => ({ value: name, label: name })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const trimmed = args.trim();
			const canvas = ensureSession(ctx);

			if (trimmed.length === 0 || trimmed === "list") {
				ctx.ui.notify(renderOverview(await canvas.list()), "info");
				return;
			}

			if (trimmed.startsWith("close")) {
				const instanceId = trimmed.slice("close".length).trim();
				if (!instanceId) {
					ctx.ui.notify("Usage: /canvas close <instanceId>  (see /canvas list)", "warning");
					return;
				}
				const closed = await canvas.close(instanceId);
				if (!closed) ctx.ui.notify(`No open canvas instance "${instanceId}".`, "warning");
				else ctx.ui.notify(`Closed ${closed.canvasId} (${closed.instanceId}).`, "info");
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

			// Opening forks a process and binds a port, so it can be slow and must be
			// interruptible. The loader's signal is what makes Esc mean something: it reaches
			// the registry's abandon path, which tells the extension to release a port it may
			// already have bound. Outside a terminal (--print, RPC) there is nothing to draw
			// and nothing to press, so the open simply runs.
			const outcome = ctx.hasUI
				? await ctx.ui.custom<OpenOutcome | undefined>((tui, theme, _keybindings, done) => {
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
					})
				: await canvas
						.open(ref)
						.then((instance): OpenOutcome => ({ kind: "opened", instance }))
						.catch(
							(error: unknown): OpenOutcome => ({
								kind: "failed",
								message: error instanceof Error ? error.message : String(error),
							}),
						);

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
			ctx.ui.notify(
				[
					`Opened ${outcome.instance.canvasId} (${outcome.instance.instanceId}).`,
					outcome.instance.url ? `Open in a browser: ${outcome.instance.url}` : "",
					"The agent can now read and drive it; close it with /canvas close <instanceId>.",
				]
					.filter((line) => line.length > 0)
					.join("\n"),
				"info",
			);
		},
	});

	// Teardown on shutdown (§6): a browser tab gives no close signal, so without this
	// every child and loopback port outlives the session. `session_shutdown` is where
	// loop.ts stops its scheduler, and it is synchronous, so the dispose is fired and
	// not awaited.
	pi.on("session_shutdown", () => {
		const closing = session;
		session = undefined;
		void closing?.dispose();
	});
}
