/**
 * Emit the eval corpus as JSONL for embsearch indexing benchmarks.
 *
 * Uses the same scanner and chunker the indexer does, so the record set is the
 * one a real run would embed — measuring anything else measures the wrong
 * thing. Output is `{"id","text"}` per line, the shape `embsearch index` and
 * the daemon's `bulk` op both take.
 *
 * Usage:
 *   bun scripts/bench-corpus.ts [--cwd <repo>] [--out chunks.jsonl] [--limit N]
 */

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { chunkFile } from "../src/core/embsearch/chunker.js";
import { scanRepo } from "../src/core/embsearch/repo-scan.js";

function flag(name: string): string | undefined {
	const i = process.argv.indexOf(`--${name}`);
	return i >= 0 ? process.argv[i + 1] : undefined;
}

const cwd = path.resolve(flag("cwd") ?? process.cwd());
const out = path.resolve(flag("out") ?? "chunks.jsonl");
const limit = flag("limit") ? Number(flag("limit")) : Infinity;

const scan = scanRepo(cwd);
const lines: string[] = [];
let files = 0;

for (const file of scan.files) {
	if (lines.length >= limit) break;
	let content: string;
	try {
		content = readFileSync(path.join(cwd, file.rel), "utf8");
	} catch {
		continue;
	}
	const chunks = chunkFile(file.rel, content);
	if (chunks.length === 0) continue;
	files++;
	for (const chunk of chunks) {
		if (lines.length >= limit) break;
		lines.push(JSON.stringify({ id: chunk.id, text: chunk.text }));
	}
}

writeFileSync(out, `${lines.join("\n")}\n`);

// Character length is the honest proxy for embedding cost here: the tokenizer
// pads each batch to its longest member, so the spread matters as much as the
// mean.
const lengths = lines.map((l) => (JSON.parse(l) as { text: string }).text.length).sort((a, b) => a - b);
const sum = lengths.reduce((a, b) => a + b, 0);
const pct = (p: number) => lengths[Math.min(lengths.length - 1, Math.floor((p / 100) * lengths.length))];

console.error(`corpus: ${lines.length} chunks from ${files} files -> ${out}`);
console.error(
	`chars/chunk: mean ${Math.round(sum / lengths.length)}  p50 ${pct(50)}  p90 ${pct(90)}  max ${lengths[lengths.length - 1]}`,
);
