/**
 * Repository scan for semantic indexing: enumerates indexable source files
 * using the same ignore-aware walk the native find/grep fallbacks use, and
 * totals their bytes so the caller can apply the size threshold.
 */

import { statSync } from "fs";
import path from "path";
import { collectEntries } from "../tools/native-search.js";

/** Files larger than this are never indexed (vendored bundles, lockfiles, data dumps). */
const MAX_FILE_BYTES = 1024 * 1024;

/** Files that are part of the ignore mechanism, not source. */
const SKIP_NAMES = new Set([".gitignore", ".gitattributes", ".gitkeep"]);

/** Extensions that are never worth embedding. */
const SKIP_EXTENSIONS = new Set([
	".png",
	".jpg",
	".jpeg",
	".gif",
	".webp",
	".ico",
	".bmp",
	".svg",
	".pdf",
	".zip",
	".gz",
	".tar",
	".bz2",
	".xz",
	".7z",
	".jar",
	".war",
	".class",
	".exe",
	".dll",
	".so",
	".dylib",
	".bin",
	".dat",
	".onnx",
	".pt",
	".safetensors",
	".woff",
	".woff2",
	".ttf",
	".otf",
	".eot",
	".mp3",
	".mp4",
	".wav",
	".avi",
	".mov",
	".webm",
	".lock",
	".min.js",
	".min.css",
	".map",
]);

export interface RepoScanFile {
	/** Absolute path. */
	abs: string;
	/** POSIX path relative to the scan root. */
	rel: string;
	/** File size in bytes. */
	size: number;
	/** mtime in ms. */
	mtimeMs: number;
}

export interface RepoScanResult {
	files: RepoScanFile[];
	totalBytes: number;
}

function hasSkippedExtension(rel: string): boolean {
	const lower = rel.toLowerCase();
	for (const ext of SKIP_EXTENSIONS) {
		if (lower.endsWith(ext)) return true;
	}
	return false;
}

/**
 * Enumerate indexable files under `root`: respects hierarchical `.gitignore`,
 * always skips `.git`/`node_modules`, drops binary-ish extensions and files
 * over 1MB. Returns files plus their byte total for threshold checks.
 */
export function scanRepo(root: string, signal?: AbortSignal): RepoScanResult {
	const entries = collectEntries(root, {
		signal,
		alwaysSkipDirs: new Set([".git", "node_modules"]),
	});
	const files: RepoScanFile[] = [];
	let totalBytes = 0;
	for (const entry of entries) {
		if (entry.type !== "f") continue;
		if (SKIP_NAMES.has(path.basename(entry.rel))) continue;
		if (hasSkippedExtension(entry.rel)) continue;
		let stat: ReturnType<typeof statSync>;
		try {
			stat = statSync(entry.abs);
		} catch {
			continue;
		}
		if (stat.size === 0 || stat.size > MAX_FILE_BYTES) continue;
		files.push({ abs: entry.abs, rel: entry.rel, size: stat.size, mtimeMs: stat.mtimeMs });
		totalBytes += stat.size;
	}
	return { files, totalBytes };
}
