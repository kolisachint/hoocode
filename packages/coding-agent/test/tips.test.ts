import { describe, expect, it } from "vitest";
import { renderTip, STAR_NUDGE_LIMIT, TIPS, type Tip, TipRotation } from "../src/modes/interactive/tips.js";
import {
	DEFAULT_COOLDOWN_MS,
	DEFAULT_GRACE_MS,
	DEFAULT_IDLE_DELAY_MS,
	DEFAULT_STREAMING_DELAY_MS,
	TipsController,
	type TipsControllerOptions,
} from "../src/modes/interactive/tips-controller.js";

/** A rotation wired to in-memory stores, so a test can watch what it remembers. */
function makeRotation(options: { tips?: readonly Tip[]; seen?: string[]; starNudges?: number } = {}) {
	const seen = new Set(options.seen ?? []);
	let starNudges = options.starNudges ?? 0;
	const rotation = new TipRotation({
		tips: options.tips,
		seen: () => [...seen],
		markSeen: (id) => seen.add(id),
		starNudgeCount: () => starNudges,
		markStarNudge: () => {
			starNudges += 1;
		},
	});
	return { rotation, seen, starNudges: () => starNudges };
}

describe("TIPS content", () => {
	it("has no duplicate ids", () => {
		// Ids are the key "already seen" is stored against, so a duplicate would
		// silently suppress one of the two tips forever.
		const ids = TIPS.map((tip) => tip.id);
		expect(new Set(ids).size).toBe(ids.length);
	});

	it("renders every tip without throwing", () => {
		// Several tips resolve keybindings at display time. A tip that throws
		// would take down a timer callback in the middle of a session.
		for (const tip of TIPS) {
			const rendered = renderTip(tip);
			expect(rendered.title.length).toBeGreaterThan(0);
			expect(Array.isArray(rendered.body)).toBe(true);
		}
	});

	it("keeps every tip within the band's row budget", () => {
		// The band shows at most three rows of body and drops the rest, so a
		// four-row tip is a tip with an invisible last line.
		for (const tip of TIPS) {
			expect(renderTip(tip).body.length).toBeLessThanOrEqual(3);
		}
	});
});

describe("TipRotation", () => {
	const a: Tip = { id: "a", title: "A" };
	const b: Tip = { id: "b", title: "B" };
	const c: Tip = { id: "c", title: "C" };

	it("walks unseen tips in declaration order", () => {
		const { rotation } = makeRotation({ tips: [a, b, c] });
		expect(rotation.next("idle")?.id).toBe("a");
		expect(rotation.next("idle")?.id).toBe("b");
		expect(rotation.next("idle")?.id).toBe("c");
	});

	it("skips what a previous session already showed", () => {
		const { rotation } = makeRotation({ tips: [a, b, c], seen: ["a", "b"] });
		expect(rotation.next("idle")?.id).toBe("c");
	});

	it("records each tip it shows", () => {
		const { rotation, seen } = makeRotation({ tips: [a, b] });
		rotation.next("idle");
		expect([...seen]).toEqual(["a"]);
	});

	it("starts over once everything has been seen", () => {
		// Silence would read as the feature having broken; a session long enough
		// to exhaust the list has earned a repeat.
		const { rotation } = makeRotation({ tips: [a, b], seen: ["a", "b"] });
		expect(rotation.next("idle")?.id).toBe("a");
		expect(rotation.next("idle")?.id).toBe("b");
		expect(rotation.next("idle")).toBeUndefined();
	});

	it("honours a tip's moment", () => {
		const streamOnly: Tip = { id: "s", title: "S", moments: ["streaming"] };
		const idleOnly: Tip = { id: "i", title: "I", moments: ["idle"] };
		const { rotation } = makeRotation({ tips: [streamOnly, idleOnly] });
		expect(rotation.next("idle")?.id).toBe("i");
		expect(rotation.next("streaming")?.id).toBe("s");
	});

	it("does not ask for a star before it has been useful", () => {
		// The first thing a new user sees must not be an ask.
		const { rotation, starNudges } = makeRotation({ tips: [a, b, c] });
		rotation.next("idle");
		rotation.next("idle");
		rotation.next("idle");
		expect(starNudges()).toBe(0);
	});

	it("asks for a star once the cadence is reached, and only once a session", () => {
		const many = Array.from({ length: 20 }, (_, i) => ({ id: `t${i}`, title: `T${i}` }));
		const { rotation, starNudges } = makeRotation({ tips: many });

		const shown: string[] = [];
		for (let i = 0; i < 20; i++) {
			const tip = rotation.next("idle");
			if (tip) shown.push(tip.id);
		}

		expect(shown.filter((id) => id === "star")).toHaveLength(1);
		expect(starNudges()).toBe(1);
	});

	it("stops asking for a star once the lifetime cap is spent", () => {
		const many = Array.from({ length: 20 }, (_, i) => ({ id: `t${i}`, title: `T${i}` }));
		const { rotation, starNudges } = makeRotation({ tips: many, starNudges: STAR_NUDGE_LIMIT });

		for (let i = 0; i < 20; i++) rotation.next("idle");

		expect(starNudges()).toBe(STAR_NUDGE_LIMIT);
	});

	it("never nudges while streaming", () => {
		// The star ask is idle-only: interrupting a turn to ask for a favour is
		// the worst possible moment for it.
		const many = Array.from({ length: 20 }, (_, i) => ({ id: `t${i}`, title: `T${i}` }));
		const { rotation } = makeRotation({ tips: many });

		const shown: string[] = [];
		for (let i = 0; i < 20; i++) {
			const tip = rotation.next("streaming");
			if (tip) shown.push(tip.id);
		}
		expect(shown).not.toContain("star");
	});
});

/** A controller on a fake clock and fake timers, so no test waits on wall time. */
function makeController(overrides: Partial<TipsControllerOptions> = {}) {
	let now = 0;
	const pending = new Map<number, { fn: () => void; at: number }>();
	let nextHandle = 1;
	const shown: Tip[] = [];
	let enabled = true;
	let bandFree = true;

	const tips = Array.from({ length: 30 }, (_, i) => ({ id: `t${i}`, title: `T${i}` }));
	const { rotation } = makeRotation({ tips });

	const controller = new TipsController({
		isEnabled: () => enabled,
		bandIsFree: () => bandFree,
		show: (tip) => shown.push(tip),
		rotation,
		now: () => now,
		setTimer: (fn, ms) => {
			const handle = nextHandle++;
			pending.set(handle, { fn, at: now + ms });
			return handle;
		},
		clearTimer: (handle) => {
			pending.delete(handle as number);
		},
		...overrides,
	});

	/** Advance the clock and fire whatever was due. */
	const advance = (ms: number) => {
		now += ms;
		for (const [handle, timer] of [...pending]) {
			if (timer.at <= now) {
				pending.delete(handle);
				timer.fn();
			}
		}
	};

	return {
		controller,
		shown,
		advance,
		setEnabled: (value: boolean) => {
			enabled = value;
		},
		setBandFree: (value: boolean) => {
			bandFree = value;
		},
	};
}

describe("TipsController", () => {
	it("says nothing during the startup grace period", () => {
		// Startup already has a banner, a changelog and possibly warnings.
		const { controller, shown, advance } = makeController();
		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS);
		expect(shown).toHaveLength(0);
	});

	it("shows a tip once the prompt has been quiet for the idle delay", () => {
		const { controller, shown, advance } = makeController();
		advance(DEFAULT_GRACE_MS);
		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS);
		expect(shown).toHaveLength(1);
	});

	it("restarts the idle clock on every keystroke", () => {
		const { controller, shown, advance } = makeController();
		advance(DEFAULT_GRACE_MS);
		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS - 1_000);
		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS - 1_000);
		expect(shown).toHaveLength(0);
	});

	it("shows a tip during a long turn", () => {
		const { controller, shown, advance } = makeController();
		advance(DEFAULT_GRACE_MS);
		controller.onTurnStart();
		advance(DEFAULT_STREAMING_DELAY_MS);
		expect(shown).toHaveLength(1);
	});

	it("says nothing during a short turn", () => {
		const { controller, shown, advance } = makeController();
		advance(DEFAULT_GRACE_MS);
		controller.onTurnStart();
		advance(DEFAULT_STREAMING_DELAY_MS - 1_000);
		controller.onTurnEnd();
		advance(DEFAULT_STREAMING_DELAY_MS);
		expect(shown).toHaveLength(0);
	});

	it("never takes the band from a notification the user caused", () => {
		const { controller, shown, advance, setBandFree } = makeController();
		advance(DEFAULT_GRACE_MS);
		setBandFree(false);
		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS);
		expect(shown).toHaveLength(0);
	});

	it("does not pounce the moment the band frees up", () => {
		// A refused offer is dropped, not retried: retrying is how "the band is
		// busy" turns into a tip that lands the instant the user's own
		// notification fades.
		const { controller, shown, advance, setBandFree } = makeController();
		advance(DEFAULT_GRACE_MS);
		setBandFree(false);
		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS);
		setBandFree(true);
		advance(1_000);
		expect(shown).toHaveLength(0);
	});

	it("respects the cooldown between tips", () => {
		const { controller, shown, advance } = makeController();
		advance(DEFAULT_GRACE_MS);
		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS);
		expect(shown).toHaveLength(1);

		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS);
		expect(shown).toHaveLength(1);

		controller.onActivity();
		advance(DEFAULT_COOLDOWN_MS);
		expect(shown).toHaveLength(2);
	});

	it("goes quiet as soon as the setting is turned off", () => {
		const { controller, shown, advance, setEnabled } = makeController();
		advance(DEFAULT_GRACE_MS);
		setEnabled(false);
		controller.onActivity();
		advance(DEFAULT_IDLE_DELAY_MS);
		expect(shown).toHaveLength(0);
	});

	it("shows nothing after stop()", () => {
		const { controller, shown, advance } = makeController();
		advance(DEFAULT_GRACE_MS);
		controller.onActivity();
		controller.stop();
		advance(DEFAULT_IDLE_DELAY_MS);
		expect(shown).toHaveLength(0);
	});
});
