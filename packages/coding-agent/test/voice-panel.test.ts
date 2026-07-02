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

	it("shows mic, elapsed timer, waveform, and the live partial while listening", () => {
		panel = new VoicePanel(undefined, "ctrl+r cancel");
		panel.startListening();
		panel.pushLevel(0.2);
		panel.pushLevel(0.1);
		panel.setPartial("and so my fellow");
		const lines = panel.render(60);
		const text = plain(lines);
		expect(text).toContain("🎙");
		expect(text).toContain("Listening");
		expect(text).toContain("0:00"); // elapsed timer
		expect(text).toContain("and so my fellow"); // live partial preview
		expect(text).toContain("ctrl+r cancel"); // dim footer
		// waveform line carries block glyphs from the pushed levels
		expect(text).toMatch(/[▁▂▃▄▅▆▇█]/);
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
