/**
 * Shared helpers for the fd-backed file search tools (find, glob).
 *
 * These are pure functions extracted to remove duplication between find.ts and
 * glob.ts. The tools keep their own control flow (find streams a single fd run;
 * glob fans out one fd run per pattern), but path normalization and fd glob
 * argument handling are identical and live here.
 */

import path from "path";

/** Convert a platform path to forward-slash (POSIX) form for stable output. */
export function toPosixPath(value: string): string {
	return value.split(path.sep).join("/");
}

/**
 * Relativize an fd result line against the search root, preserving any trailing
 * slash that marked a directory, and return it in POSIX form.
 */
export function relativizeFdLine(line: string, searchPath: string): string {
	const hadTrailingSlash = line.endsWith("/") || line.endsWith("\\");
	let relativePath: string;
	if (line.startsWith(searchPath)) {
		relativePath = line.slice(searchPath.length + 1);
	} else {
		relativePath = path.relative(searchPath, line);
	}
	if (hadTrailingSlash && !relativePath.endsWith("/")) relativePath += "/";
	return toPosixPath(relativePath);
}

/**
 * Append a glob pattern (and any required --full-path flag) to an fd argument
 * list. fd --glob matches against the basename unless --full-path is set; in
 * --full-path mode a path-containing pattern like 'src/**\/*.ts' needs a leading
 * '**\/' to match anything.
 */
export function applyFdGlobPattern(args: string[], pattern: string): string {
	let effectivePattern = pattern;
	if (pattern.includes("/")) {
		args.push("--full-path");
		if (!pattern.startsWith("/") && !pattern.startsWith("**/") && pattern !== "**") {
			effectivePattern = `**/${pattern}`;
		}
	}
	return effectivePattern;
}
