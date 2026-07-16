/**
 * Completion chime — a short audible cue (terminal BEL) played when an assistant
 * turn finishes after the user has likely stepped away, or when the agent blocks
 * awaiting the user's input (the ask_options pane).
 *
 * Design constraints:
 *   - Output-only, zero dependencies: the cue is a single BEL byte written to the
 *     terminal. No bundled audio, no subprocesses.
 *   - Side-effect-free from the turn's perspective: the ring is wrapped in a
 *     try/catch so a failed write can never throw into the turn path. Worst case
 *     is no sound.
 *
 * Firing conditions (the caller drives these; this class only decides whether a
 * given signal should actually ring):
 *   - Turn-complete: the turn ran longer than {@link CompletionChimeOptions.thresholdMs}
 *     (default 10s), measured from the turn's start to when the agent goes truly
 *     idle, and the turn was not user-aborted.
 *   - Blocked-for-input: the agent opened the ask_options pane. This bypasses the
 *     duration threshold — "it needs you" is worth surfacing immediately.
 *
 * Both paths are gated by an enable check and share a single debounce window so
 * rapid successive turns (or a long turn that ends in an ask_options prompt) do
 * not spam the bell.
 */

/** Terminal bell (BEL, `\a`). Rings the terminal's configured audible/visual alert. */
export const BELL = "\x07";

const DEFAULT_THRESHOLD_MS = 10_000;
const DEFAULT_DEBOUNCE_MS = 5_000;

export interface CompletionChimeOptions {
	/** Read fresh on every potential ring so live setting changes take effect immediately. */
	isEnabled: () => boolean;
	/**
	 * Emit the audible cue. Called inside a try/catch — implementations need not
	 * guard against throwing, but nothing they throw will reach the turn path.
	 */
	ring: () => void;
	/** Minimum turn duration before a turn-complete chime fires. Defaults to 10s. */
	thresholdMs?: number;
	/** Minimum gap between two chimes. Defaults to 5s. */
	debounceMs?: number;
	/** Clock, injectable for tests. Defaults to {@link Date.now}. */
	now?: () => number;
}

export class CompletionChime {
	private readonly isEnabled: () => boolean;
	private readonly doRing: () => void;
	private readonly thresholdMs: number;
	private readonly debounceMs: number;
	private readonly now: () => number;

	/** Start of the current turn, or undefined when no turn is in flight. */
	private turnStart: number | undefined;
	/** Timestamp of the last chime, for debouncing. */
	private lastChimeAt: number | undefined;

	constructor(options: CompletionChimeOptions) {
		this.isEnabled = options.isEnabled;
		this.doRing = options.ring;
		this.thresholdMs = options.thresholdMs ?? DEFAULT_THRESHOLD_MS;
		this.debounceMs = options.debounceMs ?? DEFAULT_DEBOUNCE_MS;
		this.now = options.now ?? Date.now;
	}

	/**
	 * Anchor the start of a turn. Only the first call per turn takes effect, so a
	 * turn that internally retries (re-entering agent_start) still measures its
	 * duration from the original start.
	 */
	onTurnStart(): void {
		if (this.turnStart === undefined) {
			this.turnStart = this.now();
		}
	}

	/**
	 * The agent has gone truly idle after a turn. Rings if the turn ran longer than
	 * the threshold and was not aborted. Clears the turn anchor either way.
	 */
	onTurnComplete(options: { aborted: boolean }): void {
		const start = this.turnStart;
		this.turnStart = undefined;
		if (options.aborted || start === undefined) {
			return;
		}
		if (this.now() - start < this.thresholdMs) {
			return;
		}
		this.fire();
	}

	/**
	 * The agent is blocked awaiting user input (ask_options pane). Rings
	 * immediately, bypassing the duration threshold. Leaves the turn anchor intact
	 * — the turn is still in flight and will complete once the user answers.
	 */
	onBlockedForInput(): void {
		this.fire();
	}

	private fire(): void {
		if (!this.isEnabled()) {
			return;
		}
		const at = this.now();
		if (this.lastChimeAt !== undefined && at - this.lastChimeAt < this.debounceMs) {
			return;
		}
		this.lastChimeAt = at;
		try {
			this.doRing();
		} catch {
			// Side-effect-free: worst case is no sound. Never throw into the turn path.
		}
	}
}
