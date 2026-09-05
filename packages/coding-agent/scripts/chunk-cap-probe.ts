/**
 * Token cost of a chunk-cap change, measured rather than assumed.
 *
 * A3/A4 raise `CHUNK_MAX_CHARS` to unlock bge-small's 512-token window. Whether
 * that window is actually unlocked or immediately overrun is a question about
 * tokens, and the cap is in characters — so it has to be measured against the
 * real tokenizer on the real corpus before the arms are worth running.
 */
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { chunkFile } from "../src/core/embsearch/chunker.js";
import { scanRepo } from "../src/core/embsearch/repo-scan.js";

const cwd = path.resolve(process.argv[2] ?? process.cwd());
for (const cap of [1000, 1500, 2000]) {
	const texts: string[] = [];
	for (const f of scanRepo(cwd).files) {
		if (f.rel === ".git") continue;
		let c: string;
		try {
			c = readFileSync(path.join(cwd, f.rel), "utf8");
		} catch {
			continue;
		}
		for (const ch of chunkFile(f.rel, c, cap)) texts.push(ch.text);
	}
	const L = [...texts].map((t) => t.length).sort((a, b) => a - b);
	console.log(
		JSON.stringify({
			cap,
			chunks: texts.length,
			meanChars: Math.round(L.reduce((a, b) => a + b, 0) / L.length),
			p50: L[Math.floor(L.length / 2)],
			p90: L[Math.floor(L.length * 0.9)],
		}),
	);
	writeFileSync(
		`/tmp/chunks-cap${cap}.jsonl`,
		`${texts.map((t, i) => JSON.stringify({ id: `c${i}`, text: t })).join("\n")}\n`,
	);
}
