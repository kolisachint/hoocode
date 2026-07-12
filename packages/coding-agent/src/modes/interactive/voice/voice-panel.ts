/**
 * Multi-line voice-input status panel.
 *
 * Renders the live state of a `voicetools serve` capture across its phases so
 * the user always sees what's happening:
 *
 * ```text
 *  ● Listening · 0:03                                    esc cancel
 *  ▁▂▃▅▇▆▄▂▁▂▄▆▇▅▃▂▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁    ← level history, left-anchored on a track
 *  › and so my fellow ameri▏           ← live partial transcript (PARTIAL)
 * ```
 *
 * Phases:
 *  - `warming`      spinner while the binary resolves / model loads; shows a
 *                   real progress bar when a download reports byte counts
 *  - `listening`    recording dot + elapsed timer + waveform + partial text
 *  - `silence`      trailing-silence countdown; waveform dims
 *  - `transcribing` spinner while the final decode runs
 *
 * The component self-animates (spinner, timer, countdown) on an internal timer
 * and calls `ui.requestRender()`; the owner feeds it protocol events via
 * `pushLevel` / `setPartial` / `beginSilence` / `setDownloadProgress` etc. and
 * disposes it on collapse.
 */

import type { Component, TUI } from "@kolisachint/hoocode-tui";
import { truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import { theme } from "../theme/theme.js";

export type VoicePanelPhase = "warming" | "listening" | "silence" | "transcribing";

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
// 8 filled levels; index 0 renders as a baseline dot so an empty history still
// reads as "a track", not a blank line.
const WAVE_LEVELS = ["▁", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const TICK_MS = 100;
const HISTORY_CAP = 256;
// RMS from voicetools' vad::rms sits well below 1.0 for speech; this gain maps a
// normal speaking level to roughly the top of the meter without clipping constantly.
const LEVEL_GAIN = 6;
// During trailing silence the binary keeps emitting low LEVELs; only an RMS above
// this counts as the speaker resuming, so we don't cancel the countdown on noise.
const RESUME_RMS = 0.02;
/** Cells in the download progress bar shown while the voicetools binary fetches. */
const PROGRESS_CELLS = 20;

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

/** `left` flush left, `right` flush right when it fits; otherwise just `left`. */
function lineWithHint(
	width: number,
	leftPlain: string,
	leftStyled: string,
	hintPlain: string,
	hintStyled: string,
): string {
	const lw = visibleWidth(leftPlain);
	if (hintPlain && lw + 2 + visibleWidth(hintPlain) <= width) {
		return leftStyled + " ".repeat(width - lw - visibleWidth(hintPlain)) + hintStyled;
	}
	if (lw <= width) return leftStyled;
	return truncateToWidth(leftStyled, width, theme.fg("dim", "…"));
}

function formatMb(bytes: number): string {
	return `${Math.round(bytes / (1024 * 1024))} MB`;
}

export class VoicePanel implements Component {
	private phase: VoicePanelPhase = "warming";
	private frame = 0;
	private levels: number[] = [];
	private partial = "";
	private listeningStartMs = 0;
	private silenceRemainingMs = 0;
	private warmingMessage = "Warming up voice input…";
	private warmingDetail: string | undefined;
	private downloadReceived = 0;
	private downloadTotal: number | null = null;
	private downloading = false;
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

	setWarming(message: string, detail?: string): void {
		this.phase = "warming";
		this.warmingMessage = message;
		this.warmingDetail = detail;
		this.ensureAnimating();
		this.ui?.requestRender();
	}

	/**
	 * Live byte counts while the voicetools binary downloads (first run only).
	 * Turns the warming spinner into a determinate progress bar.
	 */
	setDownloadProgress(receivedBytes: number, totalBytes: number | null): void {
		this.downloading = true;
		this.downloadReceived = receivedBytes;
		this.downloadTotal = totalBytes;
		this.ui?.requestRender();
	}

	startListening(): void {
		this.phase = "listening";
		this.downloading = false;
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

	/**
	 * Level history, LEFT-anchored: the meter grows from the left edge and the
	 * unfilled remainder renders as a dim baseline track, so the first seconds of
	 * a capture read as a meter filling — not a cluster of glyphs floating at the
	 * far right edge over a void (the old right-anchored layout). Once the
	 * history exceeds the width it scrolls, keeping the newest samples visible.
	 */
	private renderWaveform(width: number): string {
		const cells = Math.max(1, width - 2);
		const recent = this.levels.slice(-cells);
		const wave = recent.map(levelGlyph).join("");
		const track = "▁".repeat(Math.max(0, cells - recent.length));
		const waveStyled = this.phase === "silence" ? theme.fg("dim", wave) : theme.fg("accent", wave);
		return waveStyled + theme.fg("dim", track);
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

	/** Determinate download bar: `·` fill over a dim track with % and MB counts. */
	private renderDownloadBar(): string {
		const total = this.downloadTotal;
		if (total === null || total <= 0) {
			return ` ${theme.fg("dim", `downloaded ${formatMb(this.downloadReceived)}…`)}`;
		}
		const ratio = Math.max(0, Math.min(1, this.downloadReceived / total));
		const filled = Math.round(ratio * PROGRESS_CELLS);
		const bar = theme.fg("accent", "·".repeat(filled)) + theme.fg("dim", "·".repeat(PROGRESS_CELLS - filled));
		const pct = `${Math.round(ratio * 100)}%`;
		const sizes = `${formatMb(this.downloadReceived)} / ${formatMb(total)}`;
		return ` ${bar} ${theme.fg("muted", pct)} ${theme.fg("dim", `· ${sizes}`)}`;
	}

	render(width: number): string[] {
		const spinner = SPINNER_FRAMES[this.frame] ?? SPINNER_FRAMES[0]!;
		const hintPlain = this.cancelHint;
		const hintStyled = theme.fg("dim", this.cancelHint);
		const lines: string[] = [""];

		if (this.phase === "warming") {
			const headPlain = ` ${spinner} ${this.warmingMessage}`;
			const headStyled = ` ${theme.fg("accent", spinner)} ${theme.fg("text", this.warmingMessage)}`;
			lines.push(lineWithHint(width, headPlain, headStyled, hintPlain, hintStyled));
			if (this.downloading) {
				lines.push(this.renderDownloadBar());
			}
			if (this.warmingDetail) {
				lines.push(`   ${theme.fg("dim", this.warmingDetail)}`);
			}
			return lines;
		}

		if (this.phase === "transcribing") {
			const headPlain = ` ${spinner} Transcribing…`;
			const headStyled = ` ${theme.fg("accent", spinner)} ${theme.fg("accent", "Transcribing…")}`;
			lines.push(lineWithHint(width, headPlain, headStyled, hintPlain, hintStyled));
			const partial = this.renderPartial(width);
			if (partial) lines.push(partial);
			return lines;
		}

		// listening / silence. A single-cell recording dot (●, error red = "live")
		// replaces the old emoji mic, whose emoji-presentation width drift was the
		// reason for a VS16 workaround; ● needs no such care.
		const dot = theme.fg("error", "●");
		let headPlain = ` ● Listening · ${this.elapsedLabel()}`;
		let headStyled = ` ${dot} ${theme.fg("text", "Listening")} ${theme.fg("dim", `· ${this.elapsedLabel()}`)}`;
		if (this.phase === "silence") {
			const remaining = (this.silenceRemainingMs / 1000).toFixed(1);
			headPlain += ` · cutting off in ${remaining}s`;
			headStyled += ` ${theme.fg("warning", `· cutting off in ${remaining}s`)}`;
		}
		lines.push(lineWithHint(width, headPlain, headStyled, hintPlain, hintStyled));
		lines.push(` ${this.renderWaveform(width)}`);
		const partial = this.renderPartial(width);
		if (partial) lines.push(partial);
		return lines;
	}
}
