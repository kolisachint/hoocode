/**
 * Photon-backed pixel decoding for Sixel terminals (Windows Terminal, foot, …).
 *
 * kitty and iTerm2 are sent the encoded PNG/JPEG and decode it themselves; a
 * Sixel terminal is sent pixels. The TUI has no image library of its own, so it
 * takes a rasterizer from the host (`setImageRasterizer`), and this is ours.
 * Photon already ships with every build — including the standalone binary, which
 * carries `photon_rs_bg.wasm` beside the executable — so this adds no dependency.
 */

import { getCapabilities, type RgbaImage, setImageRasterizer } from "@kolisachint/hoocode-tui";
import { loadPhoton } from "./photon.js";

/**
 * Register the rasterizer when the terminal speaks Sixel. Resolves once it is in
 * place (or photon could not load, in which case images keep their text
 * fallback), so callers can await it before the first render.
 */
export async function installSixelRasterizer(): Promise<boolean> {
	if (getCapabilities().images !== "sixel") return false;
	const photon = await loadPhoton();
	if (!photon) return false;

	setImageRasterizer((base64Data, _mimeType, widthPx, heightPx): RgbaImage | null => {
		const source = photon.PhotonImage.new_from_byteslice(new Uint8Array(Buffer.from(base64Data, "base64")));
		try {
			const sized =
				source.get_width() === widthPx && source.get_height() === heightPx
					? source
					: photon.resize(source, widthPx, heightPx, photon.SamplingFilter.Lanczos3);
			try {
				return { data: sized.get_raw_pixels(), width: sized.get_width(), height: sized.get_height() };
			} finally {
				if (sized !== source) sized.free();
			}
		} finally {
			source.free();
		}
	});
	return true;
}
