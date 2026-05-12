/**
 * Check if a string is a file URL (starts with "file:///").
 */
export declare function isFileUrl(filePath: string): boolean;
/**
 * Convert a file URL to a file path.
 * Handles both Unix-style (file:///path) and Windows-style (file:///C:/path) file URLs.
 * Returns the original path if it's not a file URL.
 */
export declare function normalizeFileUrl(filePath: string): string;
export declare function expandPath(filePath: string): string;
/**
 * Resolve a path relative to the given cwd.
 * Handles ~ expansion and absolute paths.
 */
export declare function resolveToCwd(filePath: string, cwd: string): string;
export declare function resolveReadPath(filePath: string, cwd: string): string;
//# sourceMappingURL=path-utils.d.ts.map