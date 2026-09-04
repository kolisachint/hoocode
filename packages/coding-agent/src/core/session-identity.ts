/**
 * A session's visual identity: the name it carries before anyone names it, and
 * the palette slot its chip is filled with.
 *
 * Both are pure functions of the session id, which means they need no storage of
 * their own: the id is already in the session header, so a resumed session comes
 * back wearing the same slug and the same colour, and a fork — which mints a new
 * id — gets its own identity instead of quietly wearing its parent's. What *is*
 * stored is an override: `/name` and `/color` write to the session's
 * `session_info` entry, and those win over everything here (see
 * SessionManager.getDisplayName / getSessionColorSlot).
 *
 * This module is deliberately theme-free. It deals in slot *numbers*; mapping a
 * slot onto a colour token is the theme's job, so `core/` never has to reach into
 * a mode's rendering code.
 */

/** Number of colour slots a session can land in. Matches the theme's identity palette. */
export const SESSION_COLOR_SLOTS = 6;

/**
 * The slug vocabulary. Chosen for the terminal, not for poetry: every word is
 * short (so a two-word slug stays well inside the chip's 20-column cap), lower
 * case, unambiguous when read at a glance from across a desk, and free of the
 * near-homographs that make two sessions hard to tell apart (no "lake"/"lane").
 * 32 × 32 gives 1024 combinations, which is far more than the number of sessions
 * anyone has open at once — the point is recognition, not uniqueness.
 */
const ADJECTIVES = [
	"amber",
	"ash",
	"azure",
	"brisk",
	"bronze",
	"calm",
	"clay",
	"coral",
	"crisp",
	"dusk",
	"ember",
	"fern",
	"flint",
	"frost",
	"gold",
	"ivory",
	"jade",
	"lunar",
	"mellow",
	"mint",
	"olive",
	"onyx",
	"plum",
	"quiet",
	"rust",
	"sable",
	"sage",
	"slate",
	"solar",
	"swift",
	"teal",
	"velvet",
] as const;

const NOUNS = [
	"anchor",
	"arbor",
	"basin",
	"beacon",
	"birch",
	"canyon",
	"cedar",
	"cinder",
	"cove",
	"delta",
	"drift",
	"ferry",
	"forge",
	"harbor",
	"hollow",
	"lantern",
	"meadow",
	"mesa",
	"orchard",
	"pier",
	"prairie",
	"quarry",
	"ridge",
	"summit",
	"thicket",
	"tide",
	"trellis",
	"vale",
	"willow",
	"windmill",
	"yarrow",
	"zenith",
] as const;

/**
 * djb2 over the whole string. The same hash the agent palette uses — tiny,
 * stable across runs and machines, and good spread for identifier-like input.
 *
 * Hashing the *whole* id matters here. Session ids are uuidv7, which is
 * time-ordered: two sessions started in the same minute share their leading
 * characters, so a hash over a prefix would drop them into the same slug and the
 * same colour — exactly the sessions a person most needs to tell apart.
 */
function djb2(value: string): number {
	let hash = 5381;
	for (let i = 0; i < value.length; i++) {
		hash = ((hash << 5) + hash + value.charCodeAt(i)) >>> 0;
	}
	return hash;
}

/**
 * A second hash over the same string, seeded differently (sdbm). The slug needs
 * two independent indices; running djb2 twice would tie the noun to the
 * adjective and collapse the vocabulary from 1024 pairs to 32.
 */
function sdbm(value: string): number {
	let hash = 0;
	for (let i = 0; i < value.length; i++) {
		hash = (value.charCodeAt(i) + (hash << 6) + (hash << 16) - hash) >>> 0;
	}
	return hash;
}

/** The auto-assigned name for a session: a memorable two-word slug, e.g. `amber-harbor`. */
export function sessionSlugFor(sessionId: string): string {
	const adjective = ADJECTIVES[djb2(sessionId) % ADJECTIVES.length];
	const noun = NOUNS[sdbm(sessionId) % NOUNS.length];
	return `${adjective}-${noun}`;
}

/** The auto-assigned colour slot for a session, in `1…SESSION_COLOR_SLOTS`. */
export function sessionColorSlotFor(sessionId: string): number {
	return (djb2(sessionId) % SESSION_COLOR_SLOTS) + 1;
}

/** Whether `slot` is one a session can actually be assigned. */
export function isSessionColorSlot(slot: number): boolean {
	return Number.isInteger(slot) && slot >= 1 && slot <= SESSION_COLOR_SLOTS;
}

/**
 * The name each slot answers to, and the other spellings that reach it.
 *
 * A slot is a palette position, not a fixed hue — every theme fills
 * `agent1…agent6` with its own colours — but across the built-in themes each
 * position keeps a recognisable family, and that is what these names describe:
 * slot 3 is the warm yellow/amber one wherever you are, slot 6 the blue. That
 * is enough for the thing names are for, which is typing `/color green`
 * instead of remembering that green is the fifth swatch.
 *
 * The aliases are deliberately generous, because a name only helps if the
 * obvious spelling of it works. Each name's initial stands for it, and the
 * hues that sit between slots (`orange`, `red`) resolve to the nearest slot
 * rather than being rejected — the palette has six entries, not a full colour
 * wheel, and refusing `/color red` because the swatch is officially magenta
 * would be pedantry. A theme whose palette drifts from these names is still
 * addressable by number, which is why the numbers never go away.
 */
const SESSION_COLOR_NAMES: ReadonlyArray<{
	readonly slot: number;
	readonly name: string;
	readonly aliases: readonly string[];
}> = [
	{ slot: 1, name: "cyan", aliases: ["c", "teal", "aqua"] },
	{ slot: 2, name: "purple", aliases: ["p", "violet"] },
	{ slot: 3, name: "yellow", aliases: ["y", "amber", "gold", "orange", "o"] },
	{ slot: 4, name: "magenta", aliases: ["m", "pink", "rose", "red", "r"] },
	{ slot: 5, name: "green", aliases: ["g"] },
	{ slot: 6, name: "blue", aliases: ["b"] },
];

/** The canonical slot names, in slot order. For usage lines and pickers. */
export const SESSION_COLOR_NAME_LIST: readonly string[] = SESSION_COLOR_NAMES.map((entry) => entry.name);

/** What to call `slot` when reporting it back, or undefined if it is not a slot. */
export function sessionColorName(slot: number): string | undefined {
	return SESSION_COLOR_NAMES.find((entry) => entry.slot === slot)?.name;
}

/**
 * The slot an argument to `/color` asks for: a number (`4`), a name
 * (`magenta`), or an initial (`m`). Case and surrounding space do not matter.
 * Undefined when it names no slot at all.
 */
export function parseSessionColorSlot(input: string): number | undefined {
	const normalized = input.trim().toLowerCase();
	if (!normalized) return undefined;
	if (/^\d+$/.test(normalized)) {
		const slot = Number(normalized);
		return isSessionColorSlot(slot) ? slot : undefined;
	}
	const match = SESSION_COLOR_NAMES.find((entry) => entry.name === normalized || entry.aliases.includes(normalized));
	return match?.slot;
}

/**
 * The slot one step away from `current`, wrapping at both ends.
 *
 * The keyboard cycle needs this rather than a picker because picking a colour is
 * usually a two-second decision made while looking at four terminals: step until
 * the chip stops looking like its neighbour's, then stop. Wrapping means the
 * cycle has no dead end to back out of, and a session sitting on a slot no theme
 * defines — or on nothing at all — starts from the first one rather than
 * refusing to move.
 */
export function cycleSessionColorSlot(current: number, direction: "forward" | "backward"): number {
	if (!isSessionColorSlot(current)) return 1;
	const step = direction === "forward" ? 1 : SESSION_COLOR_SLOTS - 1;
	return ((current - 1 + step) % SESSION_COLOR_SLOTS) + 1;
}
