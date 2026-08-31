import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import { theme as appTheme } from "../../modes/interactive/theme/theme.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import { getTextOutput, invalidArgText, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";
import {
	blockedHostForUrl,
	fetchStatusNote,
	isTruncatedContent,
	resolveWebtoolsTimeoutSecs,
	resolveWebtoolsTLSConfig,
	runWebtools,
	type WebFetchContentStatus,
	type WebFetchResult,
	WebToolsCache,
	type WebtoolsTLSConfig,
} from "./webtools-shared.js";

const DEFAULT_MAX_TOKENS = 4000;
// Hard ceiling so a single fetch can never flood the context window, regardless
// of what the model requests. The binary still applies its own soft cap.
const MAX_TOKENS_CAP = 25000;

/** Clamp a requested token budget into `(0, MAX_TOKENS_CAP]`, defaulting when unset. */
export function clampMaxTokens(requested?: number): number {
	if (!requested || requested <= 0) return DEFAULT_MAX_TOKENS;
	return Math.min(requested, MAX_TOKENS_CAP);
}

const webfetchSchema = Type.Object({
	url: Type.String({ description: "The URL to fetch (http or https)" }),
	maxTokens: Type.Optional(
		Type.Number({
			description: `Soft cap on returned output size in estimated tokens (default: ${DEFAULT_MAX_TOKENS}, max: ${MAX_TOKENS_CAP})`,
		}),
	),
	output: Type.Optional(
		Type.Union([Type.Literal("text"), Type.Literal("markdown")], {
			description:
				"Output format: 'text' (default, most token-efficient, links as [N] with a trailing reference block) or 'markdown' (inline links).",
		}),
	),
	offset: Type.Optional(
		Type.Number({
			description:
				"Byte offset into the page's extracted text to read from, for continuing a long page. Use the offset the previous fetch reported; windows tile the document exactly, so nothing is skipped or repeated.",
		}),
	),
});

type WebFetchToolInput = Static<typeof webfetchSchema>;

export interface WebFetchToolDetails {
	finalUrl?: string;
	title?: string;
	tokenEstimate?: number;
	/** The page continued past the token budget: what came back is a prefix. */
	truncated?: boolean;
	/** The budget the cut was made at, so the TUI can say what to raise. */
	maxTokens?: number;
	/** Estimated tokens of the whole page, when the binary reports it. */
	totalTokenEstimate?: number;
	/** Where to resume reading, when the binary reports paging offsets. */
	nextOffset?: number;
	contentType?: string;
	media?: string;
	/** Non-"ok" means extraction produced nothing usable; see {@link WebFetchContentStatus}. */
	status?: WebFetchContentStatus;
}

export interface WebFetchToolOptions extends WebtoolsTLSConfig {
	/** Override the result cache (mainly for tests). */
	cache?: WebToolsCache<WebFetchResult>;
	/** Effective per-request timeout (seconds); falls back to env/default when unset. */
	timeoutSecs?: number;
}

function formatWebfetchCall(args: { url?: string; output?: string } | undefined): string {
	const url = str(args?.url);
	const urlDisplay = url === null ? invalidArgText(appTheme) : url ? url : appTheme.fg("toolOutput", "...");
	const format = args?.output === "markdown" ? appTheme.fg("muted", " (markdown)") : "";
	return appTheme.fg("toolTitle", appTheme.bold("webfetch ")) + appTheme.fg("accent", urlDisplay) + format;
}

function formatWebfetchResult(
	result: { content: Array<{ type: string; text?: string }>; details?: WebFetchToolDetails },
	options: ToolRenderResultOptions,
	showImages: boolean,
): string {
	const output = getTextOutput(result as any, showImages).trim();
	let text = "";
	if (output) {
		const lines = output.split("\n");
		const maxLines = options.expanded ? lines.length : 15;
		const displayLines = lines.slice(0, maxLines);
		const remaining = lines.length - maxLines;
		text += `\n${displayLines.map((line) => appTheme.fg("toolOutput", line)).join("\n")}`;
		if (remaining > 0) {
			text += `${appTheme.fg("muted", `\n... (${remaining} more lines,`)} ${keyHint("app.tools.expand", "to expand")})`;
		}
	}
	const tokenEstimate = result.details?.tokenEstimate;
	if (tokenEstimate !== undefined) {
		// A cut page and a complete one cost the same at the budget, so the number
		// alone reads as "this is the page". Mark the ones that are a prefix.
		const cut = result.details?.truncated
			? appTheme.fg("warning", ` (truncated at ${result.details.maxTokens ?? tokenEstimate})`)
			: "";
		text += `\n${appTheme.fg("muted", `~${tokenEstimate} tokens`)}${cut}`;
	}
	return text;
}

/**
 * What to tell the model when a page did not fit.
 *
 * With paging offsets it is a position to resume at, which is the whole point:
 * the rest of the document is one call away and costs another window, not
 * another copy of the page. Without them (an older binary) the only truthful
 * advice is a larger budget.
 */
function continuationNote(result: WebFetchResult, offset: number, maxTokens: number): string {
	const next = result.next_offset;
	if (next === undefined) {
		return `output stopped at the ${maxTokens}-token budget; the page continues past this point. Re-fetch with a larger maxTokens (up to ${MAX_TOKENS_CAP}) for more, or fetch a more specific URL or #anchor.`;
	}
	const total = result.total_token_estimate;
	const progress =
		total !== undefined ? `~${result.token_estimate} of ~${total} tokens` : `${result.token_estimate} tokens`;
	return `showing bytes ${result.offset ?? offset}-${next} of ${result.total_bytes ?? "?"} (${progress}); continue with offset=${next}`;
}

export function createWebFetchToolDefinition(
	cwd: string,
	options?: WebFetchToolOptions,
): ToolDefinition<typeof webfetchSchema, WebFetchToolDetails | undefined> {
	const cache = options?.cache ?? new WebToolsCache<WebFetchResult>();
	// Resolve CA/insecure plumbing and the request timeout once (settings
	// overrides, else env) and thread them into every spawn; not hardcoded.
	const tlsConfig = resolveWebtoolsTLSConfig(options);
	const timeoutSecs = resolveWebtoolsTimeoutSecs(options?.timeoutSecs);
	return {
		name: "webfetch",
		label: "webfetch",
		description:
			"Fetch a web page (or JSON/text resource) and return token-efficient, reference-style content. HTML is extracted to clean text; links become inline [N] markers with full URLs in a trailing reference block. Returns title, final URL (after redirects), and an estimated token count. Off by default; enabled with --enable-webtools.",
		promptSnippet: "Fetch a URL and return clean, token-efficient page content",
		promptGuidelines: [
			"Use webfetch to read a known URL instead of bash curl/wget; it returns clean extracted text with reference-style [N] links, not raw HTML.",
			"A fetch that reports it stopped at its token budget returned a prefix, not the page. When it names a continue offset, pass that as `offset` to read the next window; windows tile exactly, so nothing is skipped or repeated. Only keep going while the answer is genuinely further down — a more specific URL or #anchor is usually cheaper than paging a whole document.",
		],
		parameters: webfetchSchema,
		async execute(_toolCallId, { url, maxTokens, output, offset }: WebFetchToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");

			// Policy gate (.webtoolsignore). SSRF/private-address blocking lives in
			// the binary; this is host-level allow/deny policy only.
			const blockedHost = blockedHostForUrl(cwd, url);
			if (blockedHost) {
				throw new Error(`Blocked by .webtoolsignore policy: ${blockedHost}`);
			}

			const effectiveMaxTokens = clampMaxTokens(maxTokens);
			const format = output ?? "text";
			const effectiveOffset = Number.isFinite(offset) && offset !== undefined ? Math.max(0, Math.floor(offset)) : 0;
			const cacheKey = `${format}:${effectiveMaxTokens}:${effectiveOffset}:${url}`;

			const args = ["--url", url, "--max-tokens", String(effectiveMaxTokens), "--output", format];
			// Only sent when non-zero: an older binary rejects the unknown flag,
			// and a page read from the start never needs it.
			if (effectiveOffset > 0) args.push("--offset", String(effectiveOffset));
			const result = await cache.getOrCompute(cacheKey, signal, (sig) =>
				runWebtools<WebFetchResult>("fetch", args, cwd, sig, timeoutSecs, tlsConfig),
			);

			const header = result.title ? `${result.title}\n${result.final_url}\n\n` : `${result.final_url}\n\n`;
			// An empty body and a JavaScript-rendered shell look identical in the
			// content alone. Say which it was, so the page is not read as "nothing
			// to say" when it simply needs a browser.
			const note = fetchStatusNote(result.status);
			let body = note ? `${result.content}\n\n[webtools: ${note}]`.trimStart() : result.content;

			// A cut page used to end in a bare elision marker: the model could see
			// that something was missing but had no way to act on it, so a long
			// document was a dead end rather than a first page. Say where the
			// window sits and how to continue past it.
			//
			// The binary's own flag is authoritative; the marker is the fallback
			// for binaries older than the paging fields, where the only honest
			// advice is a larger budget because there is no offset to resume at.
			const truncated = result.truncated ?? isTruncatedContent(result.content);
			if (truncated) {
				body += `\n\n[webtools: ${continuationNote(result, effectiveOffset, effectiveMaxTokens)}]`;
			}

			return {
				content: [{ type: "text" as const, text: header + body }],
				details: {
					finalUrl: result.final_url,
					title: result.title,
					tokenEstimate: result.token_estimate,
					truncated,
					maxTokens: effectiveMaxTokens,
					totalTokenEstimate: result.total_token_estimate,
					nextOffset: result.next_offset,
					contentType: result.content_type,
					media: result.media,
					status: result.status,
				},
			};
		},
		renderCall(args, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatWebfetchCall(args));
			return text;
		},
		renderResult(result, options, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatWebfetchResult(result as any, options, context.showImages));
			return text;
		},
	};
}

export function createWebFetchTool(cwd: string, options?: WebFetchToolOptions): AgentTool<typeof webfetchSchema> {
	return wrapToolDefinition(createWebFetchToolDefinition(cwd, options));
}
