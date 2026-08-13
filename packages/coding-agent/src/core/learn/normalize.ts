/**
 * Normalization and noise filters for the `/learn` extractor.
 *
 * Everything downstream — clustering, dedupe, cross-session counting — is only
 * as good as what happens here. Two runs of the same failing command differ by
 * absolute paths, line numbers, PIDs and timings; two statements of the same
 * rule differ by politeness and word order. Collapsing that variance is what
 * turns a pile of transcripts into counts, and the counts are the product.
 *
 * These are deliberately lexical and cheap. Semantic grouping is left to the
 * model, which sees the deduped candidates and can tell that "run tests with
 * coverage" and "use --coverage on the suite" are one rule. Trying to do that
 * here would cost a dependency and still be worse.
 */

/** Longest text we normalize; error dumps can run to megabytes. */
const MAX_SIGNATURE_CHARS = 400;

/**
 * Volatile fragments that make two runs of the same failure look different.
 *
 * Order is load-bearing, because these patterns overlap and the first one to
 * match wins the characters. Longest-and-most-specific therefore runs first:
 * paths before line numbers so `foo.ts:12:3` does not leave a bare `:12:3`, and
 * timestamps before line numbers so the `:00:00` inside `10:00:00` is not
 * mistaken for a line/column pair. Adding a pattern means placing it by
 * specificity, not appending it.
 */
const VOLATILE_PATTERNS: Array<[RegExp, string]> = [
	// Absolute POSIX and Windows paths, including the temp dirs that change per run.
	[/\/(?:[\w.@+-]+\/)+[\w.@+-]+/g, "<path>"],
	[/[A-Za-z]:\\(?:[\w.@+-]+\\)+[\w.@+-]+/g, "<path>"],
	// UUIDs, before the shorter hex rule can eat their first group.
	[/\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi, "<uuid>"],
	// ISO timestamps and clock times, before the line/column rule.
	[/\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?Z?/g, "<time>"],
	[/\b\d{1,2}:\d{2}:\d{2}\b/g, "<time>"],
	// Ports, before the line/column rule can claim the colon.
	[/\b(?:127\.0\.0\.1|localhost):\d+/gi, "localhost:<port>"],
	// file:line:col and bare line references.
	[/:\d+:\d+/g, ":<line>"],
	[/\bline \d+/gi, "line <line>"],
	// Hex-ish identifiers: git shas, content hashes, addresses.
	[/\b0x[0-9a-f]+\b/gi, "<addr>"],
	[/\b[0-9a-f]{7,40}\b/gi, "<hash>"],
	// Durations and byte counts reported by build tools.
	[/\b\d+(?:\.\d+)?\s?(?:ms|s|sec|secs|seconds|m|min|mins|kb|mb|gb)\b/gi, "<qty>"],
	// PIDs, which vary per machine and per run.
	[/\bpid[= ]\d+/gi, "pid <pid>"],
	// Anything left that is a bare multi-digit run.
	[/\b\d{3,}\b/g, "<n>"],
];

/**
 * Collapse a raw error or command output into a comparable signature.
 *
 * Case is preserved: `TS2307` and `ERR_MODULE_NOT_FOUND` carry meaning that
 * lowercasing would blur into surrounding prose, and they are exactly the
 * tokens worth matching on.
 */
export function normalizeErrorSignature(text: string): string {
	let out = text.slice(0, MAX_SIGNATURE_CHARS * 4);
	for (const [pattern, replacement] of VOLATILE_PATTERNS) {
		out = out.replace(pattern, replacement);
	}
	return out.replace(/\s+/g, " ").trim().slice(0, MAX_SIGNATURE_CHARS);
}

/**
 * Normalize a shell command so two invocations of "the same" command match.
 *
 * Paths are collapsed but flags are kept — `npm test` and `npm test --coverage`
 * are genuinely different commands, and flags are frequently the fix itself.
 */
export function normalizeCommand(command: string): string {
	const collapsed = command
		.replace(/\s+/g, " ")
		.replace(/(?:^|\s)\/(?:[\w.@+-]+\/)+[\w.@+-]+/g, " <path>")
		.trim();
	return collapsed.slice(0, MAX_SIGNATURE_CHARS);
}

/** Filler that carries no rule content, stripped before directives are compared. */
const DIRECTIVE_FILLER =
	/^(?:please|pls|hey|ok|okay|now|also|and|so|just|quickly|can you|could you|would you|i want you to|i'd like you to|let's|lets)\b[\s,]*/i;

/** Words too common to distinguish one directive from another. */
const STOPWORDS = new Set([
	"a",
	"an",
	"and",
	"are",
	"as",
	"at",
	"be",
	"but",
	"by",
	"do",
	"for",
	"from",
	"in",
	"is",
	"it",
	"its",
	"of",
	"on",
	"or",
	"that",
	"the",
	"then",
	"this",
	"to",
	"we",
	"when",
	"with",
	"you",
	"your",
]);

/**
 * Normalize a user directive for clustering: lowercase, strip filler and
 * punctuation, collapse whitespace. Unlike error signatures these are compared
 * loosely, so case and symbols are noise.
 */
export function normalizeDirective(text: string): string {
	let out = text.trim();
	// Filler can stack ("ok so please also ..."), so strip until it stops matching.
	for (let i = 0; i < 4 && DIRECTIVE_FILLER.test(out); i++) {
		out = out.replace(DIRECTIVE_FILLER, "");
	}
	return out
		.toLowerCase()
		.replace(/[^\w\s@./+-]/g, " ")
		.replace(/\s+/g, " ")
		.trim();
}

/** Content words of a directive, for overlap comparisons against existing rules. */
export function contentWords(text: string): string[] {
	return [...new Set(normalizeDirective(text).split(" "))].filter((w) => w.length > 2 && !STOPWORDS.has(w));
}

/**
 * Fraction of `words` that appear in `haystack`, 0 when there is nothing to
 * compare. Used to decide whether a proposed rule is already written down.
 */
export function wordOverlap(words: string[], haystack: string): number {
	if (words.length === 0) return 0;
	const hay = ` ${normalizeDirective(haystack)} `;
	const hits = words.filter((w) => hay.includes(` ${w} `)).length;
	return hits / words.length;
}

/** Approvals and acknowledgements. These are the bulk of user turns and none are rules. */
const ACKNOWLEDGEMENT =
	/^(?:y|n|ok|okay|yes|no|yep|yeah|nope|sure|thanks|thank you|ty|go|go ahead|continue|proceed|do it|next|done|good|great|nice|perfect|lgtm|cool|k|fine|stop|wait|hmm|\?+|\.+)$/i;

/**
 * Imperative or corrective phrasing — the shape a rule takes when it is spoken
 * aloud. A directive that matches none of these is almost always a task
 * ("add a button here"), which is worth doing once and never worth writing down.
 */
const RULE_SHAPED =
	/\b(?:always|never|don'?t|do not|avoid|make sure|ensure|must|should|prefer|instead|remember|from now on|going forward|every time|each time|whenever|stop|use|run|keep|only|no need to)\b/i;

/** Minimum length before a directive is worth considering at all. */
const MIN_DIRECTIVE_CHARS = 12;
/** Above this a "directive" is a task description or a pasted spec, not a rule. */
const MAX_DIRECTIVE_CHARS = 400;

/**
 * Whether a user message is worth mining as a potential rule.
 *
 * Deliberately strict. The extractor's output is read by a model with a limited
 * budget, and recall costs precision: every task statement that slips through
 * displaces a real recurring directive in the ranking.
 */
export function isRuleShapedDirective(text: string): boolean {
	const trimmed = text.trim();
	if (trimmed.length < MIN_DIRECTIVE_CHARS || trimmed.length > MAX_DIRECTIVE_CHARS) return false;
	if (ACKNOWLEDGEMENT.test(trimmed)) return false;
	// Slash commands are invocations, not instructions.
	if (trimmed.startsWith("/")) return false;
	// Pasted output, diffs and stack traces are not directives.
	if (/^(?:```|diff --git|\s*at\s|\{|\[)/.test(trimmed)) return false;
	if (trimmed.split("\n").length > 6) return false;
	return RULE_SHAPED.test(trimmed);
}

/**
 * Commands whose non-zero exit is a normal answer rather than a failure.
 *
 * `rg`/`grep` exit 1 on "no match", `test`/`diff`/`cmp` exit non-zero to report
 * their result, and a TDD loop is *supposed* to go red. Without this filter the
 * fail→fix extractor is mostly noise, since these are among the most-run
 * commands in any session.
 */
const BENIGN_EXIT_COMMANDS = /^(?:rg|grep|egrep|fgrep|ag|ack|test|\[|diff|cmp|which|command|type|hash|find|fd)\b/;

/** Whether a non-zero exit from this command should be ignored by the fix extractor. */
export function isBenignFailure(command: string): boolean {
	const first = command.trim().replace(/^(?:sudo|env|time|nice)\s+/, "");
	return BENIGN_EXIT_COMMANDS.test(first);
}
