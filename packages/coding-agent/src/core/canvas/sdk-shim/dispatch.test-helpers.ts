/**
 * Test doubles for the shim's stdio streams, so dispatch can be exercised without
 * forking a child process. Kept beside the shim rather than under `test/` because
 * what these implement is the shim's own stream contract.
 */

import type { CanvasShimStreams } from "./index.js";

/** A readable side whose chunks are pushed by the test. */
export class FakeCanvasInput {
	private listener: ((chunk: string) => void) | undefined;

	setEncoding(_encoding: "utf8"): this {
		return this;
	}

	on(_event: "data", listener: (chunk: string) => void): this {
		this.listener = listener;
		return this;
	}

	/** Deliver a chunk as if the host had written it. */
	feed(chunk: string): void {
		this.listener?.(chunk);
	}
}

/** Streams plus the lines the shim wrote, for assertions. */
export interface FakeCanvasStreams extends CanvasShimStreams {
	input: FakeCanvasInput;
	/** Every line written by the shim, parsed. */
	written: unknown[];
}

/** Build injectable streams whose writes are collected instead of sent to stdout. */
export function fakeCanvasStreams(extensionId = "test-extension"): FakeCanvasStreams {
	const written: unknown[] = [];
	return {
		input: new FakeCanvasInput(),
		write: (chunk: string) => {
			for (const line of chunk.split("\n")) {
				if (line.trim().length > 0) written.push(JSON.parse(line));
			}
		},
		extensionId,
		written,
	};
}
