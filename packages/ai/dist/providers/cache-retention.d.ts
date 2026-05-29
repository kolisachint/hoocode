import type { CacheRetention } from "../types.js";
/**
 * Resolve cache retention preference.
 *
 * Default is "long" — Anthropic 1h / OpenAI 24h prompt cache. Re-using cached
 * prompts past the 5-minute short-TTL window is roughly 10x cheaper than the
 * un-cached read. Models that don't advertise `supportsLongCacheRetention`
 * silently fall back to the provider's default ephemeral cache, so defaulting
 * to "long" is safe.
 *
 * Opt out with HOOCODE_CACHE_RETENTION=short (5-min ephemeral) or =none
 * (disable caching). The legacy PI_CACHE_RETENTION env var is honored too.
 */
export declare function resolveCacheRetention(cacheRetention?: CacheRetention): CacheRetention;
//# sourceMappingURL=cache-retention.d.ts.map