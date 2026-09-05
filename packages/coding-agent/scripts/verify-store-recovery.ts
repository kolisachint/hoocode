/**
 * Manual end-to-end check for the model-mismatch recovery path.
 *
 * Not a unit test: it needs a real embsearch binary, which CI does not have.
 *
 *   bun scripts/verify-store-recovery.ts /path/to/embsearch
 *
 * A mock build is enough. The service rejects one — but only at the very end of
 * `run()`, after recovery has already happened, so what recovery did is on disk
 * either way. This asserts on the store rather than on the final phase, which
 * also keeps the check honest: "ready" would not prove a rebuild occurred,
 * whereas the store's recorded model can only change if one did.
 *
 * Against a binary with no `store-info` (v0.3.2 and earlier) the expected result
 * is the opposite: the tampered store survives and the failure stands, because
 * nothing can confirm what the daemon choked on.
 */

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { EmbsearchService } from "../src/core/embsearch/embsearch-service.js";
import { getVectorStoreDir } from "../src/core/embsearch/index-meta.js";

const binary = process.argv[2];
if (!binary) throw new Error("usage: verify-store-recovery.ts <embsearch-binary>");

const root = mkdtempSync(path.join(tmpdir(), "embsearch-recovery-"));
const corpus = path.join(root, "corpus");
const storeDir = path.join(root, "store");
mkdirSync(corpus, { recursive: true });
for (let i = 0; i < 12; i++) {
	writeFileSync(
		path.join(corpus, `mod${i}.ts`),
		Array.from({ length: 40 }, (_, l) => `export const sym${i}_${l} = "retrieval fixture line ${l}";`).join("\n"),
	);
}

const manifestPath = path.join(getVectorStoreDir(storeDir), "manifest.json");
const recordedModel = (): string | undefined =>
	existsSync(manifestPath) ? JSON.parse(readFileSync(manifestPath, "utf-8")).model_id : undefined;

const run = async (): Promise<EmbsearchService> => {
	const service = new EmbsearchService({
		cwd: corpus,
		binaryPath: binary,
		storeDir,
		thresholdBytes: 0,
		// Pinned so the only rebuild trigger in play is the model mismatch. Left
		// to follow the binary's version this would want a hybrid store, not find
		// one, and rebuild for that reason instead — passing the check for the
		// wrong reason.
		hybridStore: false,
	});
	await service.start();
	await service.dispose();
	return service;
};

try {
	// Seed a store the way an older binary would have left one. The service
	// cannot seed it here: a mock build is rejected before it indexes anything.
	const seed = path.join(root, "seed.jsonl");
	writeFileSync(
		seed,
		Array.from({ length: 12 }, (_, i) => JSON.stringify({ id: `mod${i}.ts#0`, text: `retrieval fixture ${i}` })).join(
			"\n",
		),
	);
	execFileSync(binary, ["index", "--path", getVectorStoreDir(storeDir), "--input", seed], { stdio: "ignore" });

	const built = recordedModel();
	console.log("1. seeded store        -> store built by:", built);
	if (!built) throw new Error("no store was built; cannot test recovery");

	// Re-stamp the store's recorded model, exactly as shipping a new model does.
	const foreign = "bge-small-en-v1.5-int8.deadbeef";
	const manifest = JSON.parse(readFileSync(manifestPath, "utf-8"));
	manifest.model_id = foreign;
	writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
	console.log(`2. store re-stamped    -> ${built} => ${foreign}`);

	await run();
	const after = recordedModel();
	console.log("3. after model change  -> store built by:", after);

	// Against a mock build the rebuild cannot finish — the service rejects the
	// mock embedder before it indexes — so the store is gone rather than
	// re-stamped. Either outcome proves the point: the foreign store was
	// discarded. Only its survival is a failure.
	if (after === foreign) {
		throw new Error("FAIL: the foreign store survived — recovery did not fire (binary lacks store-info?)");
	}
	console.log(
		after === undefined
			? "\nPASS: the foreign store was discarded (rebuild then stopped on the mock embedder)."
			: "\nPASS: the foreign store was discarded and rebuilt by this binary's own model.",
	);
} finally {
	rmSync(root, { recursive: true, force: true });
}
