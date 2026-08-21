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
 * `pi.sendUserMessage`. Its decisions live in `core/canvas/scaffold.ts`.
 *
 * The agent tools register on the first successful open and stay for the session:
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
import { CANVAS_HOMES, canvasBuildBrief, parseCanvasRequest, scaffoldCanvas } from "../../core/canvas/scaffold.js";
import {
	type CanvasOverview,
	CanvasSession,
	type CanvasSessionOptions,
	parseCanvasRef,
} from "../../core/canvas/session.js";
import { getWorkspacePlatforms } from "../../core/extensions/plugins/formats/platform-targets.js";
import { isWorkspaceTrusted, trustWorkspace } from "../../core/extensions/plugins/trust.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";
import { createCanvasToolDefinitions } from "../../core/tools/canvas.js";
import { BorderedLoader } from "../../modes/interactive/components/bordered-loader.js";

const SUBCOMMANDS = ["list", "open", "close", "reload"] as const;

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
}

export function setupCanvas(pi: ExtensionAPI, overrides?: CanvasSetupOverrides): void {
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
			homeDir: overrides?.homeDir ?? homedir(),
			agentDir: getAgentDir(),
			resolveRuntime: overrides?.resolveRuntime,
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
	): Promise<OpenOutcome | undefined> =>
		ctx.hasUI
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

	pi.registerCommand("canvas", {
		description:
			"Work with canvas extensions. /canvas list | /canvas open <extension>[:<canvas>] | /canvas reload [extension] | /canvas close <instanceId>",
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
					for (const instance of result.reopened) {
						// The url is the point of saying anything: the extension binds a new
						// port and mints a new token on every open, so the tab the person has
						// in front of them is now pointing at a closed port.
						lines.push(`  ${describeInstance(instance)}`);
					}
					if (result.reopened.length > 0) lines.push("", "Open the new url(s) — the previous tab is dead.");
					for (const drop of result.dropped) {
						lines.push(`  ${drop.canvasId} (${drop.instanceId}) did not come back: ${drop.reason}`);
					}
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

	// ── /new-canvas <name> | <description> ────────────────────────────────────
	// Authoring, in Copilot's `/create-canvas` shape: describe what you want, the
	// agent writes it, and it is already open while it does. Design:
	// `docs/canvas-extensions-design.md` §9 Phase 3, §13.
	//
	// Unlike the other `/new-*` scaffolds this one needs no /reload: canvases are
	// discovered when /canvas runs, not loaded at session start.

	pi.registerCommand("new-canvas", {
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
			if (opened) registerToolsOnce(canvas);

			const lines = ["Canvas created:", ...created.map((file) => `  ${file}`)];
			if (skipped.length > 0) lines.push("Skipped (already exist):", ...skipped.map((file) => `  ${file}`));
			lines.push("");
			if (opened) {
				lines.push(`Opened ${opened.canvasId} (${opened.instanceId}).`);
				if (opened.url) lines.push(`Open in a browser: ${opened.url}`);
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
				await pi.sendUserMessage(
					canvasBuildBrief(
						request.name,
						request.description,
						created[0] as string,
						opened ? { instanceId: opened.instanceId, url: opened.url } : undefined,
					),
					{ deliverAs: "followUp" },
				);
			}
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
