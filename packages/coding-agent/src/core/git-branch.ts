import { spawnSync } from "child_process";
import { existsSync, readFileSync, statSync } from "fs";
import { dirname, join, resolve } from "path";

/**
 * Locating a repo's git metadata, and reading the branch out of it.
 *
 * Shared because two callers want it for different reasons: the footer shows the
 * live branch and watches it for changes, while a session records the branch it
 * started on into its header. Keeping one implementation means the branch a
 * session remembers is the same string the footer was showing at the time.
 */

export type GitPaths = {
	repoDir: string;
	commonGitDir: string;
	headPath: string;
};

/**
 * Find git metadata paths by walking up from cwd.
 * Handles both regular git repos (.git is a directory) and worktrees (.git is a file).
 */
export function findGitPaths(cwd: string): GitPaths | null {
	let dir = cwd;
	while (true) {
		const gitPath = join(dir, ".git");
		if (existsSync(gitPath)) {
			try {
				const stat = statSync(gitPath);
				if (stat.isFile()) {
					const content = readFileSync(gitPath, "utf8").trim();
					if (content.startsWith("gitdir: ")) {
						const gitDir = resolve(dir, content.slice(8).trim());
						const headPath = join(gitDir, "HEAD");
						if (!existsSync(headPath)) return null;
						const commonDirPath = join(gitDir, "commondir");
						const commonGitDir = existsSync(commonDirPath)
							? resolve(gitDir, readFileSync(commonDirPath, "utf8").trim())
							: gitDir;
						return { repoDir: dir, commonGitDir, headPath };
					}
				} else if (stat.isDirectory()) {
					const headPath = join(gitPath, "HEAD");
					if (!existsSync(headPath)) return null;
					return { repoDir: dir, commonGitDir: gitPath, headPath };
				}
			} catch {
				return null;
			}
		}
		const parent = dirname(dir);
		if (parent === dir) return null;
		dir = parent;
	}
}

/** Ask git for the current branch. Returns null on detached HEAD or if git is unavailable. */
export function resolveBranchWithGitSync(repoDir: string): string | null {
	const result = spawnSync("git", ["--no-optional-locks", "symbolic-ref", "--quiet", "--short", "HEAD"], {
		cwd: repoDir,
		encoding: "utf8",
		stdio: ["ignore", "pipe", "ignore"],
	});
	const branch = result.status === 0 ? result.stdout.trim() : "";
	return branch || null;
}

/**
 * Read the branch from an already-located HEAD. Returns "detached" when HEAD is
 * not on a branch, null when it cannot be read at all. Reftable repos write a
 * `.invalid` placeholder into HEAD, which is the one case that has to shell out.
 */
export function readBranchFromHead(paths: GitPaths): string | null {
	try {
		const content = readFileSync(paths.headPath, "utf8").trim();
		if (content.startsWith("ref: refs/heads/")) {
			const branch = content.slice(16);
			return branch === ".invalid" ? (resolveBranchWithGitSync(paths.repoDir) ?? "detached") : branch;
		}
		return "detached";
	} catch {
		return null;
	}
}

/**
 * The branch `cwd` is on, or undefined when there is no branch worth recording —
 * outside a repo, or on a detached HEAD, where the answer names no work.
 *
 * Cheap enough to call on the session-creation path: a walk up to `.git` and one
 * file read, with no subprocess outside the reftable case.
 */
export function readGitBranch(cwd: string): string | undefined {
	const paths = findGitPaths(cwd);
	if (!paths) return undefined;
	const branch = readBranchFromHead(paths);
	return branch && branch !== "detached" ? branch : undefined;
}
