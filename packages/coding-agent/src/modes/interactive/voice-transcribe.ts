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
 * The caller wires `onSegment` to inject text into the editor (via bracketed
 * paste) and `onStatus` / `onError` to surface feedback.
 */

export type VoiceStatus = "recording" | "transcribing" | "done" | string;

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

export function startVoiceTranscribe(bin: string, handlers: VoiceTranscribeHandlers): VoiceSession {
	let proc: ChildProcess;
	try {
		proc = spawn(bin, ["transcribe"], {
			stdio: ["ignore", "pipe", "ignore"],
		});
	} catch (err) {
		handlers.onError(err instanceof Error ? err.message : String(err));
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
		handlers.onError(err.message);
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
