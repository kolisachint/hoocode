/**
 * Generates the colored half-block ANSI art of the Hoo owl symbol used in the
 * interactive startup banner.
 *
 * Source: <repo-root>/assets/symbol.svg
 * Output: packages/coding-agent/src/core/wordmark-symbol.generated.ts
 *
 * Each terminal cell renders two vertically stacked source pixels using the
 * upper-half-block glyph "▀": the top pixel is the foreground color and the
 * bottom pixel is the background color. Transparent pixels fall back to the
 * terminal default (no color) so the mark composites onto any background.
 *
 * Re-run after changing the symbol artwork:
 *   npx tsx scripts/generate-wordmark-symbol.ts
 */

import { createCanvas, loadImage } from "canvas";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, "../../..");
const SVG_PATH = resolve(REPO_ROOT, "assets/symbol.svg");
const OUT_PATH = resolve(__dirname, "../src/core/wordmark-symbol.generated.ts");

// Symbol viewBox is 168x80 (aspect 2.1:1). Two source pixel rows per cell, one
// source pixel column per cell, so COLS / (ROWS * 2) should match the aspect.
const COLS = 42;
const ROWS = 10;
const ALPHA_THRESHOLD = 100;

const ESC = "\\x1b";
const RESET = `${ESC}[0m`;

function fg(r: number, g: number, b: number): string {
	return `${ESC}[38;2;${r};${g};${b}m`;
}

function bg(r: number, g: number, b: number): string {
	return `${ESC}[48;2;${r};${g};${b}m`;
}

async function main(): Promise<void> {
	const svg = readFileSync(SVG_PATH);
	const img = await loadImage(svg);

	const widthPx = COLS;
	const heightPx = ROWS * 2;
	const canvas = createCanvas(widthPx, heightPx);
	const ctx = canvas.getContext("2d");
	ctx.clearRect(0, 0, widthPx, heightPx);
	ctx.drawImage(img, 0, 0, widthPx, heightPx);
	const { data } = ctx.getImageData(0, 0, widthPx, heightPx);

	const px = (x: number, y: number): [number, number, number, number] => {
		const i = (y * widthPx + x) * 4;
		return [data[i], data[i + 1], data[i + 2], data[i + 3]];
	};

	const lines: string[] = [];
	const debug: string[] = [];
	for (let row = 0; row < ROWS; row++) {
		let line = "";
		let dbg = "";
		for (let col = 0; col < COLS; col++) {
			const [tr, tg, tb, ta] = px(col, row * 2);
			const [br, bg_, bb, ba] = px(col, row * 2 + 1);
			const topOn = ta >= ALPHA_THRESHOLD;
			const botOn = ba >= ALPHA_THRESHOLD;

			if (topOn && botOn) {
				line += `${fg(tr, tg, tb)}${bg(br, bg_, bb)}▀${RESET}`;
				dbg += "█";
			} else if (topOn) {
				line += `${fg(tr, tg, tb)}▀${RESET}`;
				dbg += "▀";
			} else if (botOn) {
				line += `${fg(br, bg_, bb)}▄${RESET}`;
				dbg += "▄";
			} else {
				line += " ";
				dbg += " ";
			}
		}
		lines.push(line.replace(/\s+$/, ""));
		debug.push(dbg.replace(/\s+$/, ""));
	}

	// Print a shape preview to stderr for visual verification.
	process.stderr.write(`${debug.join("\n")}\n`);

	const body = lines.map((l) => `\t"${l}",`).join("\n");
	const out = `/**
 * Colored half-block ANSI art of the Hoo owl symbol for the startup banner.
 *
 * GENERATED FILE - do not edit by hand.
 * Regenerate with: npx tsx scripts/generate-wordmark-symbol.ts
 * Source: assets/symbol.svg
 */

export const WORDMARK_SYMBOL_BLOCKS = [
${body}
].join("\\n");
`;
	writeFileSync(OUT_PATH, out);
	process.stderr.write(`\nWrote ${OUT_PATH}\n`);
}

main().catch((err) => {
	process.stderr.write(`${err instanceof Error ? err.stack : String(err)}\n`);
	process.exit(1);
});
