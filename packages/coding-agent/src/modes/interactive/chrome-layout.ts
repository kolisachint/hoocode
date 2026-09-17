/**
 * How much of the screen the chrome is allowed to have.
 *
 * ## The problem
 *
 * Nine things hang off the TUI root, and seven of them sit *below* the
 * transcript: queued messages, status rows, two widget containers, the task
 * ledger, the prompt, and the footer. On a thirty-row terminal that routinely
 * comes to eight or twelve rows — a third of the screen spent on furniture,
 * taken from the conversation. Each of those components also decided its own
 * visibility, in its own way, in its own file: the ledger self-hides when it has
 * no tasks, the footer always draws, the header is cleared by whoever remembers
 * to. There was no one place to ask "what is on screen right now", and so no
 * place to change it.
 *
 * ## The shape
 *
 * One dial and one table. `ChromeDensity` is the dial — an ordered set of stops
 * the way the other six are, so `alt+z` steps it and `shift+alt+z` steps back,
 * and the answer is always reachable in one more press. `resolveChrome` is the
 * table: a pure function from (dial, what is happening) to what each slot
 * shows. Adding a stop, or a new reason to hide something, is an edit to that
 * one function rather than a hunt through seven components.
 *
 * The prompt is deliberately not in the table. Everything else can go, but a
 * screen you can type into and not see is the worst possible failure here, and
 * a dial that can reach that state will eventually be left in it.
 *
 * ## Speed
 *
 * Two rules, both about not doing work:
 *
 * - A hidden slot returns one frozen array and never renders its child (see
 *   `Slot`). Hiding the footer does not make it cheaper to draw; it makes it
 *   free.
 * - `apply` compares the resolved layout against the last one and returns
 *   whether anything moved. The inputs change on events that fire *constantly* —
 *   an autocomplete opens on a keystroke, streaming flips many times a turn —
 *   so the callers push state in on every one of those and this decides whether
 *   a frame is owed. Recomputing a layout is a few comparisons; re-rendering
 *   because you did not check is a frame.
 */

import type { Slot } from "@kolisachint/hoocode-tui";
import { CHROME_DENSITIES, type ChromeDensity } from "../../core/chrome-density.js";

export { CHROME_DENSITIES, type ChromeDensity, isChromeDensity } from "../../core/chrome-density.js";

/**
 * Below this many rows the dial starts at `compact` rather than `full`.
 *
 * A starting point, never a correction: it is read once, when nothing is stored
 * yet, so a small terminal opens sensibly and the dial still does exactly what
 * it is told from then on. Re-deciding this on resize would move the layout
 * under someone dragging a pane divider, which is the kind of thing that makes
 * a UI feel like it is arguing with you.
 */
export const SMALL_TERMINAL_ROWS = 25;

export interface ChromeInputs {
	density: ChromeDensity;
	/** The prompt's completion list is open and wants the room. */
	autocompleteOpen: boolean;
	/** The agent is mid-turn, so transcript rows are worth more than ledger rows. */
	agentStreaming: boolean;
}

export interface ChromeLayout {
	/** `full` is every row it has; `line` is the one-row vitals strip. */
	footer: "full" | "line" | "hidden";
	/** `summary` is the ledger's header strip alone — the counts, no rows. */
	tasks: "full" | "summary" | "hidden";
}

/**
 * The whole policy, in one place.
 *
 * Read it as: the dial says what you asked for, and the two transient inputs
 * say what is happening. Where they disagree the transient one wins, because it
 * is the one that ends on its own — an autocomplete closes, a turn settles, and
 * the dial's answer comes back without anyone pressing anything.
 */
export function resolveChrome({ density, autocompleteOpen, agentStreaming }: ChromeInputs): ChromeLayout {
	// The completion list is the reason the prompt grew; the footer is the
	// nearest thing with rows to give. It comes straight back on dismissal, so
	// this can never strand anyone somewhere they have to key their way out of.
	const footer: ChromeLayout["footer"] = autocompleteOpen
		? "hidden"
		: density === "full"
			? "full"
			: density === "compact"
				? "line"
				: "hidden";

	// `bare` is the stop that means nothing below the prompt, so the ledger goes
	// with the footer. `compact` keeps the counts and gives up the rows, which is
	// the same trade the dial is already making for the footer one line up — a
	// stop where the footer shrinks and the ledger vanishes outright was the odd
	// one out, and it made the middle stop feel like a cliff rather than a step.
	if (density === "bare") return { footer, tasks: "hidden" };
	if (density === "compact") return { footer, tasks: "summary" };

	// Mid-turn the ledger keeps its counts and gives up its rows. The counts are
	// the part you watch; the rows are the part you read afterwards.
	return { footer, tasks: agentStreaming ? "summary" : "full" };
}

/** What the controller needs of the footer and the ledger, and nothing more. */
export interface ChromeSurfaces {
	footerSlot: Slot;
	tasksSlot: Slot;
	setFooterDensity(density: "full" | "line"): void;
	setTasksDensity(density: "full" | "summary"): void;
}

/**
 * Holds the dial, takes the transient inputs, and moves the slots.
 *
 * Every setter returns nothing and instead reports through `changed`, so a
 * caller that pushes state in on a hot path (a keystroke, a stream event) can
 * ask once whether a render is owed.
 */
export class ChromeLayoutController {
	private inputs: ChromeInputs;
	private applied: ChromeLayout | null = null;

	constructor(
		private readonly surfaces: ChromeSurfaces,
		density: ChromeDensity,
	) {
		this.inputs = { density, autocompleteOpen: false, agentStreaming: false };
	}

	get density(): ChromeDensity {
		return this.inputs.density;
	}

	/** The layout currently on screen, for tests and for the hint strip. */
	get layout(): ChromeLayout {
		return this.applied ?? resolveChrome(this.inputs);
	}

	setDensity(density: ChromeDensity): boolean {
		if (this.inputs.density === density) return false;
		this.inputs = { ...this.inputs, density };
		return this.apply();
	}

	/** Step the dial, wrapping, the way every other dial in the app steps. */
	cycleDensity(direction: "forward" | "backward"): ChromeDensity {
		const at = CHROME_DENSITIES.indexOf(this.inputs.density);
		const step = direction === "forward" ? 1 : -1;
		const next = CHROME_DENSITIES[(at + step + CHROME_DENSITIES.length) % CHROME_DENSITIES.length];
		this.setDensity(next);
		return next;
	}

	setAutocompleteOpen(open: boolean): boolean {
		if (this.inputs.autocompleteOpen === open) return false;
		this.inputs = { ...this.inputs, autocompleteOpen: open };
		return this.apply();
	}

	setAgentStreaming(streaming: boolean): boolean {
		if (this.inputs.agentStreaming === streaming) return false;
		this.inputs = { ...this.inputs, agentStreaming: streaming };
		return this.apply();
	}

	/** Push the current layout onto the slots; true when anything moved. */
	apply(): boolean {
		const next = resolveChrome(this.inputs);
		const previous = this.applied;
		if (previous && previous.footer === next.footer && previous.tasks === next.tasks) return false;
		this.applied = next;

		// Density before visibility: a slot that is about to be shown should
		// render at the size it is meant to be, not at the last size it had.
		if (next.footer !== "hidden") this.surfaces.setFooterDensity(next.footer);
		if (next.tasks !== "hidden") this.surfaces.setTasksDensity(next.tasks);
		this.surfaces.footerSlot.setVisible(next.footer !== "hidden");
		this.surfaces.tasksSlot.setVisible(next.tasks !== "hidden");
		return true;
	}
}
