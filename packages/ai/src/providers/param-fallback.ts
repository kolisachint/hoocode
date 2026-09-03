/**
 * Recovery for endpoints that refuse the optional params we send.
 *
 * Every OpenAI-compatible request carries a few params that are not part of
 * what a chat-completions endpoint has to accept: `prompt_cache_retention` and
 * `prompt_cache_key` buy the long prompt cache, `store` opts out of
 * server-side retention, `stream_options` asks for usage in the stream.
 * OpenAI ignores the ones a model does not use. A gateway that validates its
 * request schema strictly does not — a LiteLLM-style proxy in front of, say,
 * Qwen answers `422 prompt_cache_retention: Extra inputs are not permitted`
 * and the request never reaches the model at all.
 *
 * None of these params change the answer, so a rejection is recoverable: drop
 * the ones the endpoint named and send the request again. Which params an
 * endpoint refused is remembered per base URL, so the cost is one wasted round
 * trip per process rather than one per request, and every later request is
 * built without them from the start — including the summarization behind
 * auto-compaction, which calls the provider directly and so cannot be patched
 * by whatever sanitising the main loop's `onPayload` hook does.
 */

/**
 * Params safe to drop when an endpoint rejects them. Deliberately a fixed
 * list: a 4xx naming `messages` or `tools` is a bug in the request we built,
 * not a param to quietly throw away.
 */
export const DROPPABLE_PARAMS = ["prompt_cache_retention", "prompt_cache_key", "store", "stream_options"] as const;

export type DroppableParam = (typeof DROPPABLE_PARAMS)[number];

/** Params each base URL has been observed rejecting, for this process. */
const rejectedByBaseUrl = new Map<string, Set<DroppableParam>>();

const NONE_REJECTED: ReadonlySet<DroppableParam> = new Set();

/** The params `baseUrl` has already refused. Empty for an endpoint that never has. */
export function rejectedParamsFor(baseUrl: string): ReadonlySet<DroppableParam> {
	return rejectedByBaseUrl.get(baseUrl) ?? NONE_REJECTED;
}

/** Remember that `baseUrl` rejected these, so the next request omits them. */
export function noteRejectedParams(baseUrl: string, params: Iterable<DroppableParam>): void {
	let rejected = rejectedByBaseUrl.get(baseUrl);
	if (!rejected) {
		rejected = new Set();
		rejectedByBaseUrl.set(baseUrl, rejected);
	}
	for (const param of params) {
		rejected.add(param);
	}
}

/** Forget everything learned so far. Tests only — the map is process-lifetime state. */
export function resetRejectedParams(): void {
	rejectedByBaseUrl.clear();
}

/**
 * The text an error carries about the request, both the SDK's own message
 * (`422 prompt_cache_retention: Extra inputs are not permitted`) and the raw
 * body it parsed, since proxies differ in which one names the field.
 */
function errorText(error: unknown): string {
	if (!error || typeof error !== "object") return "";
	const parts: string[] = [];
	const message = (error as { message?: unknown }).message;
	if (typeof message === "string") parts.push(message);
	const body = (error as { error?: unknown }).error;
	if (body !== undefined) {
		try {
			parts.push(JSON.stringify(body));
		} catch {
			// A body that will not serialize tells us nothing; the message still might.
		}
	}
	return parts.join(" ");
}

/**
 * The droppable params an error blames, if it is the kind of error that blames
 * params at all.
 *
 * Only a 4xx is about the request we sent: a 5xx or a dropped connection says
 * nothing about which params the endpoint accepts, and dropping caching on the
 * strength of one would be a silent, permanent downgrade. Matching is on whole
 * words so `store` does not answer for `datastore`.
 */
export function droppableParamsNamedBy(error: unknown): DroppableParam[] {
	const status = (error as { status?: unknown } | null | undefined)?.status;
	if (typeof status !== "number" || status < 400 || status >= 500) return [];
	const text = errorText(error);
	if (!text) return [];
	return DROPPABLE_PARAMS.filter((param) => new RegExp(String.raw`\b${param}\b`).test(text));
}
