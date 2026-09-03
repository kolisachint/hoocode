/**
 * Path normalization shared across the codebase. `toPosixPath` gives stable
 * forward-slash output wherever a platform path is rendered or compared
 * (native-search, skills, read, package resource discovery).
 */

import path from "path";

/** Convert a platform path to forward-slash (POSIX) form for stable output. */
export function toPosixPath(value: string): string {
	return value.split(path.sep).join("/");
}
