/**
 * hoocode's own canvas: `.agents/extensions/arrow-key-games`.
 *
 * It is committed to this repository, which is the reason this file exists. A
 * canvas has no passive half — everything about it comes from running its code —
 * so a committed one with no test is a file that rots silently: it would keep
 * being listed by `/canvas`, keep being openable, and keep serving whatever it
 * served the day it stopped working.
 *
 * It was built by driving `/new-canvas <description>` and then iterating with
 * `reload_canvas`, which is what §13 is about, so it doubles as the standing
 * proof that a canvas authored that way survives contact with the real runner.
 *
 * What is asserted is the contract, not the gameplay: the declarations the agent
 * sees, the token gate, and the turn rules of Duel — the part that lets a person
 * on a keyboard and a model on a tool call share one board. Snake's tick and the
 * maze's carving are exercised only far enough to prove they run.
 */

import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CANVAS_ENTRY_FILE } from "../src/core/canvas/discovery.js";
import { type CanvasExtensionProcess, spawnCanvasExtension } from "../src/core/canvas/runner.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const EXTENSION_ID = "arrow-key-games";
const ENTRY = path.resolve(__dirname, "..", "..", "..", ".agents", "extensions", EXTENSION_ID, CANVAS_ENTRY_FILE);

interface Snapshot {
	game: string;
	cols: number;
	rows: number;
	grid: string[];
	status: string;
	over: boolean;
	duel: { turn: string | null; you: number; model: number; coinsLeft: number; last: string } | null;
}

describe("the arrow-key games canvas", () => {
	let running: CanvasExtensionProcess | undefined;

	afterEach(async () => {
		await running?.terminate();
		running = undefined;
	});

	const params = { sessionId: "s1", extensionId: EXTENSION_ID, canvasId: EXTENSION_ID, instanceId: "i1" };

	async function start() {
		running = spawnCanvasExtension({
			extensionId: EXTENSION_ID,
			entry: ENTRY,
			runtime: canvasTestRuntime(),
			cwd: path.dirname(ENTRY),
			requestTimeoutMs: { "canvas.open": 20_000, "canvas.action.invoke": 20_000 },
		});
		const ready = await running.ready;
		const opened = (await running.open(params)) as { url: string; title: string };
		// The runner is typed against the wire protocol, which knows only JSON — the
		// shape of an action's result is the extension's business, so reading it as
		// a Snapshot is this test's assertion about its own canvas.
		const act = (actionName: string, input?: unknown) =>
			running?.invokeAction({ ...params, actionName, input: input as never }) as unknown as Promise<Snapshot>;
		return { ready, opened, act };
	}

	/**
	 * Assert a refusal by its code as well as its wording.
	 *
	 * The code is the part that has to be stable: `CanvasError.code` survives the
	 * process boundary as `CanvasCallError.code`, and it is what tells the model
	 * whether to wait or to pick another direction. The message is for the person.
	 * Driving the runner directly is why they are separate here — the agent tool is
	 * what folds the code into the message.
	 */
	async function expectRefusal(call: Promise<unknown>, code: string, message: RegExp) {
		const error = await call.then(
			() => undefined,
			(cause: unknown) => cause as Error & { code?: string },
		);
		expect(error, `expected a refusal with code "${code}"`).toBeDefined();
		expect(error?.code).toBe(code);
		expect(error?.message).toMatch(message);
	}

	/** Where a piece is on the board, or undefined. `[column, row]`, row 0 at the top. */
	function locate(grid: string[], glyph: string): [number, number] | undefined {
		for (let row = 0; row < grid.length; row++) {
			const column = (grid[row] as string).indexOf(glyph);
			if (column >= 0) return [column, row];
		}
		return undefined;
	}

	it("declares itself and the actions the agent drives it through", async () => {
		const { ready } = await start();

		expect(ready.canvases).toHaveLength(1);
		// The id is what an open instance is bound to, so a rename here would drop
		// the canvas out from under anyone watching it on the next reload.
		expect(ready.canvases[0]?.id).toBe(EXTENSION_ID);
		expect(ready.canvases[0]?.actions?.map((action) => action.name).sort()).toEqual([
			"await_turn",
			"get_state",
			"press_arrow",
			"restart_game",
			"select_game",
			"take_turn",
		]);
	});

	it("serves a token-gated page, and everything the page loads is behind the same gate", async () => {
		const { opened } = await start();
		expect(opened.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/\?token=/);

		const page = await fetch(opened.url);
		expect(page.status).toBe(200);
		const html = await page.text();

		const bare = new URL(opened.url);
		bare.searchParams.delete("token");
		expect((await fetch(bare)).status).toBe(403);

		// The regression this pins: the page's own <link> and <script> once pointed
		// at /app.css and /app.js with no token, so every asset 403'd and the page
		// rendered nothing. A curl check that appends the token by hand cannot see
		// that; only following the page's own urls can.
		const assets = [...html.matchAll(/(?:href|src)="(\/[^"]+)"/g)].map((match) => match[1] as string);
		expect(assets.length).toBeGreaterThan(0);
		for (const asset of assets) {
			expect((await fetch(new URL(asset, bare.origin))).status, `${asset} should load`).toBe(200);
		}
	});

	it("starts snake stopped, so it is not dead before the person touches the keyboard", async () => {
		const { act } = await start();

		expect((await act("select_game", { game: "snake" })).status).toContain("Press an arrow key");
		await new Promise((resolve) => setTimeout(resolve, 600));
		// Still where it started: without this the snake set off on open and was
		// against the far wall in about a second.
		expect((await act("get_state")).status).toContain("Press an arrow key");

		await act("press_arrow", { direction: "up" });
		await new Promise((resolve) => setTimeout(resolve, 400));
		const moving = await act("get_state");
		expect(moving.status).toBe("Playing");
		expect(locate(moving.grid, "S")).toBeDefined();
	});

	it("carves a maze that is connected, so every gem can actually be reached", async () => {
		const { act } = await start();
		const maze = await act("select_game", { game: "maze" });

		const open = new Set<string>();
		const gems = new Set<string>();
		maze.grid.forEach((row, y) => {
			[...row].forEach((cell, x) => {
				if (cell !== "#") open.add(`${x},${y}`);
				if (cell === "*") gems.add(`${x},${y}`);
			});
		});
		expect(gems.size).toBe(6);

		// Flood fill from the player. A maze that failed to connect would strand a
		// gem and make the game unwinnable, which no amount of play would reveal
		// reliably.
		const start_ = locate(maze.grid, "@") as [number, number];
		const seen = new Set([`${start_[0]},${start_[1]}`]);
		const queue: [number, number][] = [start_];
		while (queue.length > 0) {
			const [x, y] = queue.shift() as [number, number];
			for (const [dx, dy] of [
				[0, -1],
				[0, 1],
				[-1, 0],
				[1, 0],
			]) {
				const key = `${x + dx},${y + dy}`;
				if (seen.has(key) || !open.has(key)) continue;
				seen.add(key);
				queue.push([x + dx, y + dy]);
			}
		}
		for (const gem of gems) expect(seen.has(gem), `gem at ${gem} is walled off`).toBe(true);
	});

	// ── Duel: the game the model plays rather than watches ────────────────────

	it("alternates turns, and refuses each side's move out of turn", async () => {
		const { act } = await start();
		const duel = await act("select_game", { game: "duel" });
		expect(duel.duel?.turn).toBe("you");

		// The model moving first is the mistake worth naming precisely: the fix is
		// to wait, not to pick another direction, so it must not read as an illegal
		// move.
		await expectRefusal(act("take_turn", { direction: "left" }), "not_your_turn", /waiting on the person/);

		await act("press_arrow", { direction: "up" });
		const afterPerson = await act("get_state");
		expect(afterPerson.duel?.turn).toBe("model");

		// And the person cannot play twice: their second key is ignored rather than
		// erroring, because they are just early.
		await act("press_arrow", { direction: "up" });
		expect((await act("get_state")).duel?.turn).toBe("model");

		const afterModel = await act("take_turn", { direction: "left" });
		expect(afterModel.duel?.turn).toBe("you");
		expect(afterModel.duel?.last).toContain("The model moved left");
	});

	it("refuses a move off the board or onto the other player, and says which", async () => {
		const { act } = await start();
		const duel = await act("select_game", { game: "duel" });
		// The model starts in the top-right corner, so up and right leave the board.
		expect(locate(duel.grid, "M")).toEqual([duel.cols - 1, 0]);

		await act("press_arrow", { direction: "up" });
		await expectRefusal(act("take_turn", { direction: "up" }), "illegal_move", /off the board/);
		await expectRefusal(act("take_turn", { direction: "right" }), "illegal_move", /off the board/);
		// The turn is still the model's — a refused move costs nothing.
		expect((await act("get_state")).duel?.turn).toBe("model");
	});

	it("plays to a finish and scores it, with every coin accounted for", async () => {
		const { act } = await start();
		let state = await act("select_game", { game: "duel" });
		const coins = state.duel?.coinsLeft as number;
		expect(coins).toBe(9);

		// Both sides walk toward the nearest coin. An earlier version of this test
		// moved by fixed preference instead, which traced the border forever and
		// never emptied the board — the game was fine, the players were not.
		const DELTA = { up: [0, -1], down: [0, 1], left: [-1, 0], right: [1, 0] } as const;
		const chase = (glyph: string, snapshot: Snapshot): keyof typeof DELTA | undefined => {
			const me = locate(snapshot.grid, glyph) as [number, number];
			const them = locate(snapshot.grid, glyph === "@" ? "M" : "@") as [number, number];
			const key = (cell: [number, number]) => `${cell[0]},${cell[1]}`;
			const from = new Map<string, [number, number] | null>([[key(me), null]]);
			const queue: [number, number][] = [me];
			let goal: [number, number] | undefined;
			while (queue.length > 0 && !goal) {
				const cell = queue.shift() as [number, number];
				if ((snapshot.grid[cell[1]] as string)[cell[0]] === "o") {
					goal = cell;
					break;
				}
				for (const [dx, dy] of Object.values(DELTA)) {
					const next: [number, number] = [cell[0] + dx, cell[1] + dy];
					if (from.has(key(next)) || (next[0] === them[0] && next[1] === them[1])) continue;
					if (next[0] < 0 || next[1] < 0 || next[0] >= snapshot.cols || next[1] >= snapshot.rows) continue;
					from.set(key(next), cell);
					queue.push(next);
				}
			}
			if (!goal) return undefined;
			let step = goal;
			while (from.get(key(step)) && key(from.get(key(step)) as [number, number]) !== key(me)) {
				step = from.get(key(step)) as [number, number];
			}
			const move = [step[0] - me[0], step[1] - me[1]];
			return (Object.keys(DELTA) as (keyof typeof DELTA)[]).find(
				(name) => DELTA[name][0] === move[0] && DELTA[name][1] === move[1],
			);
		};

		for (let turn = 0; turn < 300 && !state.over; turn++) {
			const theirs = chase("@", state);
			if (!theirs) break;
			state = await act("press_arrow", { direction: theirs });
			if (state.over) break;
			const mine = chase("M", state);
			if (!mine) break;
			state = await act("take_turn", { direction: mine });
		}

		expect(state.over).toBe(true);
		expect(state.duel?.coinsLeft).toBe(0);
		expect((state.duel?.you as number) + (state.duel?.model as number)).toBe(coins);
		expect(state.status).toMatch(/you win|the model wins|a draw/);
		// A finished game stays finished.
		await expectRefusal(act("take_turn", { direction: "left" }), "game_over", /already over/);
	}, 60_000);

	it("keeps the turn actions out of the games that have no turns", async () => {
		const { act } = await start();
		await act("select_game", { game: "snake" });
		await expectRefusal(act("take_turn", { direction: "up" }), "not_playing_duel", /only for the Duel game/);
		await expectRefusal(act("await_turn"), "not_playing_duel", /only for the Duel game/);
	});

	it("wakes await_turn the moment the person moves, instead of making the model poll", async () => {
		const { act } = await start();
		await act("select_game", { game: "duel" });

		// Block first, move second: the ordering is the assertion. If the wait only
		// resolved on a later poll this would hang until its own timeout.
		const waiting = act("await_turn");
		await new Promise((resolve) => setTimeout(resolve, 150));
		await act("press_arrow", { direction: "up" });

		const woken = (await waiting) as Snapshot & { waiting: boolean };
		expect(woken.waiting).toBe(false);
		expect(woken.duel?.turn).toBe("model");
		expect(woken.duel?.last).toContain("You moved up");
	}, 40_000);
});
