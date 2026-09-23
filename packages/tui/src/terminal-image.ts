export type ImageProtocol = "kitty" | "iterm2" | "sixel" | null;

export interface TerminalCapabilities {
	images: ImageProtocol;
	trueColor: boolean;
	hyperlinks: boolean;
}

export interface CellDimensions {
	widthPx: number;
	heightPx: number;
}

export interface ImageDimensions {
	widthPx: number;
	heightPx: number;
}

export interface ImageRenderOptions {
	maxWidthCells?: number;
	maxHeightCells?: number;
	preserveAspectRatio?: boolean;
	/** Kitty image ID. If provided, reuses/replaces existing image with this ID. */
	imageId?: number;
	/** Whether Kitty should apply its default cursor movement after placement. */
	moveCursor?: boolean;
	/** MIME type of `base64Data`. Sixel needs it to decode the image into pixels. */
	mimeType?: string;
}

/** Decoded pixels, 4 bytes (RGBA) per pixel, row-major. */
export interface RgbaImage {
	data: Uint8Array;
	width: number;
	height: number;
}

/**
 * Decodes an encoded image and scales it to exactly `widthPx` x `heightPx`.
 *
 * Sixel carries pixels, not a PNG, so the terminal cannot decode the image for
 * us the way kitty and iTerm2 do. Decoding needs an image library, which this
 * package deliberately does not depend on: the host registers one with
 * {@link setImageRasterizer}. Until it does, a sixel terminal gets the text
 * fallback. Must be synchronous, since it runs inside `render()`.
 */
export type ImageRasterizer = (
	base64Data: string,
	mimeType: string,
	widthPx: number,
	heightPx: number,
) => RgbaImage | null;

let imageRasterizer: ImageRasterizer | null = null;

export function setImageRasterizer(rasterizer: ImageRasterizer | null): void {
	imageRasterizer = rasterizer;
}

export function getImageRasterizer(): ImageRasterizer | null {
	return imageRasterizer;
}

let cachedCapabilities: TerminalCapabilities | null = null;

// Default cell dimensions - updated by TUI when terminal responds to query
let cellDimensions: CellDimensions = { widthPx: 9, heightPx: 18 };

export function getCellDimensions(): CellDimensions {
	return cellDimensions;
}

export function setCellDimensions(dims: CellDimensions): void {
	cellDimensions = dims;
}

/**
 * `HOOCODE_IMAGE_PROTOCOL=kitty|iterm2|sixel|none` names the protocol outright,
 * for terminals detection cannot identify (foot, mlterm, xterm -ti vt340, a
 * Windows Terminal reached over SSH) or gets wrong. Anything else is ignored.
 */
function imageProtocolOverride(): ImageProtocol | undefined {
	const value = process.env.HOOCODE_IMAGE_PROTOCOL?.trim().toLowerCase();
	if (value === "kitty" || value === "iterm2" || value === "sixel") return value;
	if (value === "none" || value === "off" || value === "0") return null;
	return undefined;
}

export function detectCapabilities(): TerminalCapabilities {
	const detected = detectTerminalCapabilities();
	const override = imageProtocolOverride();
	return override === undefined ? detected : { ...detected, images: override };
}

function detectTerminalCapabilities(): TerminalCapabilities {
	const termProgram = process.env.TERM_PROGRAM?.toLowerCase() || "";
	const term = process.env.TERM?.toLowerCase() || "";
	const colorTerm = process.env.COLORTERM?.toLowerCase() || "";

	// tmux and screen swallow OSC 8 by default (passthrough is opt-in and wraps
	// sequences differently). Force hyperlinks off whenever we detect them, even
	// when the outer terminal would otherwise support OSC 8. Image protocols are
	// also unreliable under tmux/screen, so leave `images: null` for safety.
	const inTmuxOrScreen = !!process.env.TMUX || term.startsWith("tmux") || term.startsWith("screen");
	if (inTmuxOrScreen) {
		const trueColor = colorTerm === "truecolor" || colorTerm === "24bit";
		return { images: null, trueColor, hyperlinks: false };
	}

	if (process.env.KITTY_WINDOW_ID || termProgram === "kitty") {
		return { images: "kitty", trueColor: true, hyperlinks: true };
	}

	if (termProgram === "ghostty" || term.includes("ghostty") || process.env.GHOSTTY_RESOURCES_DIR) {
		return { images: "kitty", trueColor: true, hyperlinks: true };
	}

	if (process.env.WEZTERM_PANE || termProgram === "wezterm") {
		return { images: "kitty", trueColor: true, hyperlinks: true };
	}

	if (process.env.ITERM_SESSION_ID || termProgram === "iterm.app") {
		return { images: "iterm2", trueColor: true, hyperlinks: true };
	}

	// Windows Terminal speaks neither kitty nor iTerm2 graphics, only Sixel
	// (1.22 and later; older builds ignore the sequence and show nothing, which
	// is what they showed before). It is the default terminal on Windows 11, and
	// it announces itself only through WT_SESSION — TERM_PROGRAM is unset.
	if (process.env.WT_SESSION) {
		return { images: "sixel", trueColor: true, hyperlinks: false };
	}

	if (termProgram === "vscode") {
		return { images: null, trueColor: true, hyperlinks: true };
	}

	if (termProgram === "alacritty") {
		return { images: null, trueColor: true, hyperlinks: true };
	}

	// Unknown terminal: be conservative. OSC 8 is rendered invisibly as "just
	// text" on terminals that swallow it, which means the URL disappears from
	// the rendered output. Default to the legacy `text (url)` behavior unless we
	// have positively identified a hyperlink-capable terminal above.
	const trueColor = colorTerm === "truecolor" || colorTerm === "24bit";
	return { images: null, trueColor, hyperlinks: false };
}

export function getCapabilities(): TerminalCapabilities {
	if (!cachedCapabilities) {
		cachedCapabilities = detectCapabilities();
	}
	return cachedCapabilities;
}

export function resetCapabilitiesCache(): void {
	cachedCapabilities = null;
}

/** Override the cached capabilities. Useful in tests to exercise both code paths. */
export function setCapabilities(caps: TerminalCapabilities): void {
	cachedCapabilities = caps;
}

const KITTY_PREFIX = "\x1b_G";
const ITERM2_PREFIX = "\x1b]1337;File=";
/** DCS with the parameters {@link encodeSixel} always writes: square pixels, transparent background. */
const SIXEL_PREFIX = "\x1bP0;1;0q";

export function isImageLine(line: string): boolean {
	// Fast path: sequence at line start (single-row images)
	if (line.startsWith(KITTY_PREFIX) || line.startsWith(ITERM2_PREFIX) || line.startsWith(SIXEL_PREFIX)) {
		return true;
	}
	// Slow path: sequence elsewhere (multi-row images have cursor-up prefix)
	return line.includes(KITTY_PREFIX) || line.includes(ITERM2_PREFIX) || line.includes(SIXEL_PREFIX);
}

/**
 * Generate a random image ID for Kitty graphics protocol.
 * Uses random IDs to avoid collisions between different module instances
 * (e.g., main app vs extensions).
 */
export function allocateImageId(): number {
	// Use random ID in range [1, 0xffffffff] to avoid collisions
	return Math.floor(Math.random() * 0xfffffffe) + 1;
}

export function encodeKitty(
	base64Data: string,
	options: {
		columns?: number;
		rows?: number;
		imageId?: number;
		/** Whether Kitty should apply its default cursor movement after placement. Default: true. */
		moveCursor?: boolean;
	} = {},
): string {
	const CHUNK_SIZE = 4096;

	const params: string[] = ["a=T", "f=100", "q=2"];

	if (options.moveCursor === false) params.push("C=1");
	if (options.columns) params.push(`c=${options.columns}`);
	if (options.rows) params.push(`r=${options.rows}`);
	if (options.imageId) params.push(`i=${options.imageId}`);

	if (base64Data.length <= CHUNK_SIZE) {
		return `\x1b_G${params.join(",")};${base64Data}\x1b\\`;
	}

	const chunks: string[] = [];
	let offset = 0;
	let isFirst = true;

	while (offset < base64Data.length) {
		const chunk = base64Data.slice(offset, offset + CHUNK_SIZE);
		const isLast = offset + CHUNK_SIZE >= base64Data.length;

		if (isFirst) {
			chunks.push(`\x1b_G${params.join(",")},m=1;${chunk}\x1b\\`);
			isFirst = false;
		} else if (isLast) {
			chunks.push(`\x1b_Gm=0;${chunk}\x1b\\`);
		} else {
			chunks.push(`\x1b_Gm=1;${chunk}\x1b\\`);
		}

		offset += CHUNK_SIZE;
	}

	return chunks.join("");
}

/**
 * Delete a Kitty graphics image by ID.
 * Uses uppercase 'I' to also free the image data.
 */
export function deleteKittyImage(imageId: number): string {
	return `\x1b_Ga=d,d=I,i=${imageId},q=2\x1b\\`;
}

/**
 * Delete all visible Kitty graphics images.
 * Uses uppercase 'A' to also free the image data.
 */
export function deleteAllKittyImages(): string {
	return "\x1b_Ga=d,d=A,q=2\x1b\\";
}

export function encodeITerm2(
	base64Data: string,
	options: {
		width?: number | string;
		height?: number | string;
		name?: string;
		preserveAspectRatio?: boolean;
		inline?: boolean;
	} = {},
): string {
	const params: string[] = [`inline=${options.inline !== false ? 1 : 0}`];

	if (options.width !== undefined) params.push(`width=${options.width}`);
	if (options.height !== undefined) params.push(`height=${options.height}`);
	if (options.name) {
		const nameBase64 = Buffer.from(options.name).toString("base64");
		params.push(`name=${nameBase64}`);
	}
	if (options.preserveAspectRatio === false) {
		params.push("preserveAspectRatio=0");
	}

	return `\x1b]1337;File=${params.join(";")}:${base64Data}\x07`;
}

/**
 * Build a palette of at most `maxColors` for the opaque pixels of `data`, by
 * median cut over a 15-bit (5 bits per channel) histogram.
 *
 * Returns the palette as 8-bit RGB triples and a lookup from 15-bit colour key
 * to palette index. An image with no more distinct keys than `maxColors` — a
 * screenshot of text, a diagram — keeps every one of them.
 */
function buildSixelPalette(data: Uint8Array, maxColors: number): { palette: number[]; lookup: Uint16Array } {
	const counts = new Uint32Array(32768);
	for (let i = 0; i < data.length; i += 4) {
		if (data[i + 3] < 128) continue;
		counts[((data[i] >> 3) << 10) | ((data[i + 1] >> 3) << 5) | (data[i + 2] >> 3)]++;
	}
	const keys: number[] = [];
	for (let key = 0; key < counts.length; key++) if (counts[key] > 0) keys.push(key);

	const channel = (key: number, c: number): number => (key >> (10 - c * 5)) & 31;
	type Box = { keys: number[]; channel: number; range: number };
	const measure = (boxKeys: number[]): Box => {
		let best = 0;
		let bestRange = -1;
		for (let c = 0; c < 3; c++) {
			let lo = 31;
			let hi = 0;
			for (const key of boxKeys) {
				const v = channel(key, c);
				if (v < lo) lo = v;
				if (v > hi) hi = v;
			}
			if (hi - lo > bestRange) {
				bestRange = hi - lo;
				best = c;
			}
		}
		return { keys: boxKeys, channel: best, range: bestRange };
	};

	const boxes: Box[] = keys.length > 0 ? [measure(keys)] : [];
	while (boxes.length < maxColors) {
		// Split the box spanning the widest range; boxes of one key cannot split.
		let target = -1;
		for (let i = 0; i < boxes.length; i++) {
			if (boxes[i].keys.length > 1 && (target === -1 || boxes[i].range > boxes[target].range)) target = i;
		}
		if (target === -1) break;
		const box = boxes[target];
		const c = box.channel;
		box.keys.sort((a, b) => channel(a, c) - channel(b, c));
		let total = 0;
		for (const key of box.keys) total += counts[key];
		// Weighted median, kept strictly inside so both halves are non-empty.
		let seen = 0;
		let split = 1;
		for (let i = 0; i < box.keys.length - 1; i++) {
			seen += counts[box.keys[i]];
			split = i + 1;
			if (seen * 2 >= total) break;
		}
		boxes.splice(target, 1, measure(box.keys.slice(0, split)), measure(box.keys.slice(split)));
	}

	const palette: number[] = [];
	const lookup = new Uint16Array(32768);
	const expand = (v: number): number => (v << 3) | (v >> 2);
	boxes.forEach((box, index) => {
		let r = 0;
		let g = 0;
		let b = 0;
		let n = 0;
		for (const key of box.keys) {
			const w = counts[key];
			r += expand(channel(key, 0)) * w;
			g += expand(channel(key, 1)) * w;
			b += expand(channel(key, 2)) * w;
			n += w;
			lookup[key] = index;
		}
		palette.push(Math.round(r / n), Math.round(g / n), Math.round(b / n));
	});
	return { palette, lookup };
}

/** One sixel row of one colour: 6-bit column masks, run-length encoded. */
function encodeSixelRow(masks: Uint8Array): string {
	let end = masks.length;
	while (end > 0 && masks[end - 1] === 0) end--;
	let out = "";
	let i = 0;
	while (i < end) {
		const value = masks[i];
		let run = 1;
		while (i + run < end && masks[i + run] === value) run++;
		const char = String.fromCharCode(63 + value);
		out += run > 3 ? `!${run}${char}` : char.repeat(run);
		i += run;
	}
	return out;
}

/**
 * Encode RGBA pixels as a Sixel image (DEC VT340 graphics; Windows Terminal,
 * foot, mlterm, xterm, WezTerm, Konsole).
 *
 * Pixels with alpha below 128 are left undrawn, so the terminal background
 * shows through. At most `maxColors` palette registers are used; 256 is what
 * Windows Terminal and xterm provide.
 */
export function encodeSixel(image: RgbaImage, maxColors = 256): string {
	const { data, width, height } = image;
	const { palette, lookup } = buildSixelPalette(data, maxColors);

	const parts: string[] = [`${SIXEL_PREFIX}"1;1;${width};${height}`];
	for (let i = 0; i < palette.length; i += 3) {
		const pct = (v: number): number => Math.round((v * 100) / 255);
		parts.push(`#${i / 3};2;${pct(palette[i])};${pct(palette[i + 1])};${pct(palette[i + 2])}`);
	}

	for (let top = 0; top < height; top += 6) {
		const bandHeight = Math.min(6, height - top);
		const bands = new Map<number, Uint8Array>();
		for (let dy = 0; dy < bandHeight; dy++) {
			const bit = 1 << dy;
			let offset = (top + dy) * width * 4;
			for (let x = 0; x < width; x++, offset += 4) {
				if (data[offset + 3] < 128) continue;
				const index =
					lookup[((data[offset] >> 3) << 10) | ((data[offset + 1] >> 3) << 5) | (data[offset + 2] >> 3)];
				let masks = bands.get(index);
				if (!masks) {
					masks = new Uint8Array(width);
					bands.set(index, masks);
				}
				masks[x] |= bit;
			}
		}
		const rows: string[] = [];
		for (const [index, masks] of bands) rows.push(`#${index}${encodeSixelRow(masks)}`);
		// `$` returns to the start of the band for the next colour; `-` moves to the next band.
		parts.push(`${rows.join("$")}-`);
	}

	parts.push("\x1b\\");
	return parts.join("");
}

export function calculateImageRows(
	imageDimensions: ImageDimensions,
	targetWidthCells: number,
	cellDimensions: CellDimensions = { widthPx: 9, heightPx: 18 },
): number {
	const targetWidthPx = targetWidthCells * cellDimensions.widthPx;
	const scale = targetWidthPx / imageDimensions.widthPx;
	const scaledHeightPx = imageDimensions.heightPx * scale;
	const rows = Math.ceil(scaledHeightPx / cellDimensions.heightPx);
	return Math.max(1, rows);
}

export function getPngDimensions(base64Data: string): ImageDimensions | null {
	try {
		const buffer = Buffer.from(base64Data, "base64");

		if (buffer.length < 24) {
			return null;
		}

		if (buffer[0] !== 0x89 || buffer[1] !== 0x50 || buffer[2] !== 0x4e || buffer[3] !== 0x47) {
			return null;
		}

		const width = buffer.readUInt32BE(16);
		const height = buffer.readUInt32BE(20);

		return { widthPx: width, heightPx: height };
	} catch {
		return null;
	}
}

export function getJpegDimensions(base64Data: string): ImageDimensions | null {
	try {
		const buffer = Buffer.from(base64Data, "base64");

		if (buffer.length < 2) {
			return null;
		}

		if (buffer[0] !== 0xff || buffer[1] !== 0xd8) {
			return null;
		}

		let offset = 2;
		while (offset < buffer.length - 9) {
			if (buffer[offset] !== 0xff) {
				offset++;
				continue;
			}

			const marker = buffer[offset + 1];

			if (marker >= 0xc0 && marker <= 0xc2) {
				const height = buffer.readUInt16BE(offset + 5);
				const width = buffer.readUInt16BE(offset + 7);
				return { widthPx: width, heightPx: height };
			}

			if (offset + 3 >= buffer.length) {
				return null;
			}
			const length = buffer.readUInt16BE(offset + 2);
			if (length < 2) {
				return null;
			}
			offset += 2 + length;
		}

		return null;
	} catch {
		return null;
	}
}

export function getGifDimensions(base64Data: string): ImageDimensions | null {
	try {
		const buffer = Buffer.from(base64Data, "base64");

		if (buffer.length < 10) {
			return null;
		}

		const sig = buffer.slice(0, 6).toString("ascii");
		if (sig !== "GIF87a" && sig !== "GIF89a") {
			return null;
		}

		const width = buffer.readUInt16LE(6);
		const height = buffer.readUInt16LE(8);

		return { widthPx: width, heightPx: height };
	} catch {
		return null;
	}
}

export function getWebpDimensions(base64Data: string): ImageDimensions | null {
	try {
		const buffer = Buffer.from(base64Data, "base64");

		if (buffer.length < 30) {
			return null;
		}

		const riff = buffer.slice(0, 4).toString("ascii");
		const webp = buffer.slice(8, 12).toString("ascii");
		if (riff !== "RIFF" || webp !== "WEBP") {
			return null;
		}

		const chunk = buffer.slice(12, 16).toString("ascii");
		if (chunk === "VP8 ") {
			if (buffer.length < 30) return null;
			const width = buffer.readUInt16LE(26) & 0x3fff;
			const height = buffer.readUInt16LE(28) & 0x3fff;
			return { widthPx: width, heightPx: height };
		} else if (chunk === "VP8L") {
			if (buffer.length < 25) return null;
			const bits = buffer.readUInt32LE(21);
			const width = (bits & 0x3fff) + 1;
			const height = ((bits >> 14) & 0x3fff) + 1;
			return { widthPx: width, heightPx: height };
		} else if (chunk === "VP8X") {
			if (buffer.length < 30) return null;
			const width = (buffer[24] | (buffer[25] << 8) | (buffer[26] << 16)) + 1;
			const height = (buffer[27] | (buffer[28] << 8) | (buffer[29] << 16)) + 1;
			return { widthPx: width, heightPx: height };
		}

		return null;
	} catch {
		return null;
	}
}

export function getImageDimensions(base64Data: string, mimeType: string): ImageDimensions | null {
	if (mimeType === "image/png") {
		return getPngDimensions(base64Data);
	}
	if (mimeType === "image/jpeg") {
		return getJpegDimensions(base64Data);
	}
	if (mimeType === "image/gif") {
		return getGifDimensions(base64Data);
	}
	if (mimeType === "image/webp") {
		return getWebpDimensions(base64Data);
	}
	return null;
}

export function renderImage(
	base64Data: string,
	imageDimensions: ImageDimensions,
	options: ImageRenderOptions = {},
): { sequence: string; rows: number; imageId?: number } | null {
	const caps = getCapabilities();

	if (!caps.images) {
		return null;
	}

	const maxWidth = options.maxWidthCells ?? 80;
	const rows = calculateImageRows(imageDimensions, maxWidth, getCellDimensions());

	if (caps.images === "kitty") {
		const sequence = encodeKitty(base64Data, {
			columns: maxWidth,
			rows,
			imageId: options.imageId,
			moveCursor: options.moveCursor,
		});
		return { sequence, rows, imageId: options.imageId };
	}

	if (caps.images === "iterm2") {
		const sequence = encodeITerm2(base64Data, {
			width: maxWidth,
			height: "auto",
			preserveAspectRatio: options.preserveAspectRatio ?? true,
		});
		return { sequence, rows };
	}

	if (caps.images === "sixel") {
		return renderSixel(base64Data, imageDimensions, maxWidth, options);
	}

	return null;
}

/**
 * Sixel draws pixels, so the size in cells follows from the size in pixels
 * rather than the other way round. The image is scaled to fit `maxWidthCells`
 * (and `maxHeightCells`) but never enlarged — an enlarged sixel is only blurrier —
 * and `rows` is how many cells tall the result is.
 */
function renderSixel(
	base64Data: string,
	imageDimensions: ImageDimensions,
	maxWidthCells: number,
	options: ImageRenderOptions,
): { sequence: string; rows: number } | null {
	const rasterize = imageRasterizer;
	if (!rasterize || !options.mimeType) return null;
	if (imageDimensions.widthPx <= 0 || imageDimensions.heightPx <= 0) return null;

	const cell = getCellDimensions();
	let scale = Math.min(1, (maxWidthCells * cell.widthPx) / imageDimensions.widthPx);
	if (options.maxHeightCells) {
		scale = Math.min(scale, (options.maxHeightCells * cell.heightPx) / imageDimensions.heightPx);
	}
	const widthPx = Math.max(1, Math.floor(imageDimensions.widthPx * scale));
	const heightPx = Math.max(1, Math.floor(imageDimensions.heightPx * scale));

	let pixels: RgbaImage | null;
	try {
		pixels = rasterize(base64Data, options.mimeType, widthPx, heightPx);
	} catch {
		return null;
	}
	if (!pixels || pixels.width <= 0 || pixels.height <= 0) return null;
	if (pixels.data.length < pixels.width * pixels.height * 4) return null;

	const rows = Math.max(1, Math.ceil(pixels.height / cell.heightPx));
	return { sequence: encodeSixel(pixels), rows };
}

/**
 * Wrap text in an OSC 8 hyperlink sequence.
 * The text is rendered as a clickable hyperlink in terminals that support OSC 8
 * (Ghostty, Kitty, WezTerm, iTerm2, VSCode, and others).
 * In terminals that do not support OSC 8, the escape sequences are ignored
 * and only the plain text is displayed.
 *
 * @param text - The visible text to display
 * @param url - The URL to link to
 */
export function hyperlink(text: string, url: string): string {
	return `\x1b]8;;${url}\x1b\\${text}\x1b]8;;\x1b\\`;
}

export function imageFallback(mimeType: string, dimensions?: ImageDimensions, filename?: string): string {
	const parts: string[] = [];
	if (filename) parts.push(filename);
	parts.push(`[${mimeType}]`);
	if (dimensions) parts.push(`${dimensions.widthPx}x${dimensions.heightPx}`);
	return `[Image: ${parts.join(" ")}]`;
}
