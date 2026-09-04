/**
 * Screenshot harness for interactive mode.
 *
 * Mounts a real `InteractiveMode` — real runtime, real extensions, real
 * resource loader, real components — against a capturing terminal, drives it
 * through real slash commands, and renders the whole frame to text. That makes
 * "does `/reload` show what startup showed" an assertion over what the user
 * actually sees, rather than over a unit boundary somewhere below it.
 *
 * Everything outside the frame is faux and offline: the faux provider, an
 * in-memory auth store, and a throwaway HOME so the machine's own skills and
 * settings cannot leak into a screenshot.
 */

import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type FauxProviderRegistration, registerFauxProvider } from "@kolisachint/hoocode-ai";
import type { Terminal } from "@kolisachint/hoocode-tui";
import {
	type CreateAgentSessionRuntimeFactory,
	createAgentSessionFromServices,
	createAgentSessionRuntime,
	createAgentSessionServices,
} from "../../src/core/agent-session-runtime.js";
import { AuthStorage } from "../../src/core/auth-storage.js";
import { SessionManager } from "../../src/core/session-manager.js";
import { InteractiveMode } from "../../src/modes/interactive/interactive-mode.js";

const ESC = String.fromCharCode(27);
const ANSI = new RegExp(`${ESC}\\[[0-9;?]*[a-zA-Z]`, "g");

/** Writes nowhere: the frame is read back from the component tree instead. */
class CapturingTerminal implements Terminal {
	columns = 100;
	rows = 40;
	kittyProtocolActive = false;
	start(): void {}
	stop(): void {}
	async drainInput(): Promise<void> {}
	write(): void {}
	moveBy(): void {}
	hideCursor(): void {}
	showCursor(): void {}
	clearLine(): void {}
	clearFromCursor(): void {}
	clearScreen(): void {}
	setTitle(): void {}
	setProgress(): void {}
}

export interface SurfaceHarness {
	/** Working directory of the throwaway project. */
	cwd: string;
	/** Directory holding `settings.json`, user prompts, and the rest of the user scope. */
	agentDir: string;
	/** Run a slash command exactly as a keypress would: through the editor. */
	submit(text: string): Promise<void>;
	/** The whole frame, ANSI stripped and trailing space trimmed. */
	screenshot(): string;
	/** `screenshot()` with the per-session noise masked out — see `maskVolatile`. */
	surface(): string;
	/** The whole frame with its colours intact, for asserting on the theme. */
	rawFrame(): string;
	/**
	 * Every distinct foreground escape in the frame, sorted. Two frames drawn
	 * with the same theme share a palette, and `screenshot()` throws exactly this
	 * away — so a theme that failed to reach one surface is invisible without it.
	 * Background fills are left out: the session chip picks one at random per
	 * session, which says nothing about the theme.
	 */
	palette(): string[];
	cleanup(): void;
}

/**
 * Two things in a frame differ between two runs of the same command and say
 * nothing about which resources loaded:
 *
 * - the session chip, a random name per session, drawn into the editor's top
 *   border;
 * - the transient status line each command leaves behind ("New session
 *   started", "Reloaded ...") — that line is the command's receipt, not its
 *   surface.
 *
 * The harness's own throwaway directory is masked too, so two harnesses can be
 * compared against each other.
 */
function maskVolatile(frame: string, root: string): string {
	const transient = /^\s*(✓ New session started|Reloaded keybindings|Resumed session|Session compacted)/;
	return frame
		.split("\n")
		.map((line) => (line.startsWith("┌") ? "┌<editor>┐" : line))
		.filter((line) => !transient.test(line))
		.join("\n")
		.split(root)
		.join("<root>")
		.replace(/\n{3,}/g, "\n\n")
		.trim();
}

export interface SurfaceHarnessOptions {
	/** Extra files to write into the project before the first session starts. */
	project?: Record<string, string>;
	/** `settings.json` for the user scope, written before the first session starts. */
	settings?: Record<string, unknown>;
}

export async function createSurfaceHarness(options: SurfaceHarnessOptions = {}): Promise<SurfaceHarness> {
	// Fixed width: the footer pads around the cwd, so two harnesses only render
	// identically when their roots are the same length.
	const suffix = Math.random().toString(36).slice(2, 10).padEnd(8, "0");
	const root = join(tmpdir(), `pi-surface-${Date.now()}-${suffix}`);
	const cwd = join(root, "project");
	const home = join(root, "home");
	const agentDir = join(root, "agent");
	mkdirSync(cwd, { recursive: true });
	mkdirSync(home, { recursive: true });
	mkdirSync(agentDir, { recursive: true });

	if (options.settings) {
		writeFileSync(join(agentDir, "settings.json"), JSON.stringify(options.settings, null, 2));
	}

	for (const [relativePath, content] of Object.entries(options.project ?? {})) {
		const target = join(cwd, relativePath);
		mkdirSync(join(target, ".."), { recursive: true });
		writeFileSync(target, content);
	}

	const previousHome = process.env.HOME;
	const previousUserProfile = process.env.USERPROFILE;
	process.env.HOME = home;
	process.env.USERPROFILE = home;

	const faux: FauxProviderRegistration = registerFauxProvider({ models: [{ id: "faux-1", reasoning: false }] });
	const model = faux.getModel();
	const authStorage = AuthStorage.inMemory();
	authStorage.setRuntimeApiKey(model.provider, "faux-key");

	const createRuntime: CreateAgentSessionRuntimeFactory = async ({
		cwd: sessionCwd,
		sessionManager,
		sessionStartEvent,
	}) => {
		const services = await createAgentSessionServices({
			cwd: sessionCwd,
			agentDir,
			authStorage,
			resourceLoaderOptions: {
				extensionFactories: [
					(pi) => {
						pi.registerProvider(model.provider, {
							baseUrl: model.baseUrl,
							apiKey: "faux-key",
							api: faux.api,
							models: faux.models.map((registered) => ({
								id: registered.id,
								name: registered.name,
								api: registered.api,
								reasoning: registered.reasoning,
								input: registered.input,
								cost: registered.cost,
								contextWindow: registered.contextWindow,
								maxTokens: registered.maxTokens,
							})),
						});
					},
				],
			},
		});
		return {
			...(await createAgentSessionFromServices({ services, sessionManager, sessionStartEvent, model })),
			services,
			diagnostics: services.diagnostics,
		};
	};

	const runtime = await createAgentSessionRuntime(createRuntime, {
		cwd,
		agentDir,
		sessionManager: SessionManager.create(cwd),
	});
	const mode = new InteractiveMode(runtime, { terminal: new CapturingTerminal() });
	await mode.init();

	// The frame and the submit handler are private to the mode by design; the
	// harness is the one caller allowed to look at them.
	const internals = mode as unknown as {
		ui: { render(width: number): string[] };
		defaultEditor: { onSubmit(text: string): Promise<void> };
	};

	const rawFrame = () => internals.ui.render(100).join("\n");
	const screenshot = () =>
		rawFrame()
			.replace(ANSI, "")
			.split("\n")
			.map((line) => line.replace(/\s+$/g, ""))
			.join("\n")
			.trim();

	return {
		cwd,
		agentDir,
		submit: (text: string) => internals.defaultEditor.onSubmit(text),
		screenshot,
		surface: () => maskVolatile(screenshot(), root),
		rawFrame,
		palette: () => [...new Set(rawFrame().match(ANSI) ?? [])].filter((code) => !code.includes("[48;")).sort(),
		cleanup: () => {
			mode.stop();
			runtime.session.dispose();
			faux.unregister();
			process.env.HOME = previousHome;
			process.env.USERPROFILE = previousUserProfile;
			if (existsSync(root)) rmSync(root, { recursive: true, force: true });
		},
	};
}
