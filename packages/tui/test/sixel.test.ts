/**
 * Sixel: Windows Terminal detection, the encoder, and the Image component path.
 */

import assert from "node:assert";
import { afterEach, describe, it } from "node:test";
import { Image } from "../src/components/image.js";
import {
	detectCapabilities,
	encodeSixel,
	isImageLine,
	type RgbaImage,
	renderImage,
	resetCapabilitiesCache,
	setCapabilities,
	setCellDimensions,
	setImageRasterizer,
} from "../src/terminal-image.js";

const ENV_KEYS = [
	"TERM",
	"TERM_PROGRAM",
	"COLORTERM",
	"TMUX",
	"KITTY_WINDOW_ID",
	"GHOSTTY_RESOURCES_DIR",
	"WEZTERM_PANE",
	"ITERM_SESSION_ID",
	"WT_SESSION",
	"HOOCODE_IMAGE_PROTOCOL",
] as const;

function withEnv(overrides: Record<string, string>, fn: () => void): void {
	const saved: Record<string, string | undefined> = {};
	for (const key of ENV_KEYS) {
		saved[key] = process.env[key];
		delete process.env[key];
	}
	try {
		Object.assign(process.env, overrides);
		fn();
	} finally {
		for (const key of ENV_KEYS) {
			if (saved[key] === undefined) delete process.env[key];
			else process.env[key] = saved[key];
		}
	}
}

function solid(width: number, height: number, rgba: [number, number, number, number]): RgbaImage {
	const data = new Uint8Array(width * height * 4);
	for (let i = 0; i < data.length; i += 4) data.set(rgba, i);
	return { data, width, height };
}

/** Decode a sixel back to the palette index of every pixel (-1 = undrawn). */
function decodeSixel(sequence: string): { width: number; height: number; pixels: number[]; palette: string[] } {
	const header = /^\x1bP0;1;0q"1;1;(\d+);(\d+)/.exec(sequence);
	assert.ok(header, "raster attributes present");
	const width = Number(header[1]);
	const height = Number(header[2]);
	assert.ok(sequence.endsWith("\x1b\\"), "terminated by ST");
	const body = sequence.slice(header[0].length, -2);
	const pixels = new Array<number>(width * height).fill(-1);
	const palette: string[] = [];
	let color = 0;
	let x = 0;
	let top = 0;
	let i = 0;
	while (i < body.length) {
		const ch = body[i];
		if (ch === "#") {
			const m = /^#(\d+)(;2;(\d+);(\d+);(\d+))?/.exec(body.slice(i));
			assert.ok(m);
			color = Number(m[1]);
			if (m[2]) palette[color] = `${m[3]},${m[4]},${m[5]}`;
			i += m[0].length;
		} else if (ch === "$") {
			x = 0;
			i++;
		} else if (ch === "-") {
			x = 0;
			top += 6;
			i++;
		} else {
			let run = 1;
			if (ch === "!") {
				const m = /^!(\d+)/.exec(body.slice(i));
				assert.ok(m);
				run = Number(m[1]);
				i += m[0].length;
			}
			const bits = body.charCodeAt(i) - 63;
			assert.ok(bits >= 0 && bits < 64, `sixel data char at ${i}`);
			for (let r = 0; r < run; r++, x++) {
				for (let dy = 0; dy < 6; dy++) {
					if (bits & (1 << dy) && top + dy < height) pixels[(top + dy) * width + x] = color;
				}
			}
			i++;
		}
	}
	return { width, height, pixels, palette };
}

afterEach(() => {
	setImageRasterizer(null);
	resetCapabilitiesCache();
	setCellDimensions({ widthPx: 9, heightPx: 18 });
});

describe("Windows Terminal detection", () => {
	it("uses sixel when WT_SESSION is set", () => {
		withEnv({ WT_SESSION: "7c3b4b6e-0000-0000-0000-000000000000" }, () => {
			const caps = detectCapabilities();
			assert.strictEqual(caps.images, "sixel");
			assert.strictEqual(caps.trueColor, true);
		});
	});

	it("still defers to tmux", () => {
		withEnv({ WT_SESSION: "x", TMUX: "/tmp/tmux-1000/default,1,0" }, () => {
			assert.strictEqual(detectCapabilities().images, null);
		});
	});

	it("HOOCODE_IMAGE_PROTOCOL overrides detection either way", () => {
		withEnv({ HOOCODE_IMAGE_PROTOCOL: "sixel" }, () => {
			assert.strictEqual(detectCapabilities().images, "sixel");
		});
		withEnv({ WT_SESSION: "x", HOOCODE_IMAGE_PROTOCOL: "none" }, () => {
			assert.strictEqual(detectCapabilities().images, null);
		});
		withEnv({ TERM_PROGRAM: "ghostty", HOOCODE_IMAGE_PROTOCOL: "bogus" }, () => {
			assert.strictEqual(detectCapabilities().images, "kitty");
		});
	});
});

describe("encodeSixel", () => {
	it("round-trips a two-colour image, including a partial last band", () => {
		const image = solid(5, 8, [255, 0, 0, 255]);
		// Bottom-right pixel blue, top-left transparent.
		image.data.set([0, 0, 255, 255], (7 * 5 + 4) * 4);
		image.data.set([0, 0, 0, 0], 0);
		const sequence = encodeSixel(image);
		assert.ok(isImageLine(sequence));

		const decoded = decodeSixel(sequence);
		assert.strictEqual(decoded.width, 5);
		assert.strictEqual(decoded.height, 8);
		assert.strictEqual(decoded.pixels[0], -1, "transparent pixel is left undrawn");
		const red = decoded.pixels[1];
		const blue = decoded.pixels[7 * 5 + 4];
		assert.strictEqual(decoded.palette[red], "100,0,0");
		assert.strictEqual(decoded.palette[blue], "0,0,100");
		for (let p = 1; p < 39; p++) assert.strictEqual(decoded.pixels[p], red, `pixel ${p}`);
	});

	it("run-length encodes long runs", () => {
		const sequence = encodeSixel(solid(200, 6, [10, 20, 30, 255]));
		assert.ok(sequence.includes("!200~"), sequence);
	});

	it("reduces many colours to the palette limit", () => {
		const width = 64;
		const height = 64;
		const data = new Uint8Array(width * height * 4);
		for (let y = 0; y < height; y++) {
			for (let x = 0; x < width; x++) data.set([x * 4, y * 4, (x ^ y) * 4, 255], (y * width + x) * 4);
		}
		const decoded = decodeSixel(encodeSixel({ data, width, height }, 16));
		assert.ok(decoded.palette.length <= 16);
		assert.ok(decoded.pixels.every((p) => p >= 0 && p < 16));
	});
});

describe("renderImage with sixel", () => {
	it("falls back to nothing without a rasterizer", () => {
		setCapabilities({ images: "sixel", trueColor: true, hyperlinks: false });
		assert.strictEqual(renderImage("AA==", { widthPx: 100, heightPx: 100 }, { mimeType: "image/png" }), null);
	});

	it("scales down to the width in cells, never up, and counts rows from pixels", () => {
		setCapabilities({ images: "sixel", trueColor: true, hyperlinks: false });
		setCellDimensions({ widthPx: 10, heightPx: 20 });
		const requested: Array<[number, number]> = [];
		setImageRasterizer((_data, _mime, w, h) => {
			requested.push([w, h]);
			return solid(w, h, [0, 128, 0, 255]);
		});

		const big = renderImage("x", { widthPx: 1000, heightPx: 500 }, { maxWidthCells: 40, mimeType: "image/png" });
		assert.deepStrictEqual(requested[0], [400, 200]);
		assert.strictEqual(big?.rows, 10);

		const small = renderImage("x", { widthPx: 30, heightPx: 45 }, { maxWidthCells: 40, mimeType: "image/png" });
		assert.deepStrictEqual(requested[1], [30, 45]);
		assert.strictEqual(small?.rows, 3);
	});

	it("Image reserves its rows and restores the cursor around the sixel", () => {
		setCapabilities({ images: "sixel", trueColor: true, hyperlinks: false });
		setCellDimensions({ widthPx: 10, heightPx: 20 });
		setImageRasterizer((_data, _mime, w, h) => solid(w, h, [0, 0, 0, 255]));
		const image = new Image(
			"x",
			"image/png",
			{ fallbackColor: (s) => s },
			{ maxWidthCells: 40 },
			{
				widthPx: 400,
				heightPx: 100,
			},
		);
		const lines = image.render(80);
		assert.strictEqual(lines.length, 5);
		assert.deepStrictEqual(lines.slice(0, 4), ["", "", "", ""]);
		const last = lines[4];
		assert.ok(last.startsWith("\x1b[4A\x1b7\x1bP0;1;0q"), JSON.stringify(last.slice(0, 20)));
		assert.ok(last.endsWith("\x1b\\\x1b8\x1b[4B"));
	});

	it("Image shows the text fallback when the rasterizer fails", () => {
		setCapabilities({ images: "sixel", trueColor: true, hyperlinks: false });
		setImageRasterizer(() => {
			throw new Error("corrupt");
		});
		const image = new Image(
			"x",
			"image/png",
			{ fallbackColor: (s) => s },
			{ filename: "a.png" },
			{
				widthPx: 10,
				heightPx: 10,
			},
		);
		assert.deepStrictEqual(image.render(80), ["[Image: a.png [image/png] 10x10]"]);
	});
});
