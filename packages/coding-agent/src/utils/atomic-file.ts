import { mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

/**
 * Write a file atomically: write to a unique temp file in the same directory,
 * then rename over the target. The same-directory temp keeps the rename on one
 * filesystem (rename across mounts is a non-atomic copy), so a reader can never
 * observe a torn, half-written file — it sees either the old content or the
 * new. Used for handshake files like a subagent's result.json, where a reader
 * (the parent pool) may race a writer that gets SIGKILLed mid-write.
 *
 * Throws on failure like writeFileSync; callers that treat persistence as
 * best-effort keep their own try/catch.
 */
export function writeFileAtomicSync(path: string, data: string): void {
	mkdirSync(dirname(path), { recursive: true });
	const tempPath = join(dirname(path), `.${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}.tmp`);
	try {
		writeFileSync(tempPath, data);
		renameSync(tempPath, path);
	} catch (error) {
		rmSync(tempPath, { force: true });
		throw error;
	}
}
