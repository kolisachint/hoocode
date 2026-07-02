/**
 * Multi-line voice-input status panel.
 *
 * Renders the live state of a `voicetools serve` capture across its phases so
 * the user always sees what's happening:
 *
 * ```text
 *  🎙  Listening · 0:03
 *  ▁▂▃▅▇▆▄▂▁▂▄▆▇▅▃▂▁            ← scrolling level history from LEVEL events
 *  › and so my fellow ameri▏    ← live partial transcript (PARTIAL), dim/tentative
 *  ctrl+r  cancel               ← dim key footer
 * ```
 *
 * Phases:
 *  - `warming`      spinner while the model loads (first press only)
 *  - `listening`    mic glyph + elapsed timer + waveform + growing partial text
 *  - `silence`      trailing-silence countdown; waveform dims
 *  - `transcribing` spinner while the final decode runs
 *
 * The component self-animates (spinner, timer, countdown) on an internal timer
 * and calls `ui.requestRender()`; the owner feeds it protocol events via
 * `pushLevel` / `setPartial` / `beginSilence` etc. and disposes it on collapse.
 */

import type { Component, TUI } from "@kolisachint/hoocode-tui";
import { visibleWidth } from "@kolisachint/hoocode-tui";
import { theme } from "../theme/theme.js";

export type VoicePanelPhase = "warming" | "listening" | "silence" | "transcribing";

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
// 8 filled levels; index 0 renders as a baseline dot so an empty history still
// reads as "a track", not a blank line.
const WAVE_LEVELS = ["▁", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const TICK_MS = 100;
const HISTORY_CAP = 64;
// RMS from voicetools' vad::rms sits well below 1.0 for speech; this gain maps a
// normal speaking level to roughly the top of the meter without clipping constantly.
const LEVEL_GAIN = 6;
// During trailing silence the binary keeps emitting low LEVELs; only an RMS above
// this counts as the speaker resuming, so we don't cancel the countdown on noise.
const RESUME_RMS = 0.02;

/** Map an RMS energy to a waveform block glyph. */
function levelGlyph(rms: number): string {
	const norm = Math.max(0, Math.min(1, rms * LEVEL_GAIN));
	const idx = Math.round(norm * (WAVE_LEVELS.length - 1));
	return WAVE_LEVELS[idx] ?? WAVE_LEVELS[0]!;
}

/** Keep the tail (most recent chars) of `text` within `max` visible columns. */
function clampTail(text: string, max: number): string {
	if (max <= 0) return "";
	if (visibleWidth(text) <= max) return text;
	// Partial transcripts are effectively plain text; slice by chars from the end
	// and prefix an ellipsis so the newest words stay visible as it grows.
	const tail = text.slice(-(max - 1));
	return `…${tail}`;
}

export class VoicePanel implements Component {
	private phase: VoicePanelPhase = "warming";
	private frame = 0;
	private levels: number[] = [];
	private partial = "";
	private listeningStartMs = 0;
	private silenceRemainingMs = 0;
	private warmingMessage = "Warming up voice input…";
	private timer: ReturnType<typeof setInterval> | undefined;

	constructor(
		private readonly ui: TUI | undefined,
		private readonly cancelHint: string,
	) {}

	// ---- lifecycle -------------------------------------------------------

	private ensureAnimating(): void {
		if (this.timer) return;
		this.timer = setInterval(() => {
			this.frame = (this.frame + 1) % SPINNER_FRAMES.length;
			if (this.phase === "silence" && this.silenceRemainingMs > 0) {
				this.silenceRemainingMs = Math.max(0, this.silenceRemainingMs - TICK_MS);
			}
			this.ui?.requestRender();
		}, TICK_MS);
	}

	/** Stop timers. The owner removes the panel from its container to collapse it. */
	dispose(): void {
		if (this.timer) {
			clearInterval(this.timer);
			this.timer = undefined;
		}
	}

	/** No cached render state to clear; every `render()` recomputes from live state. */
	invalidate(): void {}

	// ---- phase transitions ----------------------------------------------

	setWarming(message: string): void {
		this.phase = "warming";
		this.warmingMessage = message;
		this.ensureAnimating();
		this.ui?.requestRender();
	}

	startListening(): void {
		this.phase = "listening";
		this.levels = [];
		this.partial = "";
		this.listeningStartMs = Date.now();
		this.ensureAnimating();
		this.ui?.requestRender();
	}

	beginSilence(totalMs: number): void {
		this.phase = "silence";
		this.silenceRemainingMs = totalMs;
		this.ensureAnimating();
		this.ui?.requestRender();
	}

	/** Speech resumed before cutoff: return to the listening phase. */
	endSilence(): void {
		if (this.phase === "silence") {
			this.phase = "listening";
			this.ui?.requestRender();
		}
	}

	setTranscribing(): void {
		this.phase = "transcribing";
		this.ensureAnimating();
		this.ui?.requestRender();
	}

	// ---- data feed -------------------------------------------------------

	pushLevel(rms: number): void {
		this.levels.push(rms);
		if (this.levels.length > HISTORY_CAP) this.levels.shift();
		// Low levels keep flowing during the silence window; only a loud one means
		// the speaker resumed and the countdown should be abandoned.
		if (this.phase === "silence" && rms > RESUME_RMS) this.endSilence();
		this.ui?.requestRender();
	}

	setPartial(text: string): void {
		this.partial = text;
		if (this.phase === "silence") this.endSilence();
		this.ui?.requestRender();
	}

	// ---- rendering -------------------------------------------------------

	private renderWaveform(width: number): string {
		const cells = Math.max(1, width - 2);
		const recent = this.levels.slice(-cells);
		const pad = cells - recent.length;
		const glyphs = " ".repeat(Math.max(0, pad)) + recent.map(levelGlyph).join("");
		return this.phase === "silence" ? theme.fg("dim", glyphs) : theme.fg("accent", glyphs);
	}

	private renderPartial(width: number): string | undefined {
		if (!this.partial) return undefined;
		const body = clampTail(this.partial, Math.max(1, width - 4));
		const cursor = this.phase === "transcribing" ? "" : theme.blink(theme.fg("accent", "▏"));
		return ` ${theme.fg("muted", "›")} ${theme.fg("dim", body)}${cursor}`;
	}

	private elapsedLabel(): string {
		const secs = Math.floor((Date.now() - this.listeningStartMs) / 1000);
		const m = Math.floor(secs / 60);
		const s = secs % 60;
		return `${m}:${s.toString().padStart(2, "0")}`;
	}

	render(width: number): string[] {
		const spinner = SPINNER_FRAMES[this.frame] ?? SPINNER_FRAMES[0]!;
		const footer = theme.fg("dim", ` ${this.cancelHint}`);
		const lines: string[] = [""];

		if (this.phase === "warming") {
			lines.push(` ${theme.fg("accent", spinner)} ${theme.fg("muted", this.warmingMessage)}`);
			lines.push(footer);
			return lines;
		}

		if (this.phase === "transcribing") {
			lines.push(` ${theme.fg("accent", spinner)} ${theme.fg("accent", "Transcribing…")}`);
			const partial = this.renderPartial(width);
			if (partial) lines.push(partial);
			lines.push(footer);
			return lines;
		}

		// listening / silence
		const mic = theme.fg("error", "🎙");
		let header = `${mic}  ${theme.fg("accent", "Listening")} ${theme.fg("dim", `· ${this.elapsedLabel()}`)}`;
		if (this.phase === "silence") {
			const remaining = (this.silenceRemainingMs / 1000).toFixed(1);
			header += ` ${theme.fg("warning", `· cutting off in ${remaining}s`)}`;
		}
		lines.push(` ${header}`);
		lines.push(` ${this.renderWaveform(width)}`);
		const partial = this.renderPartial(width);
		if (partial) lines.push(partial);
		lines.push(footer);
		return lines;
	}
}
