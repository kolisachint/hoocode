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
 * transcribe handlers with the daemon-only events: `onReady` fires once after
 * models finish loading, `onLevel`/`onPhase` drive a live UI meter and the
 * trailing-silence indicator, and `onCrash` fires if the process dies after
 * having been ready (the caller should drop the reference and respawn lazily
 * on the next push-to-talk).
 */
export interface VoiceDaemonHandlers extends VoiceTranscribeHandlers {
	onReady?: () => void;
	onLevel?: (rms: number) => void;
	onPhase?: (phase: string) => void;
	onCrash?: (message: string) => void;
}

/**
 * A persistent `voicetools serve` process: models are loaded once and stay
 * warm across captures. Only one capture runs at a time; call `startCapture`
 * to open the mic and `cancel` to stop early. `spawn` doubles as the support
 * probe for old binaries: if the process exits/errors before emitting READY
 * (e.g. an unrecognized `serve` subcommand), it resolves to `undefined` so
 * the caller can fall back to `startVoiceTranscribe`.
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

	static spawn(bin: string, handlers: VoiceDaemonHandlers): Promise<VoiceDaemon | undefined> {
		return new Promise((resolve) => {
			let proc: ChildProcess;
			try {
				proc = spawn(bin, ["serve"], { stdio: ["pipe", "pipe", "ignore"] });
			} catch (err) {
				handlers.onError(describeSpawnError(err, bin));
				resolve(undefined);
				return;
			}

			if (!proc.stdout || !proc.stdin) {
				proc.kill();
				handlers.onError("failed to open voicetools serve stdio");
				resolve(undefined);
				return;
			}

			let settled = false;
			let daemon: VoiceDaemon | undefined;
			const rl = createInterface({ input: proc.stdout });

			const settleUnsupported = (): void => {
				if (settled) return;
				settled = true;
				rl.close();
				resolve(undefined);
			};

			rl.on("line", (line) => {
				if (daemon) {
					daemon.handleLine(line);
					return;
				}
				if (!settled && line === "READY") {
					settled = true;
					daemon = new VoiceDaemon(proc, handlers);
					handlers.onReady?.();
					resolve(daemon);
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
				settleUnsupported();
			});

			proc.on("close", (code) => {
				if (daemon) {
					if (!daemon.closed) {
						daemon.closed = true;
						handlers.onCrash?.(`voicetools serve exited with code ${code ?? "unknown"}`);
					}
					return;
				}
				settleUnsupported();
			});
		});
	}

	private handleLine(line: string): void {
		if (line.startsWith("STATUS ")) {
			this.handlers.onStatus(line.slice(7).trim());
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
