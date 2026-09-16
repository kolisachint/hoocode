/**
 * The chrome dial and the two things that move it on their own.
 *
 * `resolveChrome` is the whole policy, so most of this is a truth table over
 * it — which is the point of having put the policy in one pure function. The
 * controller tests are about the other half of the design: that pushing state
 * in on a hot path is cheap, and that nothing is touched when nothing moved.
 */

import { Slot } from "@kolisachint/hoocode-tui";
import { describe, expect, it } from "vitest";
import {
	CHROME_DENSITIES,
	type ChromeDensity,
	ChromeLayoutController,
	isChromeDensity,
	resolveChrome,
} from "../src/modes/interactive/chrome-layout.js";

const QUIET = { autocompleteOpen: false, agentStreaming: false };

describe("resolveChrome", () => {
	it("gives everything its rows at full", () => {
		expect(resolveChrome({ density: "full", ...QUIET })).toEqual({ footer: "full", tasks: "full" });
	});

	it("drops the ledger and shortens the footer at compact", () => {
		expect(resolveChrome({ density: "compact", ...QUIET })).toEqual({ footer: "line", tasks: "hidden" });
	});

	it("keeps neither at bare", () => {
		expect(resolveChrome({ density: "bare", ...QUIET })).toEqual({ footer: "hidden", tasks: "hidden" });
	});

	it("lends the footer's rows to an open completion list, at every stop", () => {
		for (const density of CHROME_DENSITIES) {
			const layout = resolveChrome({ density, autocompleteOpen: true, agentStreaming: false });
			expect(layout.footer, density).toBe("hidden");
		}
	});

	it("trades the ledger's rows for its counts while a turn runs", () => {
		const layout = resolveChrome({ density: "full", autocompleteOpen: false, agentStreaming: true });
		expect(layout.tasks).toBe("summary");
	});

	it("does not resurrect the ledger mid-turn at a stop that hid it", () => {
		// The transient inputs may take rows away; they must never hand back rows
		// the dial was asked to give up.
		for (const density of ["compact", "bare"] as ChromeDensity[]) {
			const layout = resolveChrome({ density, autocompleteOpen: false, agentStreaming: true });
			expect(layout.tasks, density).toBe("hidden");
		}
	});

	it("never hides the prompt, whatever is set", () => {
		// Asserted by construction: the prompt has no entry in the layout, so no
		// combination of inputs can reach a screen you can type into and not see.
		for (const density of CHROME_DENSITIES) {
			for (const autocompleteOpen of [false, true]) {
				for (const agentStreaming of [false, true]) {
					const layout = resolveChrome({ density, autocompleteOpen, agentStreaming });
					expect(Object.keys(layout).sort()).toEqual(["footer", "tasks"]);
				}
			}
		}
	});
});

describe("isChromeDensity", () => {
	it("accepts the stops and nothing else", () => {
		for (const density of CHROME_DENSITIES) expect(isChromeDensity(density)).toBe(true);
		for (const other of ["", "hidden", "FULL", null, undefined, 2]) expect(isChromeDensity(other)).toBe(false);
	});
});

/** A controller over throwaway slots, recording what it was told to do. */
function setup(density: ChromeDensity = "full") {
	const footerSlot = new Slot({ invalidate() {}, render: () => ["footer"] });
	const tasksSlot = new Slot({ invalidate() {}, render: () => ["tasks"] });
	const footerDensities: string[] = [];
	const taskDensities: string[] = [];
	const controller = new ChromeLayoutController(
		{
			footerSlot,
			tasksSlot,
			setFooterDensity: (d) => footerDensities.push(d),
			setTasksDensity: (d) => taskDensities.push(d),
		},
		density,
	);
	controller.apply();
	return { controller, footerSlot, tasksSlot, footerDensities, taskDensities };
}

describe("ChromeLayoutController", () => {
	it("moves the slots to match the stop", () => {
		const { controller, footerSlot, tasksSlot } = setup();
		expect([footerSlot.visible, tasksSlot.visible]).toEqual([true, true]);

		controller.setDensity("compact");
		expect([footerSlot.visible, tasksSlot.visible]).toEqual([true, false]);

		controller.setDensity("bare");
		expect([footerSlot.visible, tasksSlot.visible]).toEqual([false, false]);
	});

	it("steps and wraps like every other dial", () => {
		const { controller } = setup();
		expect(controller.cycleDensity("forward")).toBe("compact");
		expect(controller.cycleDensity("forward")).toBe("bare");
		expect(controller.cycleDensity("forward")).toBe("full");
		expect(controller.cycleDensity("backward")).toBe("bare");
	});

	it("reports nothing to do when nothing moved", () => {
		// The callers push state in on keystrokes and stream events, and use this
		// to decide whether a frame is owed. Answering "changed" for an unchanged
		// layout would be a render per keystroke.
		const { controller } = setup();
		expect(controller.setAutocompleteOpen(true)).toBe(true);
		expect(controller.setAutocompleteOpen(true)).toBe(false);
		expect(controller.setDensity("full")).toBe(false);
		expect(controller.setAgentStreaming(false)).toBe(false);
	});

	it("sets a slot's density before showing it", () => {
		// A slot coming back should render at the size it is meant to be, not at
		// whatever size it had when it went away.
		const { controller, footerDensities } = setup();
		controller.setDensity("compact");
		expect(footerDensities.at(-1)).toBe("line");
		controller.setDensity("full");
		expect(footerDensities.at(-1)).toBe("full");
	});

	it("gives the footer back when the completion list closes", () => {
		const { controller, footerSlot } = setup();
		controller.setAutocompleteOpen(true);
		expect(footerSlot.visible).toBe(false);
		controller.setAutocompleteOpen(false);
		expect(footerSlot.visible).toBe(true);
	});

	it("restores the stop the dial was on, not the one the transient input implied", () => {
		const { controller, footerSlot } = setup("compact");
		controller.setAutocompleteOpen(true);
		controller.setAutocompleteOpen(false);
		expect(footerSlot.visible).toBe(true);
		expect(controller.layout.footer).toBe("line");
	});
});

describe("Slot", () => {
	it("returns the very same array every time it is hidden", () => {
		// The entire cost argument rests on this: `Container.render` and the root's
		// flat cache decide "did this change" by array identity, so a fresh []
		// would report a change on every frame for something not even drawn.
		const slot = new Slot({ invalidate() {}, render: () => ["x"] });
		slot.setVisible(false);
		expect(slot.render(80)).toBe(slot.render(80));
		expect(slot.render(80)).toEqual([]);
	});

	it("does not render its child while hidden", () => {
		let renders = 0;
		const slot = new Slot({
			invalidate() {},
			render: () => {
				renders++;
				return ["x"];
			},
		});
		slot.render(80);
		expect(renders).toBe(1);
		slot.setVisible(false);
		slot.render(80);
		slot.render(80);
		expect(renders, "a hidden slot is free, not merely invisible").toBe(1);
	});

	it("reports whether a visibility change was real", () => {
		const slot = new Slot({ invalidate() {}, render: () => ["x"] });
		expect(slot.setVisible(true)).toBe(false);
		expect(slot.setVisible(false)).toBe(true);
		expect(slot.setVisible(false)).toBe(false);
	});

	it("swaps its occupant without moving itself", () => {
		const first = { invalidate() {}, render: () => ["first"] };
		const second = { invalidate() {}, render: () => ["second"] };
		const slot = new Slot(first);
		expect(slot.render(80)).toEqual(["first"]);
		expect(slot.setChild(second)).toBe(true);
		expect(slot.render(80)).toEqual(["second"]);
		expect(slot.setChild(second)).toBe(false);
	});
});
