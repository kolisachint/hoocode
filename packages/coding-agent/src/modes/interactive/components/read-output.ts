/**
 * Render read tool output with line numbers, matching the edit tool's visual style.
 *
 * The diff tool shows line numbers as: ` ${lineNum} content`
 * This component applies the same pattern to plain file reads.
 */

import { theme } from "../theme/theme.js";

export interface ReadOutputOptions {
	/** Starting line number (1-indexed). Default: 1 */
	startLine?: number;
	/** Whether to apply syntax highlighting. Default: false */
	highlight?: boolean;
}

/**
 * Format a line number with consistent width alignment.
 * Matches the diff tool's visual style: right-aligned, dim color.
 */
function formatLineNumber(num: number, maxDigits: number): string {
	return num.toString().padStart(maxDigits);
}

/**
 * Render file content with line numbers.
 *
 * @param content - The file content string
 * @param options - Rendering options
 * @returns Array of styled lines
 */
export function renderReadOutput(content: string, options: ReadOutputOptions = {}): string[] {
	const { startLine = 1 } = options;
	const lines = content.split("\n");

	// Calculate the maximum line number to determine padding width
	const endLine = startLine + lines.length - 1;
	const maxDigits = Math.max(2, endLine.toString().length);

	// Render each line with line numbers matching the diff tool's style
	return lines.map((line, index) => {
		const lineNum = startLine + index;
		const paddedNum = formatLineNumber(lineNum, maxDigits);
		// Style matches diff.ts: dim color for line numbers, followed by content
		return `${theme.fg("dim", paddedNum)} ${theme.fg("toolOutput", line)}`;
	});
}

/**
 * Render file content with line numbers and truncation notice.
 *
 * @param content - The file content string
 * @param totalLines - Total lines in the file (for truncation notice)
 * @param options - Rendering options
 * @returns Formatted string with line numbers
 */
export function renderReadOutputWithTruncation(
	content: string,
	totalLines: number,
	options: ReadOutputOptions = {},
): string {
	const { startLine = 1 } = options;
	const lines = content.split("\n");
	const renderedLines = renderReadOutput(content, options);

	// Add truncation notice if needed
	if (startLine + lines.length - 1 < totalLines) {
		const endLine = startLine + lines.length - 1;
		renderedLines.push("");
		renderedLines.push(
			theme.fg(
				"muted",
				`[Showing lines ${startLine}-${endLine} of ${totalLines}. Use offset=${endLine + 1} to continue.]`,
			),
		);
	}

	return renderedLines.join("\n");
}
