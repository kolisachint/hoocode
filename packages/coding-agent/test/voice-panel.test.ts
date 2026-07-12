import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initTheme } from "../src/modes/interactive/theme/theme.js";
import { VoicePanel } from "../src/modes/interactive/voice/voice-panel.js";

beforeAll(() => {
	initTheme("dark");
});

/** Strip ANSI SGR/color escapes so we can assert on the visible text. */
function plain(lines: string[]): string {
	return lines.join("\n").replace(/\x1b\[[0-9;]*m/g, "");
}

describe("VoicePanel", () => {
	let panel: VoicePanel | undefined;

	afterEach(() => {
		panel?.dispose();
		panel = undefined;
	});

	it("renders a spinner + message in the warming phase", () => {
		panel = new VoicePanel(undefined, "ctrl+r cancel");
		panel.setWarming("Warming up voice input…");
		const text = plain(panel.render(60));
		expect(text).toContain("Warming up voice input…");
		expect(text).toContain("ctrl+r cancel");
	});

	it("shows the recording dot, elapsed timer, waveform, and the live partial while listening", () => {
		panel = new VoicePanel(undefined, "ctrl+r cancel");
		panel.startListening();
		panel.pushLevel(0.2);
		panel.pushLevel(0.1);
		panel.setPartial("and so my fellow");
		const lines = panel.render(60);
		const text = plain(lines);
		expect(text).toContain("●"); // single-cell recording dot (replaced the emoji mic)
		expect(text).toContain("Listening");
		expect(text).toContain("0:00"); // elapsed timer
		expect(text).toContain("and so my fellow"); // live partial preview
		expect(text).toContain("ctrl+r cancel"); // hint, right-aligned on the header
		// waveform line carries block glyphs from the pushed levels
		expect(text).toMatch(/[▁▂▃▄▅▆▇█]/);
	});

	it("left-anchors the waveform and fills the remainder with a baseline track", () => {
		panel = new VoicePanel(undefined, "ctrl+r cancel");
		panel.startListening();
		panel.pushLevel(0.5);
		panel.pushLevel(0.5);
		const lines = panel.render(40);
		const wave = lines.find((l) => /[▂▃▄▅▆▇█]/.test(l.replace(/\x1b\[[0-9;]*m/g, "")));
		expect(wave).toBeDefined();
		const visible = (wave ?? "").replace(/\x1b\[[0-9;]*m/g, "");
		// The two loud samples sit at the LEFT edge (after the 1-cell indent), and
		// the rest of the row is a ▁ baseline track — never a run of blank cells.
		expect(visible.slice(1, 3)).toMatch(/[▂▃▄▅▆▇█]{2}/);
		expect(visible).not.toMatch(/ {4,}/);
		expect(visible).toContain("▁▁▁");
	});

	it("shows a determinate download bar while the binary fetches", () => {
		panel = new VoicePanel(undefined, "ctrl+r cancel");
		panel.setWarming("Starting voice input…", "first run downloads the voicetools binary and speech model");
		panel.setDownloadProgress(450 * 1024 * 1024, 900 * 1024 * 1024);
		const text = plain(panel.render(80));
		expect(text).toContain("50%");
		expect(text).toContain("450 MB / 900 MB");
		expect(text).toContain("first run downloads");
	});

	it("shows a shrinking silence countdown, and abandons it when a loud level resumes", () => {
		panel = new VoicePanel(undefined, "ctrl+r cancel");
		panel.startListening();
		panel.beginSilence(600);
		expect(plain(panel.render(60))).toContain("cutting off in 0.6s");

		// A low level during silence does NOT resume (it's just background noise).
		panel.pushLevel(0.001);
		expect(plain(panel.render(60))).toContain("cutting off");

		// A loud level means the speaker resumed: back to plain Listening.
		panel.pushLevel(0.3);
		expect(plain(panel.render(60))).not.toContain("cutting off");
	});

	it("switches to a Transcribing spinner and keeps the partial visible", () => {
		panel = new VoicePanel(undefined, "ctrl+r cancel");
		panel.startListening();
		panel.setPartial("hello world");
		panel.setTranscribing();
		const text = plain(panel.render(60));
		expect(text).toContain("Transcribing");
		expect(text).toContain("hello world");
	});

	it("does not leak timers after dispose()", () => {
		panel = new VoicePanel(undefined, "ctrl+r cancel");
		panel.startListening();
		panel.dispose();
		// A second dispose is a harmless no-op (idempotent).
		panel.dispose();
		panel = undefined;
	});
});
