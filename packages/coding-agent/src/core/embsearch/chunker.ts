/**
 * File chunking for semantic indexing.
 *
 * Splits a file into overlapping line windows, each capped by characters so a
 * chunk stays within the embedding model's effective token window (MiniLM
 * truncates around 256 tokens ≈ 1000 chars). Chunk ids are `relpath#index`;
 * the id → line-range mapping is kept in the sidecar metadata (index-meta.ts)
 * so search hits can be rendered as `path:start-end`.
 *
 * Bump CHUNKER_VERSION when the strategy changes — a version mismatch in the
 * sidecar triggers a clean rebuild of the store.
 */

export const CHUNKER_VERSION = 1;

/** Target lines per chunk. */
const CHUNK_LINES = 60;
/** Overlapping lines between consecutive chunks, for context continuity. */
const CHUNK_OVERLAP_LINES = 10;
/** Hard character cap per chunk (MiniLM truncates ~256 tokens ≈ 1000 chars). */
const CHUNK_MAX_CHARS = 1000;

export interface Chunk {
	/** `relpath#index` — the id stored in the vector index. */
	id: string;
	/** Text sent to the embedder. */
	text: string;
	/** 1-based inclusive start line. */
	startLine: number;
	/** 1-based inclusive end line. */
	endLine: number;
}

/** Heuristic binary sniff: NUL byte in the first 8KB. */
export function looksBinary(content: string): boolean {
	const probe = content.slice(0, 8192);
	return probe.includes("\u0000");
}

/**
 * Split `content` into chunks. `relPath` becomes the id prefix. Returns an
 * empty array for empty or binary-looking content.
 */
export function chunkFile(relPath: string, content: string): Chunk[] {
	if (!content.trim() || looksBinary(content)) return [];
	const lines = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
	const chunks: Chunk[] = [];
	let start = 0; // 0-based
	let index = 0;

	while (start < lines.length) {
		let end = start; // exclusive
		let chars = 0;
		while (end < lines.length && end - start < CHUNK_LINES) {
			const lineLen = lines[end].length + 1;
			if (chars + lineLen > CHUNK_MAX_CHARS && end > start) break;
			chars += lineLen;
			end++;
		}
		let text = lines.slice(start, end).join("\n").trim();
		if (text.length > CHUNK_MAX_CHARS) {
			// Oversized chunk (e.g. long minified line): keep the prefix. The
			// underlying model would truncate anyway, so this stays bounded.
			text = text.slice(0, CHUNK_MAX_CHARS);
		}
		if (text) {
			chunks.push({
				id: `${relPath}#${index}`,
				text,
				startLine: start + 1,
				endLine: end,
			});
			index++;
		}
		if (end >= lines.length) break;
		// Step forward with overlap, but always make progress.
		start = Math.max(end - CHUNK_OVERLAP_LINES, start + 1);
	}
	return chunks;
}
