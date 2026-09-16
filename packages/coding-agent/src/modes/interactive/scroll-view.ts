/**
 * Reading back through the transcript.
 *
 * The TUI owns the mechanism — a pinned window over the line buffer, painted on
 * the alternate screen, described in `tui.ts` under `scrollOffset`. This file
 * owns the two things the TUI cannot know: which keys mean what, and what the
 * indicator on the bottom row should look like in the user's theme.
 *
 * ## Two scopes, and why the split is where it is
 *
 * At the prompt only four keys are live — page up, page down, and the two ends.
 * They are the keys every pager already taught, and none of them is a key the
 * prompt has any use for, so nothing is taken away to make room. Anything more
 * would be: the arrows walk prompt history, and a reader who loses that to
 * scrolling has traded one surprise for another.
 *
 * Once the view is pinned it captures keys, exactly as a picker does, and the
 * fuller set comes alive — line steps on the arrows, escape back to live. The
 * prompt is not in front of you at that point, so its keys are free.
 *
 * The capture is an input listener rather than editor actions because it has to
 * beat the focused editor to the arrows: an editor action runs only after the
 * editor has already decided the key was history navigation.
 *
 * ## Leaving
 *
 * Anything that is not a scroll key un-pins the view and then does what it
 * always did. That is one rule rather than a list of exceptions, and it is the
 * one that matches what people actually do — you stop reading by starting to
 * type, not by remembering to press escape first. Without it, typing into a
 * pinned view would echo into a prompt that is scrolled off-screen, which is
 * the exact class of "where did my keystroke go" this whole change exists to
 * remove.
 */

import {
	type Component,
	type Container,
	type EditorComponent,
	isKeyRelease,
	type Keybinding,
	type ScrollStatus,
	type TUI,
	truncateToWidth,
	visibleWidth,
} from "@kolisachint/hoocode-tui";
import type { KeybindingsManager } from "../../core/keybindings.js";
import { keyText } from "./components/keybinding-hints.js";
import { theme } from "./theme/theme.js";

/**
 * Pin the view to the previous or next thing the user said.
 *
 * The landmarks people navigate a session by are their own messages — "where
 * did I ask about the renderer" — and they are sparse enough that a few presses
 * cross a long transcript. Agent replies and tool blocks are deliberately not
 * stops: in a tool-heavy session they are dense enough that the key degrades
 * into paging with extra steps.
 *
 * The rows come from the render memo rather than from a re-render: a message's
 * offset inside the chat container, plus that container's offset at the root,
 * is its absolute row in the buffer the viewport windows over. Both are already
 * cached from the last frame, so a jump is a walk over line-array lengths.
 */
export function jumpToUserMessage(
	ui: TUI,
	chat: Container,
	direction: "previous" | "next",
	isUserMessage: (child: Component) => boolean,
): boolean {
	const width = ui.terminal.columns;
	const rootOffsets = ui.childRowOffsets(width);
	const chatOffsets = chat.childRowOffsets(width);
	// No memo yet (nothing rendered at this width): there is nothing to point at.
	if (!rootOffsets || !chatOffsets) return false;

	const chatIndex = ui.children.indexOf(chat);
	if (chatIndex === -1) return false;
	const base = rootOffsets[chatIndex];

	const rows: number[] = [];
	for (let i = 0; i < chat.children.length; i++) {
		if (isUserMessage(chat.children[i])) rows.push(base + chatOffsets[i]);
	}
	if (rows.length === 0) return false;

	// Where the eye is now: the top of the pinned window, or the bottom of the
	// transcript when live. Live counts as "below the last message", so the
	// first press of `previous` lands on the most recent one.
	const position = ui.getScrollPosition();
	const here = position ? position.top : Number.MAX_SAFE_INTEGER;

	const target =
		direction === "previous" ? [...rows].reverse().find((row) => row < here) : rows.find((row) => row > here);
	if (target === undefined) return false;

	ui.scrollToRow(target);
	return true;
}

/** What the pinned view answers to, in the order the keys are tried. */
const PINNED_BINDINGS: Array<[Keybinding, (ui: TUI) => void]> = [
	// Before the ends, so that a binding set where they share a key still gets
	// out rather than jumping somewhere.
	["app.scroll.exit", (ui) => void ui.scrollToLive()],
	["app.scroll.top", (ui) => void ui.scrollToTop()],
	["app.scroll.bottom", (ui) => void ui.scrollToLive()],
	["app.scroll.pageUp", (ui) => void ui.scrollByPages(-1)],
	["app.scroll.pageDown", (ui) => void ui.scrollByPages(1)],
	["app.scroll.lineUp", (ui) => void ui.scrollByLines(-1)],
	["app.scroll.lineDown", (ui) => void ui.scrollByLines(1)],
];

/**
 * Whether `data` is ordinary typed text rather than a chord.
 *
 * The query line takes characters; everything else is a key. Control bytes and
 * escape sequences are never text, which is the whole distinction.
 */
function isTypedText(data: string): boolean {
	if (data.length === 0) return false;
	for (let i = 0; i < data.length; i++) {
		const code = data.charCodeAt(i);
		if (code < 32 || code === 127) return false;
	}
	return true;
}

/** Pinned-view keys that need more than the TUI to answer them. */
const PINNED_TURN_BINDINGS = ["app.scroll.previousMessage", "app.scroll.nextMessage"] as const;

/**
 * The bottom row of a pinned view.
 *
 * It answers "where am I" first and "how do I get out" second, because the
 * first question is the one a reader has on every frame and the second is the
 * one they have once. The keys are read out of the live bindings rather than
 * written into the string, so a rebind is reflected here instead of quietly
 * turning the row into a lie.
 *
 * `atBottom` is never shown: reaching the bottom releases the pin, so a pinned
 * view is by definition somewhere above it.
 */
function formatStatus(status: ScrollStatus): string {
	// While a search is running the indicator *is* the query line. Position is
	// not what you are asking at that moment; "did it find anything" is.
	if (status.search) {
		const { query, count, index, typing } = status.search;
		const hits = query.length === 0 ? "" : count === 0 ? "  no matches" : `  ${index}/${count}`;
		// A block where the caret would be, so an empty query still looks like
		// something you are expected to type into.
		const caret = typing ? "\u2588" : "";
		const left = ` search: ${query}${caret}${hits}`;
		const keys = typing
			? `${keyText("tui.select.confirm")} keep · ${keyText("app.scroll.exit")} cancel`
			: `${keyText("app.scroll.searchNext")}/${keyText("app.scroll.searchPrevious")} step · ${keyText("app.scroll.exit")} done`;
		const gap = status.width - visibleWidth(left) - visibleWidth(keys) - 2;
		const body = gap >= 0 ? `${left}${" ".repeat(gap + 1)}${keys} ` : `${left} `;
		return theme.inverse(truncateToWidth(body, status.width, "", true));
	}

	const position = `${status.top}–${status.bottom} of ${status.total}`;
	const place = status.atTop ? "start of session" : `${Math.round((status.top / status.total) * 100)}%`;
	const left = `${position}  ${place}`;
	const keys = [
		`${keyText("app.scroll.lineUp")}/${keyText("app.scroll.lineDown")} line`,
		`${keyText("app.scroll.pageUp")}/${keyText("app.scroll.pageDown")} page`,
		`${keyText("app.scroll.top")} top`,
		`${keyText("app.scroll.exit")} live`,
	].join(" · ");

	// A space of margin each side, and the keys only when they fit whole: a
	// half-printed key list is worse than none, because a truncated chord reads
	// as a different chord.
	const gap = status.width - visibleWidth(left) - visibleWidth(keys) - 3;
	const body = gap >= 0 ? ` ${left}${" ".repeat(gap + 1)}${keys} ` : ` ${left} `;
	return theme.inverse(truncateToWidth(body, status.width, "", true));
}

/**
 * Wire scrolling into a running interactive mode.
 *
 * Takes the mode's own `KeybindingsManager` rather than reading the process-wide
 * one: the keys have to resolve the same way here as they do inside the editor a
 * line above, and a listener that silently matched nothing — which is what the
 * unconfigured global resolves every `app.*` id to — would hand the pinned view's
 * keys back to the prompt without saying so.
 *
 * Returns the teardown for the input listener, matching the other listeners the
 * mode installs; the editor actions live as long as the editor does.
 */
export function installScrollView(
	ui: TUI,
	editor: EditorComponent & { onAction(action: Keybinding, handler: () => void): void },
	keybindings: KeybindingsManager,
	turns: { chat: Container; isUserMessage: (child: Component) => boolean },
): () => void {
	ui.setScrollStatusFormatter(formatStatus);

	// The wheel is answered wherever it is turned, so the gate is here rather
	// than on the keys: with a picker or a login dialog on screen, the prompt is
	// not what the user is looking at, and pinning would take that surface's
	// arrow keys away from it.
	ui.canPinScroll = () => ui.focused === editor;

	// Live at the prompt. Paging down while already live returns false and does
	// nothing, which is what page-down at the bottom of a transcript should do.
	editor.onAction("app.scroll.pageUp", () => void ui.scrollByPages(-1));
	editor.onAction("app.scroll.pageDown", () => void ui.scrollByPages(1));
	editor.onAction("app.scroll.top", () => void ui.scrollToTop());
	editor.onAction("app.scroll.bottom", () => void ui.scrollToLive());
	editor.onAction("app.scroll.previousMessage", () => {
		jumpToUserMessage(ui, turns.chat, "previous", turns.isUserMessage);
	});
	editor.onAction("app.scroll.nextMessage", () => {
		jumpToUserMessage(ui, turns.chat, "next", turns.isUserMessage);
	});

	/** The query being typed, or null when no search is being composed. */
	let typing: string | null = null;

	const openSearch = (): void => {
		typing = "";
		ui.setScrollSearch("", { typing: true });
	};

	editor.onAction("app.scroll.search", () => {
		// From the prompt this has to pin first, at the bottom, so the backwards
		// search starts from the newest thing and works back.
		if (!ui.scrollPinned) ui.scrollToRow(Number.MAX_SAFE_INTEGER);
		if (!ui.scrollPinned) return;
		openSearch();
	});

	return ui.addInputListener((data) => {
		if (!ui.scrollPinned) return undefined;

		// A key coming back up is not a decision to stop reading.
		if (isKeyRelease(data)) return { consume: true };

		// The query line owns every printable character while it is open, so it
		// is asked before any key that a letter could also mean.
		if (typing !== null) {
			if (keybindings.matches(data, "tui.select.confirm")) {
				typing = null;
				ui.commitScrollSearch();
				return { consume: true };
			}
			if (keybindings.matches(data, "app.scroll.exit")) {
				typing = null;
				ui.clearScrollSearch();
				return { consume: true };
			}
			if (keybindings.matches(data, "tui.editor.deleteCharBackward")) {
				typing = typing.slice(0, -1);
				ui.setScrollSearch(typing, { typing: true });
				return { consume: true };
			}
			if (isTypedText(data)) {
				typing += data;
				ui.setScrollSearch(typing, { typing: true });
				return { consume: true };
			}
			// A chord while composing: commit the query and let the chord through
			// to its usual meaning below.
			typing = null;
			ui.commitScrollSearch();
		}

		if (keybindings.matches(data, "app.scroll.search") || keybindings.matches(data, "app.scroll.searchInView")) {
			openSearch();
			return { consume: true };
		}
		if (ui.scrollSearchActive) {
			if (keybindings.matches(data, "app.scroll.searchNext")) {
				ui.scrollSearchStep(-1);
				return { consume: true };
			}
			if (keybindings.matches(data, "app.scroll.searchPrevious")) {
				ui.scrollSearchStep(1);
				return { consume: true };
			}
			// Escape with a committed search drops the search but keeps the view,
			// so a second press is what leaves. One escape, one thing undone.
			if (keybindings.matches(data, "app.scroll.exit")) {
				ui.clearScrollSearch();
				return { consume: true };
			}
		}

		for (const [binding, act] of PINNED_BINDINGS) {
			if (!keybindings.matches(data, binding)) continue;
			act(ui);
			return { consume: true };
		}

		for (const binding of PINNED_TURN_BINDINGS) {
			if (!keybindings.matches(data, binding)) continue;
			const direction = binding === "app.scroll.previousMessage" ? "previous" : "next";
			jumpToUserMessage(ui, turns.chat, direction, turns.isUserMessage);
			return { consume: true };
		}

		// Anything else: back to live, then let the key do its usual job there.
		ui.scrollToLive();
		return undefined;
	});
}
