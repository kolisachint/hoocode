import assert from "node:assert";
import { afterEach, describe, it, mock } from "node:test";
import { Loader } from "../src/components/loader.js";
import { TUI } from "../src/tui.js";
import { visibleWidth } from "../src/utils.js";
import { VirtualTerminal } from "./virtual-terminal.js";

const identity = (s: string) => s;

function createLoader(message: string, indicator?: { frames?: string[]; intervalMs?: number }): Loader {
	const tui = new TUI(new VirtualTerminal(80, 24));
	return new Loader(tui, identity, identity, message, indicator);
}

/** The rendered indicator + message line, without the leading spacer. */
function line(loader: Loader): string {
	return loader.render(40)[1];
}

/**
 * Advance fake time and report the interval at which the indicator actually
 * changed, or undefined if it never did within `limitMs`.
 */
function measureIntervalMs(loader: Loader, limitMs: number): number | undefined {
	const initial = line(loader);
	for (let elapsed = 1; elapsed <= limitMs; elapsed++) {
		mock.timers.tick(1);
		if (line(loader) !== initial) return elapsed;
	}
	return undefined;
}

describe("Loader cadence", () => {
	afterEach(() => mock.timers.reset());

	it("pulses slowly with the default two-frame indicator", () => {
		mock.timers.enable({ apis: ["setInterval"] });
		const loader = createLoader("Working...");
		assert.equal(measureIntervalMs(loader, 1000), 640);
		loader.stop();
	});

	it("keeps a fast cadence for multi-frame spinners", () => {
		mock.timers.enable({ apis: ["setInterval"] });
		const loader = createLoader("Working...", { frames: ["|", "/", "-", "\\"] });
		assert.equal(measureIntervalMs(loader, 1000), 120);
		loader.stop();
	});

	it("honors an explicit interval but floors it so it cannot strobe", () => {
		mock.timers.enable({ apis: ["setInterval"] });
		const slow = createLoader("Working...", { intervalMs: 250 });
		assert.equal(measureIntervalMs(slow, 1000), 250);
		slow.stop();

		const tooFast = createLoader("Working...", { intervalMs: 16 });
		assert.equal(measureIntervalMs(tooFast, 1000), 80);
		tooFast.stop();
	});

	it("does not animate a single-frame indicator", () => {
		mock.timers.enable({ apis: ["setInterval"] });
		const loader = createLoader("Working...", { frames: ["*"] });
		assert.equal(measureIntervalMs(loader, 2000), undefined);
		loader.stop();
	});
});

describe("Loader layout", () => {
	it("stays on one line when the message is longer than the terminal", () => {
		const loader = createLoader("Working on something with a very long description... (Esc to interrupt)");
		const width = 30;
		const lines = loader.render(width);
		loader.stop();

		// A leading blank spacer line plus exactly one content line.
		assert.equal(lines.length, 2);
		assert.equal(lines[0], "");
		assert.equal(visibleWidth(lines[1]), width);
		assert.ok(lines[1].includes("..."), "long messages should be truncated with an ellipsis");
	});

	it("pads short messages to the full width", () => {
		const loader = createLoader("Working...");
		const lines = loader.render(40);
		loader.stop();

		assert.equal(lines.length, 2);
		assert.equal(visibleWidth(lines[1]), 40);
		assert.ok(lines[1].includes("Working..."));
	});

	it("returns the same array while nothing changes", () => {
		const loader = createLoader("Working...", { frames: ["*"] });
		const first = loader.render(40);
		assert.strictEqual(loader.render(40), first, "unchanged renders must stay reference-stable");
		assert.notStrictEqual(loader.render(30), first, "a width change must produce fresh lines");

		loader.setMessage("Still working...");
		assert.notStrictEqual(loader.render(40), first);
		loader.stop();
	});

	it("never exceeds the width as the message changes", () => {
		const loader = createLoader("Working...");
		for (const message of ["Working...", "Reading a file with a really long path that will not fit", "Done"]) {
			loader.setMessage(message);
			for (const width of [10, 24, 80]) {
				const lines = loader.render(width);
				assert.equal(lines.length, 2, `wrapped at width ${width}`);
				assert.equal(visibleWidth(lines[1]), width, `overflowed at width ${width}`);
			}
		}
		loader.stop();
	});
});
