/**
 * /cost — session token and cost totals.
 *
 * Walks every assistant message in the current session and sums tokens + cost,
 * then prints a session total followed by a per-model breakdown.
 * Per-tool attribution is intentionally not shown — tokens aren't tracked
 * per-tool, and any heuristic would be misleading.
 */

import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";

export function setupCost(pi: ExtensionAPI): void {
	pi.registerCommand("cost", {
		description: "Show session token and cost totals, broken down by model.",
		getArgumentCompletions: () => [],
		handler: async (_args: string, ctx: ExtensionCommandContext): Promise<void> => {
			type Totals = { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number };
			const empty = (): Totals => ({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 });
			const total = empty();
			const perModel = new Map<string, Totals>();
			let assistantTurns = 0;

			for (const entry of ctx.sessionManager.getEntries()) {
				if (entry.type !== "message" || entry.message.role !== "assistant") continue;
				const u = entry.message.usage;
				if (!u) continue;
				assistantTurns++;
				total.input += u.input;
				total.output += u.output;
				total.cacheRead += u.cacheRead;
				total.cacheWrite += u.cacheWrite;
				total.cost += u.cost.total;

				const key = `${entry.message.provider}/${entry.message.model}`;
				const t = perModel.get(key) ?? empty();
				t.input += u.input;
				t.output += u.output;
				t.cacheRead += u.cacheRead;
				t.cacheWrite += u.cacheWrite;
				t.cost += u.cost.total;
				perModel.set(key, t);
			}

			if (assistantTurns === 0) {
				ctx.ui.notify("No assistant turns yet — nothing to cost.", "info");
				return;
			}

			const fmt = (n: number) => n.toLocaleString();
			const fmtCost = (n: number) => `$${n.toFixed(4)}`;
			const lines: string[] = [];
			lines.push(`Session totals (${assistantTurns} assistant turn${assistantTurns === 1 ? "" : "s"})`);
			lines.push(`  Input         ${fmt(total.input)}`);
			lines.push(`  Output        ${fmt(total.output)}`);
			lines.push(`  Cache read    ${fmt(total.cacheRead)}`);
			lines.push(`  Cache write   ${fmt(total.cacheWrite)}`);
			lines.push(`  Cost          ${fmtCost(total.cost)}`);

			if (perModel.size > 1) {
				lines.push("");
				lines.push("By model:");
				const sorted = [...perModel.entries()].sort((a, b) => b[1].cost - a[1].cost);
				for (const [key, t] of sorted) {
					lines.push(`  ${key}: ${fmt(t.input)} in / ${fmt(t.output)} out  ${fmtCost(t.cost)}`);
				}
			}

			ctx.ui.notify(lines.join("\n"), "info");
		},
	});
}
