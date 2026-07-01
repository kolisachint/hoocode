import { createInterface, type Interface } from "node:readline";
import { type ChildProcess, spawn } from "child_process";

/**
 * Drives the external `voicetools` binary and streams its stdout line protocol
 * into callbacks. One line per event on stdout (stderr is free for debug logs):
 *
 * ```text
 * STATUS recording        # state transition (recording | transcribing | ...)
 * SEGMENT hello world     # a chunk of decoded text
 * DONE                    # finished successfully
 * ERROR no model found    # fatal error; process exits non-zero
 * ```
 *
 * `voicetools serve` (see `VoiceDaemon` below) reuses this same line protocol
 * plus a few daemon-only events (READY, LEVEL, PHASE).
 *
 * The caller wires `onSegment` to inject text into the editor (via bracketed
 * paste) and `onStatus` / `onError` to surface feedback.
 */

export type VoiceStatus = "recording" | "transcribing" | "done" | "listening" | string;

export interface VoiceTranscribeHandlers {
	/** A decoded chunk of text. Injected into the editor by the caller. */
	onSegment: (text: string) => void;
	/** A state transition reported by the binary, or `"done"` on completion. */
	onStatus: (status: VoiceStatus) => void;
	/** A fatal error: spawn failure, protocol ERROR line, or non-zero exit. */
	onError: (message: string) => void;
}

/**
 * A running voice-transcribe session. Call `stop()` to cancel early (e.g. the
 * user pressing the shortcut again). `stop()` is idempotent.
 */
export interface VoiceSession {
	stop(): void;
	readonly running: boolean;
}

/** Build a friendly message for a spawn failure, calling out a missing binary. */
function describeSpawnError(err: unknown, bin: string): string {
	if (err && typeof err === "object" && (err as NodeJS.ErrnoException).code === "ENOENT") {
		return `voicetools binary not found (tried "${bin}"). Install it or set VOICETOOLS_BIN to its path.`;
	}
	return err instanceof Error ? err.message : String(err);
}

/**
 * Spawn `voicetools transcribe` for a single capture. This is the fallback
 * path for binaries that don't support `serve` (see `VoiceDaemon`): every
 * push-to-talk press pays the model load cold start.
 */
export function startVoiceTranscribe(bin: string, handlers: VoiceTranscribeHandlers): VoiceSession {
	let proc: ChildProcess;
	try {
		proc = spawn(bin, ["transcribe"], {
			stdio: ["ignore", "pipe", "ignore"],
		});
	} catch (err) {
		handlers.onError(describeSpawnError(err, bin));
		return { stop: () => {}, running: false };
	}

	let stopped = false;
	let finished = false;
	let rl: Interface | undefined;

	const finish = (): void => {
		if (finished) return;
		finished = true;
		rl?.close();
	};

	if (!proc.stdout) {
		proc.kill();
		handlers.onError("failed to capture voicetools stdout");
		return { stop: () => {}, running: false };
	}

	rl = createInterface({ input: proc.stdout });
	rl.on("line", (line) => {
		if (line.startsWith("STATUS ")) {
			handlers.onStatus(line.slice(7).trim());
		} else if (line.startsWith("SEGMENT ")) {
			handlers.onSegment(line.slice(8));
		} else if (line === "DONE") {
			handlers.onStatus("done");
			finish();
		} else if (line.startsWith("ERROR ")) {
			handlers.onError(line.slice(6).trim());
			finish();
		}
	});

	proc.on("error", (err) => {
		if (stopped || finished) return;
		finish();
		handlers.onError(describeSpawnError(err, bin));
	});

	proc.on("close", (code) => {
		const wasFinished = finished;
		finish();
		if (stopped || wasFinished) return;
		if (code && code !== 0) {
			handlers.onError(`voicetools exited with code ${code}`);
		}
	});

	return {
		stop: () => {
			if (stopped) return;
			stopped = true;
			finish();
			proc.kill();
		},
		get running() {
			return !finished && !stopped;
		},
	};
}

/**
 * Handlers for a persistent `voicetools serve` daemon. Extends the base
 * transcribe handlers with the daemon-only events:
 *  - `onReady`   fires once after models finish loading.
 *  - `onLevel`   per-audio-chunk RMS, for a live meter/waveform.
 *  - `onPhase`   phase markers (e.g. `"silence"` when trailing silence begins).
 *  - `onPartial` interim transcript while the user speaks — the FULL growing
 *                hypothesis each time (supersedes the previous), never committed.
 *  - `onFinal`   the complete committed transcript for the utterance, emitted
 *                once before DONE — this is the text to inject into the editor.
 *  - `onCrash`   the process died after having been ready (caller should drop
 *                the reference and respawn lazily on the next push-to-talk).
 */
export interface VoiceDaemonHandlers extends VoiceTranscribeHandlers {
	onReady?: () => void;
	onLevel?: (rms: number) => void;
	onPhase?: (phase: string) => void;
	onPartial?: (text: string) => void;
	onFinal?: (text: string) => void;
	onCrash?: (message: string) => void;
}

/**
 * Outcome of {@link VoiceDaemon.spawn}. `reason: "unsupported"` means the
 * process exited before READY with no ERROR line at all — the signature of
 * an old binary rejecting the unrecognized `serve` subcommand — and the
 * caller should silently fall back to `startVoiceTranscribe`. `reason:
 * "error"` means a genuine ERROR line (or OS-level spawn failure) was seen;
 * `handlers.onError` has already been called with it, and the caller should
 * surface that (not retry with the legacy path, which would just hit the
 * same failure) while leaving daemon mode available to retry next press.
 */
export type VoiceDaemonSpawnResult = { ok: true; daemon: VoiceDaemon } | { ok: false; reason: "unsupported" | "error" };

/**
 * A persistent `voicetools serve` process: models are loaded once and stay
 * warm across captures. Only one capture runs at a time; call `startCapture`
 * to open the mic and `cancel` to stop early. `spawn` doubles as the support
 * probe for old binaries (see {@link VoiceDaemonSpawnResult}).
 */
export class VoiceDaemon {
	private closed = false;

	private constructor(
		private readonly proc: ChildProcess,
		private readonly handlers: VoiceDaemonHandlers,
	) {}

	get isReady(): boolean {
		return !this.closed;
	}

	static spawn(bin: string, handlers: VoiceDaemonHandlers): Promise<VoiceDaemonSpawnResult> {
		return new Promise((resolve) => {
			let proc: ChildProcess;
			try {
				proc = spawn(bin, ["serve"], { stdio: ["pipe", "pipe", "ignore"] });
			} catch (err) {
				handlers.onError(describeSpawnError(err, bin));
				resolve({ ok: false, reason: "error" });
				return;
			}

			if (!proc.stdout || !proc.stdin) {
				proc.kill();
				handlers.onError("failed to open voicetools serve stdio");
				resolve({ ok: false, reason: "error" });
				return;
			}

			let settled = false;
			let daemon: VoiceDaemon | undefined;
			let sawPreReadyError = false;
			const rl = createInterface({ input: proc.stdout });

			const fail = (reason: "unsupported" | "error"): void => {
				if (settled) return;
				settled = true;
				rl.close();
				resolve({ ok: false, reason });
			};

			rl.on("line", (line) => {
				if (daemon) {
					daemon.handleLine(line);
					return;
				}
				if (settled) return;
				if (line === "READY") {
					settled = true;
					daemon = new VoiceDaemon(proc, handlers);
					handlers.onReady?.();
					resolve({ ok: true, daemon });
					return;
				}
				if (line.startsWith("ERROR ")) {
					// Loading can fail before READY (e.g. no model installed yet).
					// Surface it now; the process still exits right after.
					sawPreReadyError = true;
					handlers.onError(line.slice(6).trim());
				}
			});

			proc.on("error", (err) => {
				if (daemon) {
					if (!daemon.closed) {
						daemon.closed = true;
						handlers.onCrash?.(describeSpawnError(err, bin));
					}
					return;
				}
				if (!sawPreReadyError) handlers.onError(describeSpawnError(err, bin));
				fail("error");
			});

			proc.on("close", (code) => {
				if (daemon) {
					if (!daemon.closed) {
						daemon.closed = true;
						handlers.onCrash?.(`voicetools serve exited with code ${code ?? "unknown"}`);
					}
					return;
				}
				fail(sawPreReadyError ? "error" : "unsupported");
			});
		});
	}

	private handleLine(line: string): void {
		if (line.startsWith("STATUS ")) {
			this.handlers.onStatus(line.slice(7).trim());
		} else if (line.startsWith("PARTIAL ")) {
			this.handlers.onPartial?.(line.slice(8));
		} else if (line.startsWith("FINAL ")) {
			this.handlers.onFinal?.(line.slice(6));
		} else if (line.startsWith("SEGMENT ")) {
			this.handlers.onSegment(line.slice(8));
		} else if (line.startsWith("LEVEL ")) {
			const rms = Number.parseFloat(line.slice(6).trim());
			if (!Number.isNaN(rms)) this.handlers.onLevel?.(rms);
		} else if (line.startsWith("PHASE ")) {
			this.handlers.onPhase?.(line.slice(6).trim());
		} else if (line === "DONE") {
			this.handlers.onStatus("done");
		} else if (line.startsWith("ERROR ")) {
			this.handlers.onError(line.slice(6).trim());
		}
	}

	/** Begin a capture: opens the mic, streams SEGMENTs, ends with DONE. */
	startCapture(): void {
		if (this.closed) return;
		this.proc.stdin?.write("START\n");
	}

	/** Cancel the in-flight capture, if any. Idempotent. */
	cancel(): void {
		if (this.closed) return;
		this.proc.stdin?.write("CANCEL\n");
	}

	/** Ask the daemon to exit gracefully, force-killing if it doesn't within 1s. */
	shutdown(): void {
		if (this.closed) return;
		this.closed = true;
		try {
			this.proc.stdin?.write("SHUTDOWN\n");
		} catch {
			// stdin may already be gone (process died); force-kill below covers it.
		}
		const proc = this.proc;
		const killTimer = setTimeout(() => {
			if (!proc.killed) proc.kill();
		}, 1000);
		proc.once("close", () => clearTimeout(killTimer));
	}
}
