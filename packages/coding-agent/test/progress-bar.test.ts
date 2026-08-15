/**
 * The shared progress bar, and the property that motivates it: two surfaces
 * showing the same progress must render it the same way.
 *
 * The bar had drifted three ways before being extracted — glyphs, byte format,
 * and layout — so the cross-surface assertions below are the point of the file,
 * not incidental coverage.
 *
 * Spinners are deliberately NOT unified the same way; see the rotation test at
 * the bottom for why frame count is a functional choice rather than a style one.
 */

import { beforeAll, describe, expect, it } from "vitest";
import {
	formatMegabytes,
	PROGRESS_BAR_CELLS,
	renderDownloadProgress,
	renderProgressBar,
} from "../src/modes/interactive/components/progress-bar.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";
import { VoicePanel } from "../src/modes/interactive/voice/voice-panel.js";

/** Styling stripped — what a low-contrast theme or a piped capture actually shows. */
const plain = (text: string | string[]) =>
	(Array.isArray(text) ? text.join("\n") : text).replace(/\x1b\[[0-9;]*m/g, "");

beforeAll(() => {
	initTheme("dark");
});

describe("renderProgressBar", () => {
	it("distinguishes filled from empty by shape, not only by colour", () => {
		expect(plain(renderProgressBar(0.25, "3/12 sessions"))).toBe("▰▰▰▱▱▱▱▱▱▱▱▱ 25% · 3/12 sessions");
	});

	it("keeps one filled cell once work has started", () => {
		// 1/40 rounds to zero cells. "Nothing yet" and "the first of forty" are
		// different things to be told.
		expect(plain(renderProgressBar(1 / 40, "1/40 files"))).toContain("▰▱▱▱▱▱▱▱▱▱▱▱");
	});

	it("shows an untouched bar as fully empty", () => {
		expect(plain(renderProgressBar(0, "0/12 files"))).toContain("▱▱▱▱▱▱▱▱▱▱▱▱");
	});

	it("clamps out-of-range ratios rather than overflowing the track", () => {
		expect(plain(renderProgressBar(1.5, "x"))).toContain(`${"▰".repeat(PROGRESS_BAR_CELLS)} 100%`);
		expect(plain(renderProgressBar(-1, "x"))).toContain(`${"▱".repeat(PROGRESS_BAR_CELLS)} 0%`);
	});

	it("lets a surface choose its own width, and nothing else", () => {
		const wide = plain(renderProgressBar(0.5, "x", { cells: 20 }));
		expect(wide).toBe(`${"▰".repeat(10)}${"▱".repeat(10)} 50% · x`);
	});
});

describe("formatMegabytes", () => {
	it("keeps one decimal, so a slow download does not look frozen", () => {
		expect(formatMegabytes(450 * 1024 * 1024)).toBe("450.0 MB");
		expect(formatMegabytes(1536 * 1024)).toBe("1.5 MB");
	});
});

describe("renderDownloadProgress", () => {
	it("drops the bar for a running count when the length is unknown", () => {
		// A bar that cannot reach the end is worse than no bar.
		const text = plain(renderDownloadProgress(3 * 1024 * 1024, null));
		expect(text).toBe("3.0 MB…");
		expect(text).not.toContain("▰");
	});

	it("treats a zero or negative length as unknown", () => {
		expect(plain(renderDownloadProgress(1024 * 1024, 0))).toBe("1.0 MB…");
	});
});

describe("wait indicators agree across surfaces", () => {
	it("renders the voice panel's download exactly like a footer bar of the same width", () => {
		// The invariant the extraction exists to guarantee. Before it, this panel
		// drew `·` over `·` at whole megabytes while the footer drew `▰▱` at tenths.
		const panel = new VoicePanel(undefined, "esc cancel");
		panel.setWarming("Starting voice input…");
		panel.setDownloadProgress(1 * 1024 * 1024, 4 * 1024 * 1024);

		const shared = plain(renderDownloadProgress(1 * 1024 * 1024, 4 * 1024 * 1024, { cells: 20 }));
		expect(plain(panel.render(80)).replace(/\s+/g, " ")).toContain(shared.replace(/\s+/g, " "));
		panel.dispose();
	});

	it("spins with a rotation, not the Loader's two-frame pulse", () => {
		// Deliberately NOT unified with the Loader's `○`/`●`. Frame count is
		// functional: two frames read as the line switching on and off, which is
		// why the Loader beats them at 640ms rather than spinner speed. This panel
		// ticks at 100ms to drive its silence countdown, so a two-frame indicator
		// here would be precisely the strobe the Loader exists to avoid.
		const panel = new VoicePanel(undefined, "esc cancel");
		panel.setWarming("Starting voice input…");
		expect(plain(panel.render(80))).toMatch(/[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]/);
		panel.dispose();
	});
});
