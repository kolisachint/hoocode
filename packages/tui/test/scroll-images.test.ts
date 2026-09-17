/**
 * Pictures in the pinned window.
 *
 * Scrolling back used to lose them: the window named every image `[image]`
 * rather than drawing it, so a screenshot you had just been shown vanished the
 * moment you scrolled up to look at it again. Drawing one is not free, though —
 * a kitty placement is not text, `CSI 2 K` does not touch it, and deleting one
 * by id deletes *every* placement of that image, the live screen's included.
 * So the two properties under test are: the picture is on screen when all of it
 * fits, and nothing the window does to its own copies can reach the live ones.
 */

import assert from "node:assert";
import { beforeEach, describe, it } from "node:test";
import { Image } from "../src/components/image.js";
import { setCapabilities, setCellDimensions } from "../src/terminal-image.js";
import { type Component, TUI } from "../src/tui.js";
import { VirtualTerminal } from "./virtual-terminal.js";

const WIDTH = 40;
const HEIGHT = 10;
/** The last row is the scroll indicator, so a window shows one fewer. */
const VIEW = HEIGHT - 1;

const BEFORE = 20;
const AFTER = 20;
/** Two blank rows then the line that draws, so the picture is three rows tall. */
const IMAGE_ROWS = 3;
/** Where the drawing line sits in the transcript. */
const IMAGE_LINE = BEFORE + IMAGE_ROWS - 1;

/** A transcript with a three-row picture buried in the middle of it. */
class ImageTranscript implements Component {
	readonly image = new Image(
		Buffer.from("not-a-real-png").toString("base64"),
		"image/png",
		{ fallbackColor: (text) => text },
		{ maxWidthCells: 10 },
		{ widthPx: 90, heightPx: 54 },
	);

	invalidate(): void {
		this.image.invalidate();
	}

	render(width: number): string[] {
		return [
			...Array.from({ length: BEFORE }, (_, i) => `before ${i + 1}`),
			...this.image.render(width),
			...Array.from({ length: AFTER }, (_, i) => `after ${i + 1}`),
		];
	}
}

const KITTY = "\x1b_G";
const PLACEHOLDER = "[image]";

function settle(): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, 40));
}

/** The kitty image id a transmit sequence carries. */
function transmittedId(buffer: string): number | undefined {
	const start = buffer.indexOf(KITTY);
	if (start === -1) return undefined;
	const params = buffer.slice(start + KITTY.length, buffer.indexOf(";", start));
	const id = /(?:^|,)i=(\d+)/.exec(params);
	return id ? Number(id[1]) : undefined;
}

describe("a picture in the pinned window", () => {
	let terminal: VirtualTerminal;
	let tui: TUI;
	let transcript: ImageTranscript;
	let writes: string[];

	beforeEach(async () => {
		setCapabilities({ images: "kitty", trueColor: true, hyperlinks: true });
		setCellDimensions({ widthPx: 9, heightPx: 18 });
		terminal = new VirtualTerminal(WIDTH, HEIGHT);
		writes = [];
		const write = terminal.write.bind(terminal);
		terminal.write = (data: string) => {
			writes.push(data);
			write(data);
		};
		tui = new TUI(terminal);
		transcript = new ImageTranscript();
		tui.addChild(transcript);
		tui.start();
		await settle();
	});

	/** Pin the window with its top row at `top`. */
	async function pin(top: number): Promise<void> {
		tui.scrollToTop();
		if (top > 0) tui.scrollByLines(top);
		await settle();
		assert.equal(tui.getScrollPosition()?.top, top, "the window did not land where the test asked");
		writes.length = 0;
		tui.requestRender();
		await settle();
	}

	it("draws the picture when the whole of it is in the window", async () => {
		// The drawing line two rows down, so the picture's top edge is row 0.
		await pin(IMAGE_LINE - (IMAGE_ROWS - 1));
		const frame = writes.join("");
		assert.ok(frame.includes(KITTY), "no image was transmitted");
		assert.ok(!frame.includes(PLACEHOLDER), "the picture was named rather than drawn");
	});

	it("draws it with its bottom row against the bottom of the window", async () => {
		await pin(IMAGE_LINE - (VIEW - 1));
		const frame = writes.join("");
		assert.ok(frame.includes(KITTY), "no image was transmitted");
		assert.ok(!frame.includes(PLACEHOLDER), "the picture was named rather than drawn");
	});

	it("names it rather than draw it above the first row", async () => {
		// One row further down: the picture now reaches above the window, and a
		// terminal asked to draw there clamps against row 1 and paints over rows
		// that are not the picture's — with no way to take it back.
		await pin(IMAGE_LINE - (IMAGE_ROWS - 2));
		const frame = writes.join("");
		assert.ok(frame.includes(PLACEHOLDER), "the picture was not named");
		assert.ok(!frame.includes(KITTY), "the picture was drawn off the top of the window");
	});

	it("transmits its own copy, under an id the live screen does not use", async () => {
		await pin(IMAGE_LINE - (IMAGE_ROWS - 1));
		const pinned = transmittedId(writes.join(""));
		assert.ok(pinned !== undefined, "the window's copy carries no id to delete it by");
		assert.notEqual(pinned, transcript.image.getImageId());
	});

	it("takes the last frame's copy off before painting the next", async () => {
		await pin(IMAGE_LINE - (IMAGE_ROWS - 1));
		const pinned = transmittedId(writes.join(""));
		assert.ok(pinned !== undefined);

		writes.length = 0;
		tui.scrollByLines(-1);
		await settle();
		const frame = writes.join("");
		const removed = frame.indexOf(`a=d,d=I,i=${pinned},`);
		assert.ok(removed !== -1, "the previous copy was left on the screen");
		assert.ok(removed < frame.indexOf(KITTY, removed + 1), "the copy came off after the repaint, not before");
	});

	it("frees its copies on the way out and leaves the live one alone", async () => {
		await pin(IMAGE_LINE - (IMAGE_ROWS - 1));
		const pinned = transmittedId(writes.join(""));
		assert.ok(pinned !== undefined);

		writes.length = 0;
		tui.scrollToLive();
		await settle();
		const frame = writes.join("");
		assert.ok(frame.includes(`a=d,d=I,i=${pinned},`), "the window's copies were left behind");
		assert.ok(
			!frame.includes(`a=d,d=I,i=${transcript.image.getImageId()},`),
			"deleting the window's copy took the live screen's picture with it",
		);
	});
});
