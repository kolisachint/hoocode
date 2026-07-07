/**
 * Voice-to-text capture control (ctrl+r / app.input.voiceTranscribe).
 *
 * Prefers a persistent `voicetools serve` daemon: the first press probes for
 * support and (if found) loads models once, showing a "warming up" spinner;
 * every later press reuses the already-warm daemon and jumps straight to
 * Listening. Binaries without `serve` support fall back to spawning
 * `voicetools transcribe` per press. Pressing the shortcut again while
 * listening (or while still warming up) cancels.
 */

import type { Container, TUI } from "@kolisachint/hoocode-tui";
import { ensureTool } from "../../../utils/tools-manager.js";
import { keyHint } from "../components/keybinding-hints.js";
import { VoicePanel } from "./voice-panel.js";
import { startVoiceTranscribe, VoiceDaemon, type VoiceDaemonHandlers, type VoiceSession } from "./voice-transcribe.js";

// Trailing-silence window: how long a pause while speaking lasts before the
// capture auto-stops. Passed to `voicetools serve` via `--silence-ms` (see
// VoiceDaemon.spawn) so the binary's real cutoff matches the on-screen
// countdown this same value drives. Every utterance pays this in full before
// finalization can start, so keep it close to the binary's 600ms default —
// a slightly longer window tolerates brief mid-sentence pauses without
// taxing every dictation with seconds of dead air.
const VOICE_SILENCE_MS = 800;
/** How long to keep the warm voice model in memory after the last capture
 * completes. The daemon auto-shuts down after this window, releasing the
 * ~900 MB resident model; the next ctrl+r pays a cold-start respawn cost.
 * Kept long enough that normal pacing between dictations (reading a reply,
 * typing) doesn't repeatedly hit the multi-second cold start. */
const VOICE_IDLE_TIMEOUT_MS = 300_000;
const VOICE_UNAVAILABLE_MESSAGE =
	"Voice input failed: voicetools binary unavailable and could not be downloaded. " +
	"Install it, set VOICETOOLS_BIN, or ensure a published release exists for this platform.";

/** The slice of the interactive mode the voice feature needs. */
export interface VoiceControllerDeps {
	ui: TUI;
	/** The status area below the chat; the voice panel mounts here. */
	statusContainer: Container;
	/** Feed raw input (bracketed paste) to the prompt editor. */
	sendEditorInput(data: string): void;
	showError(message: string): void;
}

export class VoiceController {
	/** In-flight legacy (`voicetools transcribe`) capture. */
	private session: VoiceSession | undefined;
	// Persistent `voicetools serve` process, once probed successfully. Stays alive
	// between captures so the model stays warm.
	private daemon: VoiceDaemon | undefined;
	/** Set when the binary rejected `serve`; future presses use the legacy path. */
	private daemonUnsupported = false;
	/** True while the voicetools binary is being resolved/downloaded before a session starts. */
	private starting = false;
	/** True while a capture is active (listening or transcribing). */
	private active = false;
	/** The status panel while voice input is active. */
	private panel: VoicePanel | undefined;

	constructor(private readonly deps: VoiceControllerDeps) {}

	/** Toggle voice-to-text capture. */
	toggle(): void {
		if (this.active) {
			if (this.daemon?.isReady) {
				this.daemon.cancel();
			} else {
				this.session?.stop();
			}
			this.session = undefined;
			this.active = false;
			this.resetUI();
			return;
		}

		if (this.starting) {
			// A second press while resolving/warming up: honour the cancel. Any
			// daemon that finishes loading afterwards is kept warm for next time.
			this.starting = false;
			this.resetUI();
			return;
		}

		if (this.daemonUnsupported) {
			this.starting = true;
			this.showWarming("Starting voice input...");
			void this.resolveBin()
				.then((bin) => {
					if (!this.starting) return;
					this.starting = false;
					if (!bin) {
						this.resetUI();
						this.deps.showError(VOICE_UNAVAILABLE_MESSAGE);
						return;
					}
					this.beginLegacyCapture(bin);
				})
				.catch((err: unknown) => {
					this.starting = false;
					this.resetUI();
					this.deps.showError(`Voice input failed: ${err instanceof Error ? err.message : String(err)}`);
				});
			return;
		}

		if (this.daemon?.isReady) {
			this.beginDaemonCapture();
			return;
		}

		// No daemon yet: resolve the binary, then probe for `serve` support by
		// spawning it. `VoiceDaemon.spawn` doubles as the probe: it resolves to a
		// live daemon once READY arrives, to "unsupported" if the process exits
		// with no output at all (an old binary rejecting the unrecognized `serve`
		// subcommand), or to "error" if it printed a real ERROR first (e.g. no
		// model installed yet) — already surfaced via onError, so that case skips
		// the legacy fallback (it would just hit the same error) but leaves
		// daemon mode available to retry on the next press.
		this.starting = true;
		this.showWarming("Warming up voice input...");
		void this.resolveBin()
			.then(async (bin) => {
				if (!bin) {
					this.starting = false;
					this.resetUI();
					this.deps.showError(VOICE_UNAVAILABLE_MESSAGE);
					return;
				}
				const result = await VoiceDaemon.spawn(bin, this.buildDaemonHandlers(), {
					silenceMs: VOICE_SILENCE_MS,
					idleTimeoutMs: VOICE_IDLE_TIMEOUT_MS,
				});
				if (!result.ok) {
					if (result.reason === "unsupported") {
						this.daemonUnsupported = true;
						if (!this.starting) {
							this.resetUI();
							return;
						}
						this.starting = false;
						this.beginLegacyCapture(bin);
						return;
					}
					this.starting = false;
					this.resetUI();
					return;
				}
				this.daemon = result.daemon;
				if (!this.starting) {
					// Cancelled while warming up: keep the loaded daemon warm for next time.
					this.resetUI();
					return;
				}
				this.starting = false;
				this.beginDaemonCapture();
			})
			.catch((err: unknown) => {
				this.starting = false;
				this.resetUI();
				this.deps.showError(`Voice input failed: ${err instanceof Error ? err.message : String(err)}`);
			});
	}

	/** Stop any capture, shut down the daemon, and drop the panel. */
	dispose(): void {
		if (this.session?.running) {
			this.session.stop();
			this.session = undefined;
		}
		this.daemon?.shutdown();
		this.daemon = undefined;
		if (this.panel) {
			this.panel.dispose();
			this.panel = undefined;
		}
		this.active = false;
		this.starting = false;
	}

	/**
	 * Build the (stable, reused-across-captures) handlers for the daemon.
	 *
	 * Every handler bails out once `active` is false: CANCEL is a soft
	 * request (unlike the legacy path's `proc.kill()`, it doesn't sever the
	 * pipe), so the daemon can still have a trailing PARTIAL/FINAL/DONE for the
	 * just-cancelled capture in flight when the user presses cancel. Without
	 * this guard that stale text would land in the editor after the panel had
	 * already collapsed.
	 *
	 * v0.1.4 serve streams `PARTIAL <full growing hypothesis>` (live preview,
	 * never committed) and ends with a single `FINAL <complete text>` (the one
	 * commit to the editor). `SEGMENT` is still handled for the legacy
	 * transcribe path and any binary that streams committed chunks directly.
	 */
	private buildDaemonHandlers(): VoiceDaemonHandlers {
		return {
			onSegment: (text) => {
				if (!this.active) return;
				this.commitText(text);
			},
			onPartial: (text) => {
				if (!this.active) return;
				this.panel?.setPartial(text);
			},
			onFinal: (text) => {
				if (!this.active) return;
				this.commitText(text);
			},
			onStatus: (status) => {
				if (!this.active) return;
				if (status === "done") {
					this.active = false;
					this.resetUI();
					return;
				}
				if (status === "transcribing") {
					this.panel?.setTranscribing();
				} else if (status === "listening") {
					this.panel?.startListening();
				}
			},
			onLevel: (rms) => {
				if (!this.active) return;
				this.panel?.pushLevel(rms);
			},
			onPhase: (phase) => {
				if (!this.active) return;
				if (phase === "silence") {
					this.panel?.beginSilence(VOICE_SILENCE_MS);
				} else {
					this.panel?.endSilence();
				}
			},
			onError: (message) => {
				this.active = false;
				this.resetUI();
				this.deps.showError(`Voice input failed: ${message}`);
			},
			onCrash: (message) => {
				this.active = false;
				this.daemon = undefined;
				this.resetUI();
				this.deps.showError(`Voice input daemon crashed: ${message}. It will restart on next use.`);
			},
			onIdle: () => {
				// Daemon auto-shut down after idle timeout: drop the reference so the
				// next ctrl+r respawns (cold start). No user-facing message — this is
				// an expected memory-reclamation event, not an error.
				this.daemon = undefined;
			},
		};
	}

	/** Inject decoded text into the editor via bracketed paste (with a trailing space). */
	private commitText(text: string): void {
		this.deps.sendEditorInput(`\x1b[200~${text} \x1b[201~`);
	}

	private beginDaemonCapture(): void {
		if (!this.daemon?.isReady) return;
		this.active = true;
		this.showPanel().startListening();
		this.daemon.startCapture();
	}

	private beginLegacyCapture(bin: string): void {
		this.active = true;
		const panel = this.showPanel();
		panel.startListening();
		this.session = startVoiceTranscribe(bin, {
			onStatus: (status) => {
				if (status === "done") {
					this.active = false;
					this.session = undefined;
					this.resetUI();
					return;
				}
				// Old binaries emit no LEVEL/PARTIAL, so the panel shows a spinner
				// for the batch phases; committed words still stream into the editor.
				if (status === "transcribing") panel.setTranscribing();
			},
			onSegment: (text) => {
				this.commitText(text);
			},
			onError: (message) => {
				this.active = false;
				this.session = undefined;
				this.resetUI();
				this.deps.showError(`Voice input failed: ${message}`);
			},
		});
	}

	/** Resolve the `voicetools` binary path. Prefers an explicit VOICETOOLS_BIN
	 * override, otherwise resolves via the managed tools manager (bin dir / PATH /
	 * download from the published release). Returns undefined when unavailable.
	 */
	private async resolveBin(): Promise<string | undefined> {
		const override = process.env.VOICETOOLS_BIN?.trim();
		if (override) return override;
		return ensureTool("voicetools", true);
	}

	/** Create (or reuse) the voice panel and mount it in the status container. */
	private showPanel(): VoicePanel {
		if (!this.panel) {
			this.deps.statusContainer.clear();
			this.panel = new VoicePanel(this.deps.ui, keyHint("app.input.voiceTranscribe", "cancel"));
			this.deps.statusContainer.addChild(this.panel);
		}
		this.deps.ui.requestRender();
		return this.panel;
	}

	private showWarming(message: string): void {
		this.showPanel().setWarming(message);
	}

	/** Collapse the voice panel back to nothing (idle) and stop its animation. */
	private resetUI(): void {
		if (this.panel) {
			this.panel.dispose();
			this.panel = undefined;
		}
		this.deps.statusContainer.clear();
		this.deps.ui.requestRender();
	}
}
