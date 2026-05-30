/**
 * ASCII wordmark for the startup banner.
 * Rendered in the terminal with accent color on the "hoo" portion.
 * Source: design-system/assets/wordmark.txt
 */

export const WORDMARK = String.raw`
                   __  __            ______          __
                  / / / /___  ____  / ____/___  ____/ /__
                 / /_/ / __ \/ __ \/ /   / __ \/ __  / _ \
                / __  / /_/ / /_/ / /___/ /_/ / /_/ /  __/
               /_/ /_/\____/\____/\____/\____/\__,_/\___/

               deterministic terminal coding agent   >  hoo
`.trim();

/** Compact one-line logo for tight spaces. */
export const WORDMARK_COMPACT = "hoo — deterministic terminal coding agent";
