// arrow-key-games — Snake, a Maze, and Duel, a turn-based game the agent plays. Run it with:
//   /canvas open arrow-key-games
//
// The "@github/copilot-sdk/extension" import is resolved by the host at fork
// time. Do not install it, and do not add a node_modules here.
//
// Shape, and why:
//
//   - The server owns the game. The page is a renderer and a keyboard, nothing
//     more. That is what lets the agent read the board and act on it through the
//     canvas actions: a game living in the browser would be invisible to it.
//   - Every game is a grid, so there is one renderer and three rule sets. State
//     goes to the page over SSE on each change, keys come back over POST.
//   - The page carries no inline script and no external asset, so it runs under
//     a strict CSP. The capability token is substituted into a <meta> tag.
import { createCanvas, CanvasError, joinSession } from "@github/copilot-sdk/extension";
import { randomBytes } from "node:crypto";
import { createServer } from "node:http";

/** Per-instance state. A canvas can be opened more than once at a time. */
const instances = new Map();

/** Cell kinds. Single characters so a board is also a readable ASCII map. */
const EMPTY = ".";
const WALL = "#";
const GEM = "*";
const FOOD = "o";
const SNAKE = "s";
const HEAD = "S";
const COIN = "o";
const MODEL = "M";

const DIRECTIONS = {
	up: [0, -1],
	down: [0, 1],
	left: [-1, 0],
	right: [1, 0],
};

const KEY_TO_DIRECTION = {
	ArrowUp: "up",
	ArrowDown: "down",
	ArrowLeft: "left",
	ArrowRight: "right",
};

const SNAKE_TICK_MS = 130;

// ── Games ────────────────────────────────────────────────────────────────────

/**
 * Snake. The only game here with a clock: it advances on a tick rather than on
 * a key, and the key only changes heading.
 */
function newSnake() {
	const cols = 21;
	const rows = 15;
	const body = [
		[10, 7],
		[9, 7],
		[8, 7],
	];
	// `started` is why the clock does not run yet. Without it the snake sets off the
	// instant the canvas opens and is dead against the far wall in about a second —
	// before the person has had a chance to press anything.
	const game = { kind: "snake", cols, rows, body, heading: "right", pending: "right", food: null, score: 0, over: null, started: false };
	game.food = randomFreeCell(game, occupiedBySnake(game));
	return game;
}

function occupiedBySnake(game) {
	return new Set(game.body.map(([x, y]) => `${x},${y}`));
}

function stepSnake(game) {
	if (game.over) return false;
	game.heading = game.pending;
	const [dx, dy] = DIRECTIONS[game.heading];
	const [hx, hy] = game.body[0];
	const next = [hx + dx, hy + dy];

	if (next[0] < 0 || next[1] < 0 || next[0] >= game.cols || next[1] >= game.rows) {
		game.over = "hit the wall";
		return true;
	}
	// The tail square is about to be vacated, so running into it is not a crash —
	// unless the snake is about to grow into it.
	const eating = game.food && next[0] === game.food[0] && next[1] === game.food[1];
	const blocked = game.body.slice(0, eating ? game.body.length : -1);
	if (blocked.some(([x, y]) => x === next[0] && y === next[1])) {
		game.over = "ran into itself";
		return true;
	}

	game.body.unshift(next);
	if (eating) {
		game.score += 1;
		game.food = randomFreeCell(game, occupiedBySnake(game));
		if (!game.food) game.over = "filled the board — perfect game";
	} else {
		game.body.pop();
	}
	return true;
}

/**
 * A maze carved by recursive backtracking, which is the point: it is connected
 * by construction, so every gem dropped on an open cell is reachable and the
 * game can never be unwinnable.
 */
function newMaze() {
	const cols = 21;
	const rows = 15;
	const cells = Array.from({ length: rows }, () => Array.from({ length: cols }, () => WALL));

	const carve = (x, y) => {
		cells[y][x] = EMPTY;
		const order = shuffle(Object.values(DIRECTIONS));
		for (const [dx, dy] of order) {
			const nx = x + dx * 2;
			const ny = y + dy * 2;
			if (nx <= 0 || ny <= 0 || nx >= cols - 1 || ny >= rows - 1) continue;
			if (cells[ny][nx] !== WALL) continue;
			cells[y + dy][x + dx] = EMPTY;
			carve(nx, ny);
		}
	};
	carve(1, 1);

	const game = { kind: "maze", cols, rows, cells, player: [1, 1], gems: [], moves: 0, score: 0, over: null };
	const taken = new Set(["1,1"]);
	for (let i = 0; i < 6; i++) {
		const gem = randomFreeCell(game, taken);
		if (!gem) break;
		taken.add(`${gem[0]},${gem[1]}`);
		game.gems.push(gem);
	}
	return game;
}

function moveMaze(game, direction) {
	if (game.over) return false;
	const [dx, dy] = DIRECTIONS[direction];
	const x = game.player[0] + dx;
	const y = game.player[1] + dy;
	if (x < 0 || y < 0 || x >= game.cols || y >= game.rows) return false;
	if (game.cells[y][x] === WALL) return false;

	game.player = [x, y];
	game.moves += 1;
	const hit = game.gems.findIndex(([gx, gy]) => gx === x && gy === y);
	if (hit >= 0) {
		game.gems.splice(hit, 1);
		game.score += 1;
	}
	if (game.gems.length === 0) game.over = `all gems in ${game.moves} moves`;
	return true;
}

/**
 * Duel — the game the agent plays rather than watches.
 *
 * Turn-based on purpose. The other two games are real-time, and real time is
 * exactly what an agent cannot take part in: it acts in discrete tool calls
 * seconds apart, so a shared clock would mean it always loses. Alternating turns
 * make the two players' very different tempos irrelevant, which is what lets one
 * board hold a person on a keyboard and a model on a tool call.
 */
function newDuel() {
	const cols = 9;
	const rows = 9;
	const game = {
		kind: "duel",
		cols,
		rows,
		you: [0, rows - 1],
		model: [cols - 1, 0],
		coins: [],
		turn: "you",
		scores: { you: 0, model: 0 },
		last: "You move first.",
		over: null,
		score: 0,
	};
	const taken = new Set([`${game.you[0]},${game.you[1]}`, `${game.model[0]},${game.model[1]}`]);
	for (let i = 0; i < 9; i++) {
		const coin = randomFreeCell(game, taken);
		if (!coin) break;
		taken.add(`${coin[0]},${coin[1]}`);
		game.coins.push(coin);
	}
	return game;
}

/** Who won, once the coins run out. */
function duelVerdict(game) {
	const { you, model } = game.scores;
	if (you > model) return `you win ${you}–${model}`;
	if (model > you) return `the model wins ${model}–${you}`;
	return `a draw at ${you}–${you}`;
}

/**
 * One player's move. Returns a refusal string rather than throwing, so the same
 * code can answer a keypress (ignore it) and an action (turn it into a
 * CanvasError the model can read and correct).
 */
function moveDuel(game, side, direction) {
	if (game.over) return { code: "game_over", why: `the game is already over — ${game.over}` };
	if (game.turn !== side) return { code: "not_your_turn", why: `waiting on ${game.turn === "you" ? "the person" : "the model"} to move` };

	const [dx, dy] = DIRECTIONS[direction];
	const from = side === "you" ? game.you : game.model;
	const x = from[0] + dx;
	const y = from[1] + dy;
	if (x < 0 || y < 0 || x >= game.cols || y >= game.rows) return { code: "illegal_move", why: "that is off the board" };

	const other = side === "you" ? game.model : game.you;
	if (x === other[0] && y === other[1]) return { code: "illegal_move", why: "the other player is standing there" };

	if (side === "you") game.you = [x, y];
	else game.model = [x, y];

	const who = side === "you" ? "You" : "The model";
	const hit = game.coins.findIndex(([cx, cy]) => cx === x && cy === y);
	if (hit >= 0) {
		game.coins.splice(hit, 1);
		game.scores[side] += 1;
		game.last = `${who} moved ${direction} and took a coin.`;
	} else {
		game.last = `${who} moved ${direction}.`;
	}

	if (game.coins.length === 0) {
		game.over = duelVerdict(game);
		game.turn = null;
	} else {
		game.turn = side === "you" ? "model" : "you";
	}
	return null;
}

// ── Grid helpers ─────────────────────────────────────────────────────────────

function shuffle(items) {
	const copy = [...items];
	for (let i = copy.length - 1; i > 0; i--) {
		const j = Math.floor(Math.random() * (i + 1));
		[copy[i], copy[j]] = [copy[j], copy[i]];
	}
	return copy;
}

/** A random open cell, or null when there is none left. */
function randomFreeCell(game, taken) {
	const free = [];
	for (let y = 0; y < game.rows; y++) {
		for (let x = 0; x < game.cols; x++) {
			if (game.cells && game.cells[y][x] === WALL) continue;
			if (taken.has(`${x},${y}`)) continue;
			free.push([x, y]);
		}
	}
	if (free.length === 0) return null;
	return free[Math.floor(Math.random() * free.length)];
}

/** Flatten a game to the grid of characters both the page and the agent read. */
function toGrid(game) {
	const grid = Array.from({ length: game.rows }, () => Array.from({ length: game.cols }, () => EMPTY));
	if (game.kind === "maze") {
		for (let y = 0; y < game.rows; y++) for (let x = 0; x < game.cols; x++) grid[y][x] = game.cells[y][x];
		for (const [x, y] of game.gems) grid[y][x] = GEM;
		grid[game.player[1]][game.player[0]] = "@";
	}
	if (game.kind === "snake") {
		if (game.food) grid[game.food[1]][game.food[0]] = FOOD;
		game.body.forEach(([x, y], index) => {
			grid[y][x] = index === 0 ? HEAD : SNAKE;
		});
	}
	if (game.kind === "duel") {
		for (const [x, y] of game.coins) grid[y][x] = COIN;
		grid[game.you[1]][game.you[0]] = "@";
		grid[game.model[1]][game.model[0]] = MODEL;
	}
	return grid;
}

const GAMES = {
	snake: { title: "Snake", create: newSnake, ticks: true, help: "Arrow keys steer. Eat the o, do not hit a wall or yourself." },
	maze: { title: "Maze", create: newMaze, ticks: false, help: "Arrow keys move one square. Collect every *." },
	duel: {
		title: "Duel vs the model",
		create: newDuel,
		ticks: false,
		help: "You are @, the model is M. Take turns moving one square with the arrow keys; whoever grabs more o wins.",
	},
};

// ── State the page and the agent both read ───────────────────────────────────

function snapshot(entry) {
	const game = entry.game;
	const spec = GAMES[entry.selected];
	return {
		game: entry.selected,
		title: spec.title,
		help: spec.help,
		games: Object.entries(GAMES).map(([id, g]) => ({ id, title: g.title })),
		cols: game.cols,
		rows: game.rows,
		grid: toGrid(game).map((row) => row.join("")),
		score: game.kind === "duel" ? game.scores.you : game.score,
		// Only duel has two sides, so only duel reports them. The other games would
		// have to invent an opponent to fill these in.
		duel:
			game.kind === "duel"
				? { turn: game.turn, you: game.scores.you, model: game.scores.model, coinsLeft: game.coins.length, last: game.last }
				: null,
		status: game.over
			? `Game over — ${game.over}`
			: game.started === false
				? "Press an arrow key to start"
				: game.kind === "duel"
					? game.turn === "you"
						? "Your turn — press an arrow key"
						: "The model's turn…"
					: "Playing",
		over: Boolean(game.over),
		detail:
			game.kind === "maze"
				? `${game.gems.length} gem(s) left · ${game.moves} moves`
				: game.kind === "duel"
					? `you ${game.scores.you} · model ${game.scores.model} · ${game.coins.length} coin(s) left — ${game.last}`
					: `length ${game.body.length}`,
	};
}

function broadcast(entry) {
	const payload = `data: ${JSON.stringify(snapshot(entry))}\n\n`;
	for (const client of entry.clients) client.write(payload);
	releaseWaiters(entry);
}

/** Run the clock only while a ticking game is actually in play. */
function syncTicker(entry) {
	const wants = GAMES[entry.selected].ticks && entry.game.started && !entry.game.over;
	if (wants && !entry.ticker) {
		entry.ticker = setInterval(() => {
			if (stepSnake(entry.game)) broadcast(entry);
			if (entry.game.over) syncTicker(entry);
		}, SNAKE_TICK_MS);
	}
	if (!wants && entry.ticker) {
		clearInterval(entry.ticker);
		entry.ticker = null;
	}
}

function selectGame(entry, id) {
	if (!GAMES[id]) throw new CanvasError("unknown_game", `No game "${id}". Available: ${Object.keys(GAMES).join(", ")}.`);
	entry.selected = id;
	entry.game = GAMES[id].create();
	// A `await_turn` blocked on the duel that just ended would otherwise hang until
	// its own timeout, so every waiter is released on any game change.
	for (const waiter of entry.waiters) waiter();
	entry.waiters.clear();
	syncTicker(entry);
	broadcast(entry);
}

/** Apply one direction to whichever game is in play. Returns whether it changed. */
function applyDirection(entry, direction) {
	if (!DIRECTIONS[direction]) {
		throw new CanvasError("bad_direction", `Direction must be one of: ${Object.keys(DIRECTIONS).join(", ")}.`);
	}
	if (entry.selected === "snake") {
		// A snake cannot reverse into its own neck, so that key is ignored rather
		// than being an instant loss.
		const opposite = { up: "down", down: "up", left: "right", right: "left" };
		if (entry.game.over) return false;
		// The first arrow starts the clock, whichever way it points.
		if (!entry.game.started) {
			entry.game.started = true;
			entry.game.pending = direction;
			syncTicker(entry);
			return true;
		}
		if (opposite[direction] === entry.game.heading) return false;
		entry.game.pending = direction;
		return true;
	}
	if (entry.selected === "duel") {
		// A keypress out of turn is not an error worth showing — the person is just
		// early. The page already says whose turn it is.
		return moveDuel(entry.game, "you", direction) === null;
	}
	return moveMaze(entry.game, direction);
}

/**
 * Wake anything blocked in `await_turn`.
 *
 * Without this the model's only way to learn the person has moved is to poll
 * `get_state` in a loop, which spends a tool call per look and still lags. A
 * canvas the agent takes turns on needs a way to *wait*, and one action call
 * that blocks is cheaper than ten that ask.
 */
function releaseWaiters(entry) {
	if (entry.selected !== "duel") return;
	const game = entry.game;
	if (game.turn !== "model" && !game.over) return;
	for (const waiter of entry.waiters) waiter();
	entry.waiters.clear();
}

// ── The page ─────────────────────────────────────────────────────────────────

const PAGE = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="canvas-token" content="__TOKEN__">
<title>Arrow-key games</title>
<link rel="stylesheet" href="/app.css?token=__TOKEN__">
</head>
<body>
<header>
  <h1 id="title">…</h1>
  <nav id="games"></nav>
</header>
<p id="help"></p>
<div id="board" aria-live="polite"></div>
<footer>
  <strong id="status">connecting…</strong>
  <span id="detail"></span>
  <span id="score"></span>
  <button id="restart" type="button">Restart</button>
</footer>
<script src="/app.js?token=__TOKEN__"></script>
</body>
</html>`;

const STYLE = `
:root { color-scheme: dark light; --cell: 22px; }
body { font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; margin: 0; padding: 20px;
       background: #0d1117; color: #e6edf3; }
h1 { font-size: 18px; margin: 0; }
header { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
nav { display: flex; gap: 8px; }
button { font: inherit; background: #21262d; color: #e6edf3; border: 1px solid #30363d;
         border-radius: 6px; padding: 4px 10px; cursor: pointer; }
button[aria-pressed="true"] { background: #1f6feb; border-color: #1f6feb; }
button:hover { border-color: #8b949e; }
#help { color: #8b949e; margin: 8px 0 14px; }
#board { display: grid; gap: 1px; width: max-content;
         background: #161b22; padding: 6px; border-radius: 8px; border: 1px solid #30363d; }
/* Floor near-black and walls clearly lighter: at #30363d on #0d1117 the maze was
   technically rendered and practically unreadable. */
#board div { width: var(--cell); height: var(--cell); border-radius: 3px; background: #010409; }
#board .wall { background: #57606a; border-radius: 1px; }
#board .gem { background: #d29922; }
#board .food { background: #f85149; border-radius: 50%; }
#board .snake { background: #238636; }
#board .head { background: #3fb950; }
#board .player { background: #58a6ff; }
#board .model { background: #bc8cff; }
/* Whose turn it is has to be readable at a glance, or the person sits there
   pressing keys the server is quietly refusing. */
#board.waiting { border-color: #bc8cff; opacity: .75; }
#board.yours { border-color: #58a6ff; }
.turn { font-weight: 700; }
.turn.yours { color: #58a6ff; }
.turn.theirs { color: #bc8cff; }
footer { display: flex; align-items: center; gap: 14px; margin-top: 14px; flex-wrap: wrap; }
#detail, #score { color: #8b949e; }
.over { color: #f85149; }
`;

const CLIENT = `
const token = document.querySelector('meta[name="canvas-token"]').content;
const board = document.getElementById("board");
const CLASS = { "#": "wall", "*": "gem", o: "food", s: "snake", S: "head", "@": "player", M: "model" };
let cells = [];
let shape = "";

function render(state) {
  document.getElementById("title").textContent = state.title;
  document.getElementById("help").textContent = state.help;
  const status = document.getElementById("status");
  status.textContent = state.status;
  status.className = state.over ? "over" : state.duel ? "turn " + (state.duel.turn === "you" ? "yours" : "theirs") : "";
  board.classList.toggle("waiting", Boolean(state.duel) && state.duel.turn === "model");
  board.classList.toggle("yours", Boolean(state.duel) && state.duel.turn === "you");
  document.getElementById("detail").textContent = state.detail;
  document.getElementById("score").textContent = "score " + state.score;

  const nav = document.getElementById("games");
  if (nav.childElementCount !== state.games.length) {
    nav.replaceChildren(...state.games.map((g) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = g.title;
      b.dataset.game = g.id;
      b.addEventListener("click", () => post("/select", { game: g.id }));
      return b;
    }));
  }
  for (const b of nav.children) b.setAttribute("aria-pressed", String(b.dataset.game === state.game));

  const next = state.cols + "x" + state.rows;
  if (next !== shape) {
    shape = next;
    board.style.gridTemplateColumns = "repeat(" + state.cols + ", var(--cell))";
    cells = Array.from({ length: state.cols * state.rows }, () => document.createElement("div"));
    board.replaceChildren(...cells);
  }
  const flat = state.grid.join("");
  for (let i = 0; i < cells.length; i++) cells[i].className = CLASS[flat[i]] || "";
}

function post(path, body) {
  return fetch(path + "?token=" + encodeURIComponent(token), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

document.getElementById("restart").addEventListener("click", () => post("/restart", {}));
window.addEventListener("keydown", (event) => {
  if (!event.key.startsWith("Arrow")) return;
  event.preventDefault();
  post("/input", { key: event.key });
});

const events = new EventSource("/events?token=" + encodeURIComponent(token));
events.onmessage = (event) => render(JSON.parse(event.data));
events.onerror = () => { document.getElementById("status").textContent = "disconnected"; };
`;

// ── Wiring ───────────────────────────────────────────────────────────────────

function readJson(req) {
	return new Promise((resolve) => {
		let raw = "";
		req.on("data", (chunk) => {
			raw += chunk;
			// A body this size is already not one of ours; stop reading rather than
			// buffering whatever a stray client decides to send.
			if (raw.length > 4096) req.destroy();
		});
		req.on("end", () => {
			try {
				resolve(raw ? JSON.parse(raw) : {});
			} catch {
				resolve({});
			}
		});
	});
}

const session = await joinSession({
	canvases: [
		createCanvas({
			// The id is the identity an open instance is bound to, so it stays as the
			// extension was scaffolded even though the canvas grew a nicer name — a
			// rename here would drop the canvas the person is looking at on the next
			// reload. `displayName` is the half they actually see.
			id: "arrow-key-games",
			displayName: "Arrow-key games",
			description:
				"Lightweight keyboard games on a grid — Snake, a Maze, and Duel, a turn-based coin race you play against the person. They use the arrow keys; you read the board and move through the actions below.",
			actions: [
				{
					name: "get_state",
					description:
						"Read the board: which game is in play, the grid as rows of characters, the score, and whether it is over. Grid legend: '#' wall, '*' gem, 'o' food or coin, 'S' snake head, 's' snake body, '@' the person, 'M' you (Duel only), '.' empty. Row 0 is the top; within a row, column 0 is the left. In Duel the `duel` field carries whose turn it is and both scores.",
					inputSchema: { type: "object", properties: {}, additionalProperties: false },
					handler: (ctx) => {
						const entry = requireInstance(ctx.instanceId);
						return snapshot(entry);
					},
				},
				{
					name: "select_game",
					description: "Switch to a game and start it fresh. Use get_state first to see which games exist.",
					inputSchema: {
						type: "object",
						properties: { game: { type: "string", description: "Game id, one of the ids get_state lists." } },
						required: ["game"],
						additionalProperties: false,
					},
					handler: (ctx) => {
						const entry = requireInstance(ctx.instanceId);
						selectGame(entry, ctx.input.game);
						return snapshot(entry);
					},
				},
				{
					name: "restart_game",
					description: "Start the current game over from the beginning.",
					inputSchema: { type: "object", properties: {}, additionalProperties: false },
					handler: (ctx) => {
						const entry = requireInstance(ctx.instanceId);
						selectGame(entry, entry.selected);
						return snapshot(entry);
					},
				},
				{
					name: "take_turn",
					description:
						"Play YOUR move in the Duel game. You are 'M' on the board, the person is '@', and 'o' is an uncollected coin. You may only move on your turn: call await_turn first, or read `duel.turn` from get_state. One square per turn, orthogonally, not off the board and not onto the person. Landing on a coin takes it. Returns the board after your move, with it back to the person's turn.",
					inputSchema: {
						type: "object",
						properties: { direction: { type: "string", enum: ["up", "down", "left", "right"] } },
						required: ["direction"],
						additionalProperties: false,
					},
					handler: (ctx) => {
						const entry = requireInstance(ctx.instanceId);
						if (entry.selected !== "duel") {
							throw new CanvasError(
								"not_playing_duel",
								`take_turn is only for the Duel game; "${entry.selected}" is in play. Call select_game with "duel" first.`,
							);
						}
						// Refusals come back as a code the person and the model can both read,
						// rather than as a silently ignored move — an agent that cannot tell
						// "off the board" from "not your turn" cannot correct itself.
						const refused = moveDuel(entry.game, "model", ctx.input.direction);
						if (refused) {
							throw new CanvasError(
								refused.code,
								refused.code === "not_your_turn"
									? `Not your turn yet — ${refused.why}. Call await_turn.`
									: `Cannot move ${ctx.input.direction}: ${refused.why}.`,
							);
						}
						broadcast(entry);
						return snapshot(entry);
					},
				},
				{
					name: "await_turn",
					description:
						"Block until it is your turn in the Duel game, then return the board. Use this instead of polling get_state after you have moved: it returns the moment the person plays. If nothing happens it gives up after about 25 seconds and returns waiting:true — call it again, or ask the person whether they are still there.",
					inputSchema: { type: "object", properties: {}, additionalProperties: false },
					handler: async (ctx) => {
						const entry = requireInstance(ctx.instanceId);
						if (entry.selected !== "duel") {
							throw new CanvasError(
								"not_playing_duel",
								`await_turn is only for the Duel game; "${entry.selected}" is in play. Call select_game with "duel" first.`,
							);
						}
						if (entry.game.turn === "model" || entry.game.over) return { waiting: false, ...snapshot(entry) };

						// Comfortably inside the host's 30s ceiling for an action, so a quiet
						// game reports back as waiting rather than as a timeout.
						const timedOut = await new Promise((resolve) => {
							const timer = setTimeout(() => {
								entry.waiters.delete(wake);
								resolve(true);
							}, 25_000);
							const wake = () => {
								clearTimeout(timer);
								resolve(false);
							};
							entry.waiters.add(wake);
						});
						return { waiting: timedOut && entry.game.turn !== "model", ...snapshot(entry) };
					},
				},
				{
					name: "press_arrow",
					description:
						"Press an arrow key on the person's behalf — steer the snake, or step the maze player one square. Same effect as them pressing it. In Duel this plays THEIR move, not yours, so use take_turn there instead.",
					inputSchema: {
						type: "object",
						properties: { direction: { type: "string", enum: ["up", "down", "left", "right"] } },
						required: ["direction"],
						additionalProperties: false,
					},
					handler: (ctx) => {
						const entry = requireInstance(ctx.instanceId);
						const moved = applyDirection(entry, ctx.input.direction);
						if (moved) broadcast(entry);
						return { moved, ...snapshot(entry) };
					},
				},
			],
			open: async (ctx) => {
				const token = randomBytes(32).toString("base64url");
				const entry = { token, clients: new Set(), waiters: new Set(), ticker: null, selected: "snake", game: newSnake() };

				const server = createServer(async (req, res) => {
					const url = new URL(req.url ?? "/", "http://127.0.0.1");
					if (url.searchParams.get("token") !== token) {
						res.writeHead(403, { "Content-Type": "text/plain" });
						res.end("forbidden");
						return;
					}

					if (url.pathname === "/events") {
						res.writeHead(200, {
							"Content-Type": "text/event-stream",
							"Cache-Control": "no-cache",
							Connection: "keep-alive",
						});
						entry.clients.add(res);
						res.write(`data: ${JSON.stringify(snapshot(entry))}\n\n`);
						req.on("close", () => entry.clients.delete(res));
						return;
					}

					if (req.method === "POST") {
						const body = await readJson(req);
						try {
							if (url.pathname === "/input") {
								const direction = KEY_TO_DIRECTION[body.key];
								if (direction && applyDirection(entry, direction)) broadcast(entry);
							} else if (url.pathname === "/select") {
								selectGame(entry, body.game);
							} else if (url.pathname === "/restart") {
								selectGame(entry, entry.selected);
							}
						} catch (error) {
							res.writeHead(400, { "Content-Type": "application/json" });
							res.end(JSON.stringify({ error: error.message }));
							return;
						}
						res.writeHead(204).end();
						return;
					}

					if (url.pathname === "/app.js") {
						res.writeHead(200, { "Content-Type": "text/javascript; charset=utf-8" });
						res.end(CLIENT);
						return;
					}
					if (url.pathname === "/app.css") {
						res.writeHead(200, { "Content-Type": "text/css; charset=utf-8" });
						res.end(STYLE);
						return;
					}
					if (url.pathname === "/") {
						res.writeHead(200, {
							"Content-Type": "text/html; charset=utf-8",
							// No inline script and no external asset, so the page needs nothing
							// beyond its own origin.
							"Content-Security-Policy": "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'",
						});
						// Every route is behind the token, the two asset routes included — so
						// the page's own <link> and <script> have to carry it. Fetching them
						// without it is a 403, which a curl check that appends the token by
						// hand will never see and a browser hits immediately.
						res.end(PAGE.replaceAll("__TOKEN__", token));
						return;
					}
					res.writeHead(404, { "Content-Type": "text/plain" });
					res.end("not found");
				});

				// Port 0 on 127.0.0.1: an ephemeral port, reachable only from this machine.
				await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
				const { port } = server.address();
				entry.server = server;
				instances.set(ctx.instanceId, entry);
				syncTicker(entry);
				return {
					url: `http://127.0.0.1:${port}/?token=${token}`,
					title: "Arrow-key games",
				};
			},
			onClose: async (ctx) => {
				const entry = instances.get(ctx.instanceId);
				// Called for an abandoned open too, so the instance may be unknown.
				if (!entry) return;
				instances.delete(ctx.instanceId);
				if (entry.ticker) clearInterval(entry.ticker);
				for (const waiter of entry.waiters) waiter();
				entry.waiters.clear();
				// An open SSE stream is a live socket, and `close` waits for those — so
				// without ending them here the port outlives the canvas.
				for (const client of entry.clients) client.end();
				entry.clients.clear();
				entry.server.closeAllConnections();
				await new Promise((resolve) => entry.server.close(resolve));
			},
		}),
	],
});

/** CanvasError carries a code the host shows verbatim; a bare throw is opaque. */
function requireInstance(instanceId) {
	const entry = instances.get(instanceId);
	if (!entry) throw new CanvasError("no_instance", `Instance "${instanceId}" is not open.`);
	return entry;
}

// stdout is the protocol channel. Use session.log, never console.log.
await session.log("arrow-key-games ready");
