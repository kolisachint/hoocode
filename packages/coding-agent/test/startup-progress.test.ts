/**
 * End-to-end coverage for the startup-progress feature: the shared
 * `startupProgress` store, the `reportEmbsearchProgress` state mapping (the
 * logic lifted out of main.ts), and the footer rendering that turns store
 * entries into transient status lines. Together these are the whole pipeline
 * that surfaces first-run tool downloads and the semantic-index build.
 */

import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "vitest";
import { reportEmbsearchProgress, SEMANTIC_INDEX_PROGRESS_KEY } from "../src/core/embsearch/embsearch-progress.js";
import type { EmbsearchState } from "../src/core/embsearch/embsearch-service.js";
import { FooterDataProvider } from "../src/core/footer-data-provider.js";
import { startupProgress } from "../src/core/startup-progress.js";
import { FooterComponent } from "../src/modes/interactive/components/footer.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";
import { createHarness, type Harness } from "./suite/harness.js";

/** Strip SGR colour codes so assertions match on visible text. */
function plain(text: string): string {
	return text.replace(/\x1b\[[0-9;]*m/g, "");
}

// The store is a process singleton shared across tests; reset around each one.
beforeEach(() => startupProgress.clear());
afterEach(() => startupProgress.clear());

describe("startupProgress store", () => {
	test("set/list keeps entries in insertion order", () => {
		startupProgress.set({ key: "fd", kind: "download", label: "fd", receivedBytes: 1, totalBytes: 10 });
		startupProgress.set({ key: "rg", kind: "download", label: "ripgrep", receivedBytes: 2, totalBytes: 20 });
		expect(startupProgress.list().map((e) => e.key)).toEqual(["fd", "rg"]);
	});

	test("updating an existing key replaces in place without reordering", () => {
		startupProgress.set({ key: "fd", kind: "download", label: "fd", receivedBytes: 1, totalBytes: 10 });
		startupProgress.set({ key: "rg", kind: "download", label: "ripgrep", receivedBytes: 2, totalBytes: 20 });
		startupProgress.set({ key: "fd", kind: "download", label: "fd", receivedBytes: 9, totalBytes: 10 });

		const list = startupProgress.list();
		expect(list.map((e) => e.key)).toEqual(["fd", "rg"]);
		expect(list[0]).toMatchObject({ key: "fd", receivedBytes: 9 });
	});

	test("remove drops one key; clear wipes all", () => {
		startupProgress.set({ key: "fd", kind: "download", label: "fd", receivedBytes: 1, totalBytes: 10 });
		startupProgress.set({ key: "rg", kind: "download", label: "ripgrep", receivedBytes: 2, totalBytes: 20 });
		startupProgress.remove("fd");
		expect(startupProgress.list().map((e) => e.key)).toEqual(["rg"]);
		startupProgress.clear();
		expect(startupProgress.list()).toHaveLength(0);
	});

	test("subscribe fires on set/remove/clear and stops after unsubscribe", () => {
		let notifications = 0;
		const unsubscribe = startupProgress.subscribe(() => notifications++);

		startupProgress.set({ key: "fd", kind: "download", label: "fd", receivedBytes: 1, totalBytes: 10 });
		startupProgress.remove("fd");
		startupProgress.clear(); // no-op (already empty) → should NOT notify
		expect(notifications).toBe(2);

		unsubscribe();
		startupProgress.set({ key: "rg", kind: "download", label: "ripgrep", receivedBytes: 1, totalBytes: 2 });
		expect(notifications).toBe(2);
	});

	test("removing an unknown key does not notify", () => {
		let notifications = 0;
		const unsubscribe = startupProgress.subscribe(() => notifications++);
		startupProgress.remove("nope");
		expect(notifications).toBe(0);
		unsubscribe();
	});
});

describe("reportEmbsearchProgress — interactive (footer store)", () => {
	const set = (state: EmbsearchState) => reportEmbsearchProgress(state, { interactive: true });
	const indexEntry = () => startupProgress.list().find((e) => e.key === SEMANTIC_INDEX_PROGRESS_KEY);

	test("downloading then indexing share one evolving key", () => {
		set({ phase: "downloading", receivedBytes: 1024, totalBytes: 4096 });
		expect(startupProgress.list()).toHaveLength(1);
		expect(indexEntry()).toMatchObject({ kind: "download", receivedBytes: 1024, totalBytes: 4096 });

		set({ phase: "indexing", done: 3, total: 12 });
		expect(startupProgress.list()).toHaveLength(1);
		expect(indexEntry()).toMatchObject({ kind: "work", label: "Building semantic search index", done: 3, total: 12 });
	});

	test("ready and skipped drop the line", () => {
		set({ phase: "indexing", done: 1, total: 2 });
		set({ phase: "ready", chunkCount: 99 });
		expect(indexEntry()).toBeUndefined();

		set({ phase: "indexing", done: 1, total: 2 });
		set({ phase: "skipped", reason: "under threshold" });
		expect(indexEntry()).toBeUndefined();
	});

	test("unavailable shows a transient error entry", () => {
		set({ phase: "unavailable", reason: "binary not found" });
		expect(indexEntry()).toMatchObject({ kind: "error", message: "binary not found" });
	});

	test("idle produces nothing", () => {
		set({ phase: "idle" });
		expect(startupProgress.list()).toHaveLength(0);
	});
});

describe("reportEmbsearchProgress — non-interactive (stderr log)", () => {
	function collect(state: EmbsearchState): string[] {
		const logs: string[] = [];
		reportEmbsearchProgress(state, { interactive: false, log: (m) => logs.push(m) });
		return logs;
	}

	test("indexing logs a plain-language progress line", () => {
		expect(collect({ phase: "indexing", done: 3, total: 12 })).toEqual([
			"Building semantic search index – 3/12 files (25%)",
		]);
	});

	test("ready/skipped/unavailable log plain-language lines", () => {
		expect(collect({ phase: "ready", chunkCount: 42 })).toEqual(["Semantic search index ready (42 chunks)"]);
		expect(collect({ phase: "skipped", reason: "too small" })).toEqual(["Semantic search index skipped (too small)"]);
		expect(collect({ phase: "unavailable", reason: "offline" })).toEqual([
			"Semantic search index unavailable (offline)",
		]);
	});

	test("downloading and idle stay silent (no per-chunk spam)", () => {
		expect(collect({ phase: "downloading", receivedBytes: 1, totalBytes: 2 })).toEqual([]);
		expect(collect({ phase: "idle" })).toEqual([]);
	});

	test("non-interactive never touches the footer store", () => {
		collect({ phase: "indexing", done: 1, total: 2 });
		collect({ phase: "unavailable", reason: "x" });
		expect(startupProgress.list()).toHaveLength(0);
	});
});

describe("footer rendering of startup progress", () => {
	let harness: Harness;
	let footer: FooterComponent;
	let provider: FooterDataProvider;

	beforeAll(async () => {
		initTheme("dark");
		harness = await createHarness();
		provider = new FooterDataProvider(harness.tempDir);
		footer = new FooterComponent(harness.session, provider);
	});

	afterAll(() => {
		provider.dispose();
		harness.cleanup();
	});

	const WIDTH = 120;
	const render = () => footer.render(WIDTH);
	// Startup lines are always appended last; with a single entry the final
	// footer line is that entry. Scope assertions to it so unrelated footer
	// content (e.g. the context-usage percentage) never confuses the match.
	const lastLine = () => plain(render().at(-1) ?? "");

	test("empty store adds no footer lines", () => {
		const base = render().length;
		startupProgress.set({ key: "fd", kind: "download", label: "fd", receivedBytes: 1, totalBytes: 2 });
		expect(render().length).toBe(base + 1);
	});

	test("determinate download shows a bar, percent, and MB counts", () => {
		startupProgress.set({
			key: "fd",
			kind: "download",
			label: "fd",
			receivedBytes: 3_670_016, // 3.5 MB
			totalBytes: 8_388_608, // 8.0 MB
		});
		const line = lastLine();
		expect(line).toContain("fd");
		expect(line).toContain("3.5 MB / 8.0 MB");
		expect(line).toContain("44%");
		expect(line).toContain("·"); // bar fill glyph
	});

	test("indeterminate download (no Content-Length) shows bytes without a percent bar", () => {
		startupProgress.set({
			key: "rg",
			kind: "download",
			label: "ripgrep",
			receivedBytes: 1_048_576,
			totalBytes: null,
		});
		const line = lastLine();
		expect(line).toContain("ripgrep");
		expect(line).toContain("1.0 MB");
		expect(line).not.toContain("%");
	});

	test("index build shows a done/total files bar", () => {
		startupProgress.set({
			key: SEMANTIC_INDEX_PROGRESS_KEY,
			kind: "work",
			label: "Building semantic search index",
			done: 120,
			total: 480,
			unit: "files",
		});
		const line = lastLine();
		expect(line).toContain("Building semantic search index");
		expect(line).toContain("120/480 files");
		expect(line).toContain("25%");
	});

	test("error entry shows the message and no bar", () => {
		startupProgress.set({
			key: SEMANTIC_INDEX_PROGRESS_KEY,
			kind: "error",
			label: "Semantic search index unavailable",
			message: "binary not found",
		});
		const line = lastLine();
		expect(line).toContain("Semantic search index unavailable: binary not found");
		expect(line).not.toContain("·");
	});

	test("multiple concurrent entries render one line each, in order", () => {
		const base = render().length;
		startupProgress.set({ key: "fd", kind: "download", label: "fd", receivedBytes: 1, totalBytes: 10 });
		startupProgress.set({ key: "rg", kind: "download", label: "ripgrep", receivedBytes: 2, totalBytes: 10 });
		startupProgress.set({
			key: SEMANTIC_INDEX_PROGRESS_KEY,
			kind: "work",
			label: "Building semantic search index",
			done: 1,
			total: 4,
			unit: "files",
		});

		const lines = render();
		expect(lines.length).toBe(base + 3);
		const startupLines = lines.slice(base).map(plain);
		expect(startupLines[0]).toContain("fd");
		expect(startupLines[1]).toContain("ripgrep");
		expect(startupLines[2]).toContain("Building semantic search index");
	});

	test("an over-long startup line is clamped to the terminal width", () => {
		startupProgress.set({
			key: "long",
			kind: "download",
			label: "a-very-long-tool-name-".repeat(20),
			receivedBytes: 5_000_000,
			totalBytes: 9_000_000,
		});
		// The startup line (appended last) must be clamped to the width; the label
		// is ASCII so plain length equals visible width here.
		expect(plain(footer.render(60).at(-1) ?? "").length).toBeLessThanOrEqual(60);
	});
});
