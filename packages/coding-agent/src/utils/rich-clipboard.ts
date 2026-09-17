/**
 * Putting two flavours of the same thing on the clipboard at once.
 *
 * A clipboard holds one *content* in several representations, and the pasting
 * application picks the richest it understands. That is the whole mechanism
 * behind a paste into Word keeping its headings while the same paste into a
 * terminal arrives as text. `copyToClipboard` writes one flavour — plain text —
 * so a transcript pasted into Word arrived as a wall of markdown with the
 * asterisks still in it.
 *
 * So: markdown on `text/plain`, HTML on the platform's rich flavour, both from
 * one call. Nothing about the copy changes for a plain-text target.
 *
 * ## What each platform can carry
 *
 * - **macOS** takes both onto the general pasteboard in one clearing, so a
 *   paste into Word is rich and a paste into a terminal is the markdown.
 * - **Windows** takes both through a `DataObject`, with the HTML wrapped in the
 *   CF_HTML envelope the format requires (and PowerShell run `-STA`, because
 *   the clipboard APIs are apartment-threaded).
 * - **Linux** cannot: X11 and Wayland clipboards are owned by one process
 *   advertising one set of targets, and `xclip`/`wl-copy` advertise the single
 *   type they were given. Offering HTML there would mean *replacing* the plain
 *   text with HTML source for every other paste target, which is a worse
 *   clipboard than the one we started with. So Linux gets the markdown, and the
 *   caller is told which flavour landed rather than left to guess.
 *
 * Every path falls back to `copyToClipboard`: a rich copy that fails is still a
 * copy, and the user finds out from the status line what they got.
 */

import { spawn } from "child_process";
import { mkdtempSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { copyToClipboard } from "./clipboard.js";

/** What actually reached the clipboard. */
export type CopyFlavour = "rich" | "text";

export interface RichPayload {
	/** The `text/plain` flavour — markdown, for anything that takes text. */
	text: string;
	/** The rich flavour — an HTML *fragment*, no `<html>` or `<body>`. */
	html: string;
}

/**
 * Past this, the rich flavour is dropped rather than handed to a shell.
 *
 * Not a correctness bound — the payloads go through files, not arguments — but
 * a sanity one: a megabyte of HTML on the clipboard stalls the applications
 * that try to render a preview of it, and a session that large is an export,
 * not a paste.
 */
const MAX_RICH_BYTES = 2_000_000;

/** Run a command to completion, quietly; resolves false on any failure. */
function run(command: string, args: string[]): Promise<boolean> {
	return new Promise((resolve) => {
		try {
			const child = spawn(command, args, { stdio: ["ignore", "ignore", "ignore"] });
			child.on("error", () => resolve(false));
			child.on("close", (code) => resolve(code === 0));
		} catch {
			resolve(false);
		}
	});
}

/**
 * The CF_HTML envelope Windows requires.
 *
 * The header carries byte offsets into the string that contains it, which makes
 * it self-referential: the offsets are only correct once they are the width
 * they will be when written. Fixed-width zero-padded fields are how the format
 * solves that, and why these are padded to ten digits rather than printed
 * plainly.
 *
 * Exported for its own test: the arithmetic is invisible until a paste lands in
 * Word with its first tag missing, which is a long way from here.
 */
export function wrapCfHtml(fragment: string): string {
	const header = "Version:0.9\r\nStartHTML:%%%1\r\nEndHTML:%%%2\r\nStartFragment:%%%3\r\nEndFragment:%%%4\r\n";
	const open = "<html><body>\r\n<!--StartFragment-->";
	const close = "<!--EndFragment-->\r\n</body></html>";
	const pad = (value: number): string => value.toString().padStart(10, "0");
	const headerLength = Buffer.byteLength(header.replace(/%%%\d/g, "0000000000"), "utf8");
	const startHtml = headerLength;
	const startFragment = startHtml + Buffer.byteLength(open, "utf8");
	const endFragment = startFragment + Buffer.byteLength(fragment, "utf8");
	const endHtml = endFragment + Buffer.byteLength(close, "utf8");
	return (
		header
			.replace("%%%1", pad(startHtml))
			.replace("%%%2", pad(endHtml))
			.replace("%%%3", pad(startFragment))
			.replace("%%%4", pad(endFragment)) +
		open +
		fragment +
		close
	);
}

/**
 * macOS: both flavours onto the general pasteboard, in one clearing.
 *
 * Through JXA (`osascript -l JavaScript`) rather than AppleScript, and the
 * reason is encoding. AppleScript's name for a raw pasteboard type is
 * `«class HTML»`, so the script itself has to carry non-ASCII, and `osascript`
 * decodes a plain-text script file by the current locale — on a machine whose
 * locale is not UTF-8 the chevrons arrive mangled and the script fails to
 * compile. This script is pure ASCII whatever the payload is, because both
 * payloads are *read from files* rather than quoted into it: a transcript is
 * exactly the kind of text that contains quotes, backslashes and newlines, and
 * escaping it into a script literal is a bug waiting for the first code block.
 *
 * `clearContents` before the writes is what makes the two a single item on the
 * pasteboard, which is what lets one paste choose between them.
 */
async function copyRichDarwin(payload: RichPayload, dir: string): Promise<boolean> {
	const htmlPath = join(dir, "clip.html");
	const textPath = join(dir, "clip.txt");
	const scriptPath = join(dir, "clip.js");
	writeFileSync(htmlPath, payload.html, "utf8");
	writeFileSync(textPath, payload.text, "utf8");
	const read = (path: string): string =>
		`$.NSString.stringWithContentsOfFileEncodingError(${JSON.stringify(path)}, $.NSUTF8StringEncoding, null)`;
	const script = [
		"ObjC.import('AppKit');",
		"var pb = $.NSPasteboard.generalPasteboard;",
		"pb.clearContents;",
		`pb.setStringForType(${read(htmlPath)}, $.NSPasteboardTypeHTML);`,
		`pb.setStringForType(${read(textPath)}, $.NSPasteboardTypeString);`,
	].join("\n");
	writeFileSync(scriptPath, script, "utf8");
	return await run("osascript", ["-l", "JavaScript", scriptPath]);
}

/**
 * Windows: a `DataObject` carrying CF_HTML and Unicode text.
 *
 * `-STA` is not optional. The clipboard is a single-threaded-apartment API and
 * PowerShell's default MTA host silently fails the call, which is the kind of
 * failure that reads as "copy did nothing" rather than as an error.
 */
async function copyRichWin32(payload: RichPayload, dir: string): Promise<boolean> {
	const htmlPath = join(dir, "clip.html");
	const textPath = join(dir, "clip.txt");
	const scriptPath = join(dir, "clip.ps1");
	writeFileSync(htmlPath, wrapCfHtml(payload.html), "utf8");
	writeFileSync(textPath, payload.text, "utf8");
	const script = [
		"Add-Type -AssemblyName System.Windows.Forms",
		`$html = [System.IO.File]::ReadAllText(${JSON.stringify(htmlPath)})`,
		`$text = [System.IO.File]::ReadAllText(${JSON.stringify(textPath)})`,
		"$data = New-Object System.Windows.Forms.DataObject",
		"$data.SetData([System.Windows.Forms.DataFormats]::Html, $html)",
		"$data.SetData([System.Windows.Forms.DataFormats]::UnicodeText, $text)",
		"[System.Windows.Forms.Clipboard]::SetDataObject($data, $true)",
	].join("\n");
	writeFileSync(scriptPath, script, "utf8");
	return await run("powershell", ["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File", scriptPath]);
}

/**
 * Copy `payload`, richly where the platform allows it.
 *
 * @returns which flavour reached the clipboard: `"rich"` when the receiving
 *   application can paste structure, `"text"` when it will get the markdown.
 *   The caller reports this, because "copied" meaning two different things
 *   depending on the machine is how a feature gets reported as broken.
 */
export async function copyRichToClipboard(payload: RichPayload): Promise<CopyFlavour> {
	const oversized = Buffer.byteLength(payload.html, "utf8") > MAX_RICH_BYTES;
	if (!oversized && (process.platform === "darwin" || process.platform === "win32")) {
		const dir = mkdtempSync(join(tmpdir(), "hoocode-clip-"));
		try {
			const copied =
				process.platform === "darwin" ? await copyRichDarwin(payload, dir) : await copyRichWin32(payload, dir);
			if (copied) return "rich";
		} catch {
			// Fall through to the plain copy: a rich copy that failed is not a
			// reason for the user to lose the text as well.
		} finally {
			rmSync(dir, { recursive: true, force: true });
		}
	}
	await copyToClipboard(payload.text);
	return "text";
}
