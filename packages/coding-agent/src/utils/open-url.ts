import { spawn } from "node:child_process";

/**
 * Schemes a click may hand to the desktop.
 *
 * An allowlist rather than a denylist, because the desktop's URL handlers are
 * not a closed set: a machine can have a handler registered for very nearly
 * anything, and a transcript is full of text the model wrote. Anything outside
 * this list is a URL the user can still select and open themselves — it is just
 * not something one click does on their behalf.
 */
const OPENABLE_SCHEMES = new Set(["http:", "https:", "mailto:"]);

/**
 * Whether `url` is something we are willing to hand to the desktop.
 *
 * Exported for the same reason it exists: the check has to be identical
 * everywhere, and a second hand-written copy of it is how an allowlist stops
 * being one.
 */
export function isOpenableUrl(url: string): boolean {
	try {
		return OPENABLE_SCHEMES.has(new URL(url).protocol);
	} catch {
		return false;
	}
}

/**
 * Open a URL in the desktop's default handler.
 *
 * No shell, anywhere. The URL comes from a line of the transcript, which is
 * text a model wrote, and `exec("open " + url)` hands that to `sh` — a URL with
 * a quote and a semicolon in it would be a command. Every platform gets the
 * argument as an argument. Windows is the awkward one: `start` is a `cmd`
 * builtin rather than a program, and its first quoted argument is the window
 * title, which is why the empty string is there.
 *
 * Detached and with stdio ignored: the handler is a GUI program that outlives
 * this process, and a live pipe to it would hold the TUI's terminal.
 */
export function openUrl(url: string): void {
	if (!isOpenableUrl(url)) return;
	const [command, args] =
		process.platform === "darwin"
			? ["open", [url]]
			: process.platform === "win32"
				? ["cmd", ["/c", "start", "", url]]
				: ["xdg-open", [url]];
	try {
		const child = spawn(command, args as string[], { detached: true, stdio: "ignore" });
		// A browser that is not installed must not take the session down with it.
		child.on("error", () => {});
		child.unref();
	} catch {
		// Nothing to say: the URL is on screen and selectable either way.
	}
}
