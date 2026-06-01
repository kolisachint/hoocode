/**
 * ASCII wordmark for the startup banner.
 * Rendered in the terminal with accent color on the "hoo" portion.
 * Source: design-system/assets/wordmark.txt
 */

import { WORDMARK_SYMBOL_BLOCKS } from "./wordmark-symbol.generated.js";

export const WORDMARK = [
	"                   __  __            ______          __",
	"                  / / / /___  ____  / ____/___  ____/ /__",
	"                 / /_/ / __ \\/ __ \\/ /   / __ \\/ __  / _ \\",
	"                / __  / /_/ / /_/ / /___/ /_/ / /_/ /  __/",
	"               /_/ /_/\\____/\\____/\\____/\\____/\\__,_/\\___/",
	"",
	"               deterministic terminal coding agent   >  hoo",
].join("\n");

/** Compact one-line logo for tight spaces. */
export const WORDMARK_COMPACT = "hoo — deterministic terminal coding agent";

/**
 * Colored half-block owl symbol, indented to sit centered above the ASCII
 * wordmark. Carries baked-in brand colors (Signal Cyan + white), so it only
 * renders on truecolor terminals; callers fall back to {@link WORDMARK}
 * otherwise.
 */
const SYMBOL_INDENT = " ".repeat(15);
export const WORDMARK_SYMBOL = WORDMARK_SYMBOL_BLOCKS.split("\n")
	.map((line) => SYMBOL_INDENT + line)
	.join("\n");
