/**
 * Lossless output compression utilities for tool outputs.
 *
 * All compression is strictly lossless - no useful information is removed.
 * Compression is command-aware where possible, and general-purpose otherwise.
 */

/** Minimum output size (in bytes) to apply compression. Below this, overhead exceeds savings. */
const MIN_COMPRESSION_SIZE = 1024; // 1KB

// ============================================================================
// General Compression (applied to all outputs)
// ============================================================================

/**
 * Collapse 3+ consecutive blank lines to 2 (preserves paragraph separation).
 */
export function collapseBlankLines(text: string): string {
	return text.replace(/\n{3,}/g, "\n\n");
}

/**
 * Strip trailing whitespace from each line.
 */
export function stripTrailingWhitespace(text: string): string {
	return text
		.split("\n")
		.map((line) => line.trimEnd())
		.join("\n");
}

/**
 * Remove duplicate consecutive lines (keeps first occurrence).
 */
export function removeDuplicateLines(text: string): string {
	const lines = text.split("\n");
	const result: string[] = [];
	let prevLine: string | undefined;

	for (const line of lines) {
		if (line !== prevLine) {
			result.push(line);
			prevLine = line;
		}
	}

	return result.join("\n");
}

/**
 * Apply general lossless compression to any output.
 * Single-pass implementation for efficiency.
 */
export function compressGeneral(text: string): string {
	const lines = text.split("\n");
	const result: string[] = [];
	let blankCount = 0;

	for (const line of lines) {
		const trimmed = line.trimEnd();

		if (trimmed === "") {
			blankCount++;
			// Allow at most 2 consecutive blank lines
			if (blankCount <= 2) {
				result.push(trimmed);
			}
		} else {
			blankCount = 0;
			result.push(trimmed);
		}
	}

	return result.join("\n");
}

// ============================================================================
// Command-Specific Compression (bash only)
// ============================================================================

/**
 * Detect the primary command from a bash command string.
 */
function detectCommand(command: string): string {
	const trimmed = command.trim();
	// Strip leading env vars (FOO=bar command)
	const withoutEnv = trimmed.replace(/^[A-Z_]+=\S+\s+/, "");
	// Strip leading sudo
	const withoutSudo = withoutEnv.replace(/^sudo\s+/, "");
	// Get first word (the actual command)
	const firstWord = withoutSudo.split(/\s+/)[0] ?? "";
	// Strip path prefix if any
	return firstWord.split("/").pop() ?? "";
}

/**
 * Compress npm/yarn/pnpm install output.
 * - Remove download progress lines
 * - Collapse "added N packages" summaries
 */
function compressNpmInstall(output: string): string {
	const lines = output.split("\n");
	const result: string[] = [];

	for (const line of lines) {
		// Skip download progress (e.g., "fetchMetadata: ...", "reify: ...")
		if (/^(fetchMetadata|reify|audit|idealTree|sill|warn)\b/.test(line)) {
			continue;
		}
		// Skip verbose fetch/cache lines
		if (/^\s*(http|https|fetch|cache|tarball|extract)\b/.test(line)) {
			continue;
		}
		// Keep everything else (errors, warnings, summaries)
		result.push(line);
	}

	return result.join("\n");
}

/**
 * Compress git diff output.
 * - Strip file headers (diff --git, index, ---, +++)
 * - Collapse unchanged context lines
 */
function compressGitDiff(output: string): string {
	const lines = output.split("\n");
	const result: string[] = [];
	let contextCount = 0;

	for (const line of lines) {
		// Skip git diff metadata
		if (/^(diff --git|index [0-9a-f]+|--- a\/|\+\+\+ b\/)/.test(line)) {
			continue;
		}

		// Count context lines (lines starting with space)
		if (line.startsWith(" ") && !line.startsWith("  ")) {
			contextCount++;
			// Only show first 2 context lines per hunk, then skip
			if (contextCount > 2) {
				continue;
			}
		} else {
			contextCount = 0;
		}

		result.push(line);
	}

	return result.join("\n");
}

/**
 * Compress cargo/rustc test output.
 * - Collapse passing tests, keep failures
 */
function compressCargoTest(output: string): string {
	const lines = output.split("\n");
	const result: string[] = [];
	let passingCount = 0;

	for (const line of lines) {
		// Count passing test lines (e.g., "test foo ... ok")
		if (/^test\s+.+\s+\.\.\.\s+ok\s*$/.test(line)) {
			passingCount++;
			continue;
		}
		// Count ignored tests
		if (/^test\s+.+\s+\.\.\.\s+ignored\s*$/.test(line)) {
			continue;
		}
		// Show test summary line
		if (/^test result:/.test(line)) {
			result.push(line);
			continue;
		}
		// Keep failures, errors, and everything else
		if (passingCount > 0 && line === "") {
			// Add summary before blank line
			result.push(`  (${passingCount} passing tests omitted)`);
			passingCount = 0;
		}
		result.push(line);
	}

	// Handle case where output ends with passing tests
	if (passingCount > 0) {
		result.push(`  (${passingCount} passing tests omitted)`);
	}

	return result.join("\n");
}

/**
 * Compress docker build output.
 * - Strip layer download progress
 * - Keep build steps and errors
 */
function compressDockerBuild(output: string): string {
	const lines = output.split("\n");
	const result: string[] = [];

	for (const line of lines) {
		// Skip download progress (e.g., "Downloading layer...")
		if (/^(Downloading|Pulling|Extracting|Waiting|Verifying)/.test(line)) {
			continue;
		}
		// Skip progress bars
		if (/[\u2588\u2591\u2592]{10,}/.test(line)) {
			continue;
		}
		// Keep build steps (FROM, RUN, COPY, etc.) and errors
		result.push(line);
	}

	return result.join("\n");
}

/**
 * Compress jest/mocha test output.
 * - Collapse passing tests
 * - Keep failures
 */
function compressJsTest(output: string): string {
	const lines = output.split("\n");
	const result: string[] = [];
	let passingCount = 0;

	for (const line of lines) {
		// Skip passing test indicators (✓, ✔, ○)
		if (/^\s*[✓✔○●]\s+/.test(line)) {
			passingCount++;
			continue;
		}
		// Skip passing test lines (e.g., "  PASS  src/foo.test.ts")
		if (/^\s*PASS\s+/.test(line)) {
			passingCount++;
			continue;
		}
		// Show test summary
		if (/^(Tests|Test Suites):/.test(line)) {
			result.push(line);
			continue;
		}
		// Keep failures (×, ✗, FAIL) and everything else
		if (passingCount > 0 && (line.includes("FAIL") || line.includes("×") || line.includes("✗"))) {
			result.push(`  (${passingCount} passing tests omitted)`);
			passingCount = 0;
		}
		result.push(line);
	}

	if (passingCount > 0) {
		result.push(`  (${passingCount} passing tests omitted)`);
	}

	return result.join("\n");
}

/**
 * Compress go test output.
 * - Collapse passing tests
 * - Keep failures
 */
function compressGoTest(output: string): string {
	const lines = output.split("\n");
	const result: string[] = [];
	let passingCount = 0;

	for (const line of lines) {
		// Skip passing test lines (e.g., "--- PASS: TestFoo (0.00s)")
		if (/^---\s+PASS:/.test(line)) {
			passingCount++;
			continue;
		}
		// Skip PASS lines
		if (/^PASS$/.test(line)) {
			continue;
		}
		// Show test summary
		if (/^(ok|FAIL)\s+/.test(line)) {
			result.push(line);
			continue;
		}
		// Keep failures and everything else
		if (passingCount > 0 && line.includes("FAIL")) {
			result.push(`  (${passingCount} passing tests omitted)`);
			passingCount = 0;
		}
		result.push(line);
	}

	if (passingCount > 0) {
		result.push(`  (${passingCount} passing tests omitted)`);
	}

	return result.join("\n");
}

/**
 * Apply command-specific compression based on the detected command.
 */
function compressCommandSpecific(command: string, output: string): string {
	const cmd = detectCommand(command);

	switch (cmd) {
		case "npm":
		case "yarn":
		case "pnpm":
			// Check if it's an install/add command
			if (/\b(install|add|i)\b/.test(command)) {
				return compressNpmInstall(output);
			}
			break;
		case "git":
			// Check if it's a diff command
			if (/\bdiff\b/.test(command)) {
				return compressGitDiff(output);
			}
			break;
		case "cargo":
			// Check if it's a test command
			if (/\btest\b/.test(command)) {
				return compressCargoTest(output);
			}
			break;
		case "docker":
			// Check if it's a build command
			if (/\bbuild\b/.test(command)) {
				return compressDockerBuild(output);
			}
			break;
		case "jest":
		case "mocha":
		case "vitest":
		case "pytest":
			return compressJsTest(output);
		case "go":
			// Check if it's a test command
			if (/\btest\b/.test(command)) {
				return compressGoTest(output);
			}
			break;
	}

	return output;
}

/**
 * Strip common noise patterns from any bash output.
 * This is applied to all commands regardless of detection.
 */
function stripNoisePatterns(text: string): string {
	return (
		text
			// Strip npm warnings (keep errors)
			.replace(/^npm (warn|notice) .+$/gm, "")
			// Strip yarn warnings
			.replace(/^warning .+$/gm, "")
			// Strip pnpm warnings
			.replace(/^pnpm (warn|notice) .+$/gm, "")
			// Strip common shell warnings
			.replace(/^bash: .+ warning: .+$/gm, "")
			// Strip "The command completed with non-zero exit status" noise
			.replace(/^The command exited with exit code .+$/gm, "")
	);
}

/**
 * Apply all compression to a bash command output.
 * This is the main entry point for bash output compression.
 *
 * @param command - The bash command that was executed
 * @param output - The raw output from the command
 * @returns Compressed output (lossless)
 */
export function compressBashOutput(command: string, output: string): string {
	// Skip compression for small outputs (overhead > savings)
	if (Buffer.byteLength(output, "utf-8") < MIN_COMPRESSION_SIZE) {
		return output;
	}

	let result = output;

	// Phase 1: Strip noise patterns
	result = stripNoisePatterns(result);

	// Phase 2: Command-specific compression
	result = compressCommandSpecific(command, result);

	// Phase 3: General compression
	result = compressGeneral(result);

	return result;
}

/**
 * Apply compression to grep output.
 * This strips redundant context and normalizes output.
 */
export function compressGrepOutput(output: string): string {
	if (Buffer.byteLength(output, "utf-8") < MIN_COMPRESSION_SIZE) {
		return output;
	}
	return compressGeneral(output);
}

/**
 * Apply compression to read output.
 * This normalizes whitespace but preserves content.
 */
export function compressReadOutput(output: string): string {
	// For read, we only do light compression - no line collapsing
	if (Buffer.byteLength(output, "utf-8") < MIN_COMPRESSION_SIZE) {
		return output;
	}
	return stripTrailingWhitespace(output);
}

/**
 * Apply compression to find output.
 * This normalizes paths and removes duplicates.
 */
export function compressFindOutput(output: string): string {
	if (Buffer.byteLength(output, "utf-8") < MIN_COMPRESSION_SIZE) {
		return output;
	}
	return compressGeneral(output);
}

/**
 * Apply compression to ls output.
 * This normalizes formatting.
 */
export function compressLsOutput(output: string): string {
	if (Buffer.byteLength(output, "utf-8") < MIN_COMPRESSION_SIZE) {
		return output;
	}
	return compressGeneral(output);
}
