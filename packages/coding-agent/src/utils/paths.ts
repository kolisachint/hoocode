import { existsSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve as resolvePath, sep } from "node:path";

/**
 * Resolve a path to its canonical (real) form, following symlinks.
 * Falls back to the raw path if resolution fails (e.g. the target does
 * not exist yet), so that callers never crash on missing filesystem
 * entries.
 */
export function canonicalizePath(path: string): string {
	try {
		return realpathSync(path);
	} catch {
		return path;
	}
}

/**
 * Returns true if the value is NOT a package source (npm:, git:, etc.)
 * or a URL protocol. Bare names and relative paths without ./ prefix
 * are considered local.
 */
export function isLocalPath(value: string): boolean {
	const trimmed = value.trim();
	// Known non-local prefixes
	if (
		trimmed.startsWith("npm:") ||
		trimmed.startsWith("git:") ||
		trimmed.startsWith("github:") ||
		trimmed.startsWith("http:") ||
		trimmed.startsWith("https:") ||
		trimmed.startsWith("ssh:")
	) {
		return false;
	}
	return true;
}

function resolveAgainstCwd(filePath: string, cwd: string): string {
	return isAbsolute(filePath) ? resolvePath(filePath) : resolvePath(cwd, filePath);
}

export function getCwdRelativePath(filePath: string, cwd: string): string | undefined {
	const resolvedCwd = resolvePath(cwd);
	const resolvedPath = resolveAgainstCwd(filePath, resolvedCwd);
	const relativePath = relative(resolvedCwd, resolvedPath);
	const isInsideCwd =
		relativePath === "" ||
		(relativePath !== ".." && !relativePath.startsWith(`..${sep}`) && !isAbsolute(relativePath));

	return isInsideCwd ? relativePath || "." : undefined;
}

export function formatPathRelativeToCwdOrAbsolute(filePath: string, cwd: string): string {
	const absolutePath = resolveAgainstCwd(filePath, cwd);
	return (getCwdRelativePath(absolutePath, cwd) ?? absolutePath).split(sep).join("/");
}

/** Find the nearest ancestor directory containing a `.git` entry, or null. */
export function findGitRepoRoot(startDir: string): string | null {
	let dir = resolvePath(startDir);
	while (true) {
		if (existsSync(join(dir, ".git"))) return dir;
		const parent = dirname(dir);
		if (parent === dir) return null;
		dir = parent;
	}
}

/**
 * Collect `.agents/<subdir>/` directories walking from startDir up to the git
 * root, cwd-first (so a closer dir overrides an ancestor under first-match-wins).
 * When startDir is not inside a git repo, the walk continues to the filesystem
 * root, mirroring the agent/skill ancestor scanners.
 */
export function collectAgentsAncestorDirs(startDir: string, subdir: string): string[] {
	const dirs: string[] = [];
	const resolvedStart = resolvePath(startDir);
	const gitRoot = findGitRepoRoot(resolvedStart);
	let dir = resolvedStart;
	while (true) {
		dirs.push(join(dir, ".agents", subdir));
		if (gitRoot && dir === gitRoot) break;
		const parent = dirname(dir);
		if (parent === dir) break;
		dir = parent;
	}
	return dirs;
}
