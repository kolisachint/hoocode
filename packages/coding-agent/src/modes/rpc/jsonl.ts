import type { Readable } from "node:stream";
import { StringDecoder } from "node:string_decoder";

/**
 * Serialize a single strict JSONL record.
 *
 * Framing is LF-only. Payload strings may contain other Unicode separators such as
 * U+2028 and U+2029. Clients must split records on `\n` only.
 */
export function serializeJsonLine(value: unknown): string {
	return `${JSON.stringify(value)}\n`;
}

export interface JsonlLineReaderOptions {
	/**
	 * Cap (in UTF-16 code units) on the un-terminated line buffer. A line that
	 * grows past this without a newline is dropped in its entirety (the rest of
	 * it is skipped up to the next newline) instead of being buffered without
	 * bound — a misbehaving writer must not be able to OOM the reader. Omit for
	 * an unbounded buffer (trusted peers, e.g. the RPC channel).
	 */
	maxBuffer?: number;
}

/**
 * Attach an LF-only JSONL reader to a stream.
 *
 * This intentionally does not use Node readline. Readline splits on additional
 * Unicode separators that are valid inside JSON strings and therefore does not
 * implement strict JSONL framing.
 *
 * The reader detaches itself on stream `error` (which would otherwise crash the
 * process as an unhandled 'error' event) and flushes + detaches on `close`, so
 * a child that dies mid-line cannot leak listeners or buffered data. The
 * returned function detaches manually.
 */
export function attachJsonlLineReader(
	stream: Readable,
	onLine: (line: string) => void,
	options: JsonlLineReaderOptions = {},
): () => void {
	const decoder = new StringDecoder("utf8");
	const maxBuffer = options.maxBuffer;
	let buffer = "";
	/** True while skipping the remainder of a line that overflowed maxBuffer. */
	let discarding = false;
	let flushed = false;
	let detached = false;

	const emitLine = (line: string) => {
		onLine(line.endsWith("\r") ? line.slice(0, -1) : line);
	};

	const onData = (chunk: string | Buffer) => {
		buffer += typeof chunk === "string" ? chunk : decoder.write(chunk);

		while (true) {
			const newlineIndex = buffer.indexOf("\n");
			if (newlineIndex === -1) {
				if (maxBuffer !== undefined && buffer.length > maxBuffer) {
					buffer = "";
					discarding = true;
				}
				return;
			}

			const line = buffer.slice(0, newlineIndex);
			buffer = buffer.slice(newlineIndex + 1);
			if (discarding) {
				// The extracted piece is the tail of an oversized, already-dropped line.
				discarding = false;
				continue;
			}
			emitLine(line);
		}
	};

	const flush = () => {
		if (flushed) return;
		flushed = true;
		buffer += decoder.end();
		if (buffer.length > 0 && !discarding) {
			emitLine(buffer);
		}
		buffer = "";
	};

	const detach = () => {
		if (detached) return;
		detached = true;
		stream.off("data", onData);
		stream.off("end", onEnd);
		stream.off("close", onClose);
		stream.off("error", onError);
		buffer = "";
	};

	const onEnd = () => {
		flush();
	};

	const onClose = () => {
		flush();
		detach();
	};

	const onError = () => {
		// The stream is dead; drop partial data and release the listeners. Having
		// this handler also keeps a stream 'error' from becoming an uncaught
		// exception in the parent.
		detach();
	};

	stream.on("data", onData);
	stream.on("end", onEnd);
	stream.on("close", onClose);
	stream.on("error", onError);

	return detach;
}
