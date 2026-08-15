/**
 * End-to-end TUI coverage for startup progress. Mounts a real `TUI` with the
 * `FooterComponent` and a capturing terminal, then drives the full chain the app
 * uses at startup:
 *
 *   EmbsearchState → reportEmbsearchProgress → startupProgress store
 *     → subscription → tui.requestRender() → footer.render() → terminal writes
 *
 * so a regression anywhere in that pipeline (mapping, store, footer, render) is
 * caught against real rendered output rather than a unit boundary.
 */

import { type Terminal, TUI } from "@kolisachint/hoocode-tui";
import { afterAll, afterEach, beforeAll, describe, expect, test } from "vitest";
import { reportEmbsearchProgress } from "../src/core/embsearch/embsearch-progress.js";
import { FooterDataProvider } from "../src/core/footer-data-provider.js";
import { startupProgress } from "../src/core/startup-progress.js";
import { FooterComponent } from "../src/modes/interactive/components/footer.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";
import { createHarness, type Harness } from "./suite/harness.js";

/** Minimal capturing terminal — accumulates everything the TUI writes. */
class FakeTerminal implements Terminal {
	columns = 100;
	rows = 30;
	kittyProtocolActive = true;
	writes: string[] = [];

	start(): void {}
	stop(): void {}
	async drainInput(): Promise<void> {}
	write(data: string): void {
		this.writes.push(data);
	}
	moveBy(): void {}
	hideCursor(): void {}
	showCursor(): void {}
	clearLine(): void {}
	clearFromCursor(): void {}
	clearScreen(): void {}
	setTitle(): void {}
	setProgress(): void {}
}

const nextTick = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Poll the rendered output until it contains `text`, or fail after a timeout. */
async function waitForText(rendered: () => string, text: string, timeoutMs = 2000): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	let last = "";
	while (Date.now() < deadline) {
		await nextTick();
		last = rendered();
		if (last.includes(text)) return;
	}
	throw new Error(`Timed out waiting for "${text}". Last render:\n${last}`);
}

describe("startup progress in a live TUI", () => {
	let harness: Harness;
	let terminal: FakeTerminal;
	let tui: TUI;
	let footer: FooterComponent;
	let provider: FooterDataProvider;
	let unsubscribe: () => void;

	beforeAll(async () => {
		initTheme("dark");
		harness = await createHarness();
		provider = new FooterDataProvider(harness.tempDir);
		footer = new FooterComponent(harness.session, provider);

		terminal = new FakeTerminal();
		tui = new TUI(terminal);
		tui.addChild(footer);
		// Mirror InteractiveMode: re-render the footer whenever startup progress moves.
		unsubscribe = startupProgress.subscribe(() => tui.requestRender());
		tui.start();
		await nextTick();
	});

	afterEach(() => {
		startupProgress.clear();
	});

	afterAll(() => {
		unsubscribe();
		try {
			tui.stop();
		} catch {
			// TUI may already be stopped; ignore.
		}
		provider.dispose();
		harness.cleanup();
	});

	const rendered = () => terminal.writes.join("").replace(/\x1b\[[0-9;]*m/g, "");

	test("renders the semantic-index download bar, then the build bar", async () => {
		reportEmbsearchProgress(
			{ phase: "downloading", receivedBytes: 2_097_152, totalBytes: 8_388_608 },
			{ interactive: true },
		);
		await waitForText(rendered, "Semantic search index");
		await waitForText(rendered, "2.0 MB / 8.0 MB");

		reportEmbsearchProgress({ phase: "indexing", done: 5, total: 20 }, { interactive: true });
		await waitForText(rendered, "Building semantic search index");
		await waitForText(rendered, "5/20 files");
	});

	test("clears the line once the index is ready", async () => {
		reportEmbsearchProgress({ phase: "indexing", done: 5, total: 20 }, { interactive: true });
		await waitForText(rendered, "Building semantic search index");

		// Fresh capture, then settle the index and force a full redraw: the footer
		// re-renders without the startup line, so the label is absent going forward.
		terminal.writes = [];
		reportEmbsearchProgress({ phase: "ready", chunkCount: 100 }, { interactive: true });
		tui.requestRender(true);
		await nextTick();
		await nextTick();
		expect(rendered()).not.toContain("Semantic search index");
	});

	test("shows concurrent tool downloads as separate lines", async () => {
		startupProgress.set({
			key: "fd",
			kind: "download",
			label: "fd",
			receivedBytes: 1_048_576,
			totalBytes: 4_194_304,
		});
		startupProgress.set({
			key: "rg",
			kind: "download",
			label: "ripgrep",
			receivedBytes: 3_145_728,
			totalBytes: 4_194_304,
		});
		await waitForText(rendered, "fd");
		await waitForText(rendered, "ripgrep");
		await waitForText(rendered, "75%"); // rg: 3/4 MB
	});

	test("shows /learn reading sessions on the same bar as the index", async () => {
		startupProgress.set({
			key: "learn-mining",
			kind: "work",
			label: "Reading sessions (2 cached) — esc to stop",
			done: 3,
			total: 12,
			unit: "sessions",
		});
		await waitForText(rendered, "Reading sessions (2 cached)");
		await waitForText(rendered, "3/12 sessions");
		await waitForText(rendered, "25%");
	});

	test("distinguishes filled from empty by shape, not only colour", async () => {
		// Styling is stripped from `rendered()`, so this asserts what a
		// low-contrast theme or a piped capture actually shows. The previous
		// `·`-over-`·` fill left 10% and 90% looking identical here.
		startupProgress.set({
			key: "learn-mining",
			kind: "work",
			label: "Reading sessions",
			done: 3,
			total: 12,
			unit: "sessions",
		});
		await waitForText(rendered, "▰▰▰▱▱▱▱▱▱▱▱▱");
	});

	test("keeps one filled cell once work has started, so the first item is visible", async () => {
		// 1/40 rounds to zero cells. "Nothing has happened yet" and "the first of
		// forty is done" should not look the same.
		startupProgress.set({
			key: "learn-mining",
			kind: "work",
			label: "Reading sessions",
			done: 1,
			total: 40,
			unit: "sessions",
		});
		await waitForText(rendered, "▰▱▱▱▱▱▱▱▱▱▱▱");
	});
});
