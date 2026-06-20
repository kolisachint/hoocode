/**
 * Local-inference routing metrics and diagnostic logging.
 *
 * Routing is an optimization, so failures must never surface as errors in the
 * agent UI. This module provides a single, quiet sink for routing diagnostics
 * and per-turn metrics. Output is gated behind HOOCODE_ROUTING_DEBUG so normal
 * sessions stay silent (decision: fall back, log silently, never hard-fail).
 */

import type { TurnKind } from "./local-inference.js";

function debugEnabled(): boolean {
	const v = process.env.HOOCODE_ROUTING_DEBUG;
	return v === "1" || v === "true" || v === "yes";
}

/** Record that an executor turn failed and the caller fell back to primary/raw. */
export function logLocalInferenceFallback(turnKind: TurnKind, error: unknown): void {
	if (!debugEnabled()) return;
	const message = error instanceof Error ? error.message : String(error);
	console.warn(`[local-inference] ${turnKind} fell back to primary: ${message}`);
}

/** Per-turn routing metrics for cost/perf comparison. */
export interface RoutingMetrics {
	turnKind: TurnKind;
	provider: string;
	model: string;
	inputTokens?: number;
	outputTokens?: number;
	latencyMs: number;
	/** Bytes before/after for tool-result compression. */
	bytesBefore?: number;
	bytesAfter?: number;
	fallback: boolean;
}

/** Record per-turn routing metrics (quiet unless HOOCODE_ROUTING_DEBUG). */
export function logRoutingMetrics(m: RoutingMetrics): void {
	if (!debugEnabled()) return;
	const parts = [
		`turn=${m.turnKind}`,
		`model=${m.provider}/${m.model}`,
		`latency=${m.latencyMs}ms`,
		m.inputTokens !== undefined ? `in=${m.inputTokens}` : undefined,
		m.outputTokens !== undefined ? `out=${m.outputTokens}` : undefined,
		m.bytesBefore !== undefined && m.bytesAfter !== undefined ? `bytes=${m.bytesBefore}->${m.bytesAfter}` : undefined,
		m.fallback ? "fallback=true" : undefined,
	].filter((p): p is string => p !== undefined);
	console.error(`[local-inference] ${parts.join(" ")}`);
}
