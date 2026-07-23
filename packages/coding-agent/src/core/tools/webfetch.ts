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
	resolveWebtoolsTimeoutSecs,
	resolveWebtoolsTLSConfig,
	runWebtools,
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
});

export type WebFetchToolInput = Static<typeof webfetchSchema>;

export interface WebFetchToolDetails {
	finalUrl?: string;
	title?: string;
	tokenEstimate?: number;
	contentType?: string;
	media?: string;
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
		text += `\n${appTheme.fg("muted", `~${tokenEstimate} tokens`)}`;
	}
	return text;
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
		],
		parameters: webfetchSchema,
		async execute(_toolCallId, { url, maxTokens, output }: WebFetchToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");

			// Policy gate (.webtoolsignore). SSRF/private-address blocking lives in
			// the binary; this is host-level allow/deny policy only.
			const blockedHost = blockedHostForUrl(cwd, url);
			if (blockedHost) {
				throw new Error(`Blocked by .webtoolsignore policy: ${blockedHost}`);
			}

			const effectiveMaxTokens = clampMaxTokens(maxTokens);
			const format = output ?? "text";
			const cacheKey = `${format}:${effectiveMaxTokens}:${url}`;

			const args = ["--url", url, "--max-tokens", String(effectiveMaxTokens), "--output", format];
			const result = await cache.getOrCompute(cacheKey, signal, (sig) =>
				runWebtools<WebFetchResult>("fetch", args, cwd, sig, timeoutSecs, tlsConfig),
			);

			const header = result.title ? `${result.title}\n${result.final_url}\n\n` : `${result.final_url}\n\n`;
			return {
				content: [{ type: "text" as const, text: header + result.content }],
				details: {
					finalUrl: result.final_url,
					title: result.title,
					tokenEstimate: result.token_estimate,
					contentType: result.content_type,
					media: result.media,
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
