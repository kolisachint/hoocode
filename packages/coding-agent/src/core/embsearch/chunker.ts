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

export const CHUNKER_VERSION = 2;

/**
 * Target lines per chunk.
 *
 * Halving this (30 lines / 500 chars) was measured and rejected. The theory
 * was dilution — a four-line answer sharing one vector with five neighbouring
 * methods — but sharper chunks cost reach: Recall@50 fell 79% -> 69% on
 * `auto +rr` and the boundary class went from 2 of 4 findable to 0 of 4.
 * A fixed top-k over smaller chunks retrieves less *content*: 50 chunks at
 * ~460 chars sees half the corpus that 50 at ~930 does. Recall@1 ticked up
 * (53% -> 55%), which is the sharpening, and it did not pay for the loss.
 */
const CHUNK_LINES = 60;
/** Overlapping lines between consecutive chunks, for context continuity. */
const CHUNK_OVERLAP_LINES = 10;
/**
 * Hard character cap per chunk.
 *
 * The "~256 tokens ≈ 1000 chars" this was set from does not hold for code:
 * measured against the bundled tokenizer over this repo, 1000 chars is **313
 * tokens** at the median, so MiniLM (256) truncates 85.8% of chunks and drops
 * 20.3% of the corpus's tokens, while bge-small (512) drops 0.1%. Raising this
 * is therefore not free in the way the old comment implied — it spends a
 * budget that is already overdrawn on one model and nearly full on the other.
 */
export const CHUNK_MAX_CHARS = 1000;

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
function looksBinary(content: string): boolean {
	const probe = content.slice(0, 8192);
	return probe.includes("\u0000");
}

/**
 * Split `content` into chunks. `relPath` becomes the id prefix. Returns an
 * empty array for empty or binary-looking content.
 *
 * `maxChars` overrides {@link CHUNK_MAX_CHARS} for eval arms that sweep the
 * window. Production never passes it; a caller that does is changing what the
 * index contains and owes the store a distinct key, since nothing about a
 * stored vector records the cap it was built under.
 */
export function chunkFile(relPath: string, content: string, maxChars: number = CHUNK_MAX_CHARS): Chunk[] {
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
			if (chars + lineLen > maxChars && end > start) break;
			chars += lineLen;
			end++;
		}
		// Trim whole blank lines off both ends, and narrow the recorded range to
		// match. Trimming the joined string instead left `startLine`/`endLine`
		// claiming lines whose text was never embedded — 31% of chunks in this
		// repo, up to 3 lines each. Those lines were unfindable by the vector
		// leg while still being handed to the context assembler as if they were
		// part of the chunk.
		let from = start;
		let to = end; // exclusive
		while (from < to && lines[from].trim() === "") from++;
		while (to > from && lines[to - 1].trim() === "") to--;
		let text = lines.slice(from, to).join("\n").trim();
		if (text.length > maxChars) {
			// Oversized chunk (e.g. long minified line): keep the prefix. The
			// underlying model would truncate anyway, so this stays bounded.
			text = text.slice(0, maxChars);
		}
		if (text) {
			chunks.push({
				id: `${relPath}#${index}`,
				text,
				startLine: from + 1,
				endLine: to,
			});
			index++;
		}
		if (end >= lines.length) break;
		// Step forward with overlap, but always make progress.
		start = Math.max(end - CHUNK_OVERLAP_LINES, start + 1);
	}
	return chunks;
}
