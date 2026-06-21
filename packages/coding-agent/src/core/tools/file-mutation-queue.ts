import { realpathSync } from "node:fs";
import { resolve } from "node:path";

const fileMutationQueues = new Map<string, Promise<void>>();

/**
 * Cache resolved realpaths keyed by the absolute input path. realpathSync.native
 * is a blocking syscall; the canonical path for a given absolute path is stable
 * for the process lifetime, so caching keeps the hot edit/write path off the
 * filesystem after the first lookup.
 */
const realpathCache = new Map<string, string>();

function getMutationQueueKey(filePath: string): string {
	const resolvedPath = resolve(filePath);
	const cached = realpathCache.get(resolvedPath);
	if (cached !== undefined) return cached;
	let key: string;
	try {
		key = realpathSync.native(resolvedPath);
	} catch {
		key = resolvedPath;
	}
	realpathCache.set(resolvedPath, key);
	return key;
}

/**
 * Serialize file mutation operations targeting the same file.
 * Operations for different files still run in parallel.
 */
export async function withFileMutationQueue<T>(filePath: string, fn: () => Promise<T>): Promise<T> {
	const key = getMutationQueueKey(filePath);
	const currentQueue = fileMutationQueues.get(key) ?? Promise.resolve();

	let releaseNext!: () => void;
	const nextQueue = new Promise<void>((resolveQueue) => {
		releaseNext = resolveQueue;
	});
	const chainedQueue = currentQueue.then(() => nextQueue);
	fileMutationQueues.set(key, chainedQueue);

	await currentQueue;
	try {
		return await fn();
	} finally {
		releaseNext();
		if (fileMutationQueues.get(key) === chainedQueue) {
			fileMutationQueues.delete(key);
		}
	}
}
