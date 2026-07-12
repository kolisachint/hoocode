import { type Component, truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import type { AgentSession } from "../../../core/agent-session.js";
import type { ReadonlyFooterDataProvider } from "../../../core/footer-data-provider.js";
import { taskStore } from "../../../core/task-store.js";
import { BRAND_MARK, GIT_BRANCH_GLYPH } from "../brand.js";
import { theme } from "../theme/theme.js";

/**
 * Assemble one footer line: `left` flush left, `right` flush right when it fits
 * (≥2 cols between), padded to the full width. When it doesn't fit, drop `right`
 * and pad; when even `left` overflows, truncate it. Width math runs on the plain
 * strings; the styled strings carry the colour. Every returned line is exactly
 * `width` cells or fewer — the invariant the footer-width tests hold us to.
 */
function assembleLine(width: number, leftPlain: string, leftStyled: string, rightPlain = "", rightStyled = ""): string {
	const lw = visibleWidth(leftPlain);
	if (rightPlain && lw + 2 + visibleWidth(rightPlain) <= width) {
		return leftStyled + " ".repeat(width - lw - visibleWidth(rightPlain)) + rightStyled;
	}
	if (lw <= width) return leftStyled + " ".repeat(width - lw);
	return truncateToWidth(leftStyled, width, theme.fg("dim", "…"));
}

/** A compact context-fill gauge, coloured by proximity to the auto-compact trip point. */
function contextGauge(percent: number, errorLevel: number, warnLevel: number): { plain: string; styled: string } {
	const CELLS = 8;
	const filled = Math.max(0, Math.min(CELLS, Math.round((percent / 100) * CELLS)));
	const fill = "▰".repeat(filled);
	const track = "▱".repeat(CELLS - filled);
	const color = percent >= errorLevel ? "error" : percent >= warnLevel ? "warning" : "accent";
	return { plain: fill + track, styled: theme.fg(color, fill) + theme.fg("dim", track) };
}

/** Count subagent runs currently in flight, for the footer's live delegation cue. */
function activeSubagentCount(): number {
	return taskStore.list().filter((t) => t.source === "subagent" && t.status === "in_progress").length;
}

/**
 * Sanitize text for display in a single-line status.
 * Removes newlines, tabs, carriage returns, and other control characters.
 */
function sanitizeStatusText(text: string): string {
	// Replace newlines, tabs, carriage returns with space, then collapse multiple spaces
	return text
		.replace(/[\r\n\t]/g, " ")
		.replace(/ +/g, " ")
		.trim();
}

function formatTokens(count: number): string {
	if (count < 1000) return count.toString();
	if (count < 10000) return `${(count / 1000).toFixed(1)}k`;
	if (count < 1000000) return `${Math.round(count / 1000)}k`;
	if (count < 10000000) return `${(count / 1000000).toFixed(1)}M`;
	return `${Math.round(count / 1000000)}M`;
}

/**
 * Footer component that shows pwd, token stats, and context usage.
 * Computes token/context stats from session, gets git branch and extension statuses from provider.
 */
export class FooterComponent implements Component {
	private autoCompactEnabled = true;

	constructor(
		private session: AgentSession,
		private footerData: ReadonlyFooterDataProvider,
	) {}

	setSession(session: AgentSession): void {
		this.session = session;
	}

	setAutoCompactEnabled(enabled: boolean): void {
		this.autoCompactEnabled = enabled;
	}

	/**
	 * No-op: git branch caching now handled by provider.
	 * Kept for compatibility with existing call sites in interactive-mode.
	 */
	invalidate(): void {
		// No-op: git branch is cached/invalidated by provider
	}

	/**
	 * Clean up resources.
	 * Git watcher cleanup now handled by provider.
	 */
	dispose(): void {
		// Git watcher cleanup handled by provider
	}

	render(width: number): string[] {
		const state = this.session.state;

		// Calculate cumulative usage from ALL session entries (not just post-compaction messages)
		let totalInput = 0;
		let totalOutput = 0;
		let totalCacheRead = 0;
		let totalCacheWrite = 0;
		let totalCost = 0;

		for (const entry of this.session.sessionManager.getEntries()) {
			if (entry.type === "message" && entry.message.role === "assistant") {
				totalInput += entry.message.usage.input;
				totalOutput += entry.message.usage.output;
				totalCacheRead += entry.message.usage.cacheRead;
				totalCacheWrite += entry.message.usage.cacheWrite;
				totalCost += entry.message.usage.cost.total;
			}
		}

		// Calculate context usage from session (handles compaction correctly).
		// After compaction, tokens are unknown until the next LLM response.
		const contextUsage = this.session.getContextUsage();
		const contextWindow = contextUsage?.contextWindow ?? state.model?.contextWindow ?? 0;
		const contextPercentValue = contextUsage?.percent ?? 0;
		const contextPercent = contextUsage?.percent !== null ? contextPercentValue.toFixed(1) : "?";

		// Replace home directory with ~
		let pwd = this.session.sessionManager.getCwd();
		const home = process.env.HOME || process.env.USERPROFILE;
		if (home && pwd.startsWith(home)) {
			pwd = `~${pwd.slice(home.length)}`;
		}
		const branch = this.footerData.getGitBranch();
		const sessionName = this.session.sessionManager.getSessionName();
		const modeLabel = this.footerData.getActiveMode();

		// ── Line 1 — identity & location ────────────────────────────────────────
		// Lead with the brand mark + MODE (the agent's guardrail: Ask/Plan/Build/
		// Debug) in bold accent so it is the first thing the eye lands on, then the
		// path, git branch, and session name in descending emphasis. The live
		// subagent count sits flush right — present only while work is delegated.
		const modeUp = modeLabel.toUpperCase();
		const brand = `${BRAND_MARK} ${modeUp}`;
		let l1Plain = `${brand}  ${pwd}`;
		let l1Styled = `${theme.bold(theme.fg("accent", brand))}  ${theme.fg("muted", pwd)}`;
		if (branch) {
			l1Plain += ` ${GIT_BRANCH_GLYPH} ${branch}`;
			l1Styled += ` ${theme.fg("dim", GIT_BRANCH_GLYPH)} ${theme.fg("muted", branch)}`;
		}
		if (sessionName) {
			l1Plain += ` • ${sessionName}`;
			l1Styled += theme.fg("dim", ` • ${sessionName}`);
		}
		const nSub = activeSubagentCount();
		const l1RightPlain = nSub > 0 ? `◇${nSub} running` : "";
		const l1RightStyled = nSub > 0 ? theme.fg("accent", `◇${nSub}`) + theme.fg("dim", " running") : "";
		const line1 = assembleLine(width, l1Plain, l1Styled, l1RightPlain, l1RightStyled);

		// ── Line 2 — session vitals ─────────────────────────────────────────────
		// A context-fill gauge (coloured by proximity to the auto-compact trip
		// point) leads, then token/cost deltas, with the model + thinking level
		// flush right. Numbers read in muted, labels/arrows in dim — a legible
		// hierarchy in place of the old uniform grey.
		let thresholdPercent: number | undefined;
		if (this.autoCompactEnabled && contextWindow > 0) {
			const reserveTokens = this.session.settingsManager.getCompactionSettings().reserveTokens;
			const effective = contextWindow - reserveTokens;
			if (effective > 0) thresholdPercent = (effective / contextWindow) * 100;
		}
		const errorLevel = thresholdPercent !== undefined ? thresholdPercent - 3 : 90;
		const warnLevel = thresholdPercent !== undefined ? thresholdPercent - 10 : 70;
		const autoIndicator =
			thresholdPercent !== undefined
				? ` auto@${thresholdPercent.toFixed(0)}%`
				: this.autoCompactEnabled
					? " auto"
					: "";

		const gauge = contextGauge(contextPercentValue, errorLevel, warnLevel);
		const pctText = contextPercent === "?" ? "?" : `${contextPercent}%`;
		const pctColor =
			contextPercentValue >= errorLevel ? "error" : contextPercentValue >= warnLevel ? "warning" : "muted";
		const winText = `${formatTokens(contextWindow)}${autoIndicator}`;

		const segs: Array<{ plain: string; styled: string }> = [
			{
				plain: `${gauge.plain} ${pctText} ${winText}`,
				styled: `${gauge.styled} ${theme.fg(pctColor, pctText)} ${theme.fg("dim", winText)}`,
			},
		];
		const arrow = (a: string, n: number) => ({
			plain: `${a}${formatTokens(n)}`,
			styled: theme.fg("dim", a) + theme.fg("muted", formatTokens(n)),
		});
		if (totalInput) segs.push(arrow("↑", totalInput));
		if (totalOutput) segs.push(arrow("↓", totalOutput));
		if (totalCacheRead) segs.push(arrow("R", totalCacheRead));
		if (totalCacheWrite) segs.push(arrow("W", totalCacheWrite));
		const usingSubscription = state.model ? this.session.modelRegistry.isUsingOAuth(state.model) : false;
		if (totalCost || usingSubscription) {
			const costStr = `$${totalCost.toFixed(3)}${usingSubscription ? " (sub)" : ""}`;
			segs.push({ plain: costStr, styled: theme.fg("muted", costStr) });
		}
		const l2Plain = segs.map((s) => s.plain).join("  ");
		const l2Styled = segs.map((s) => s.styled).join("  ");

		// Right: model, thinking level, and provider (when several are configured).
		const modelName = state.model?.id || "no-model";
		let r2Plain = modelName;
		let r2Styled = theme.fg("muted", modelName);
		if (state.model?.reasoning) {
			const tl = state.thinkingLevel || "off";
			const tstr = tl === "off" ? "thinking off" : tl;
			r2Plain += ` • ${tstr}`;
			r2Styled += theme.fg("dim", ` • ${tstr}`);
		}
		if (this.footerData.getAvailableProviderCount() > 1 && state.model) {
			// Prepend the provider only when the whole right cluster still fits.
			const withProv = `(${state.model.provider}) ${r2Plain}`;
			if (visibleWidth(l2Plain) + 2 + visibleWidth(withProv) <= width) {
				r2Plain = withProv;
				r2Styled = theme.fg("dim", `(${state.model.provider}) `) + r2Styled;
			}
		}
		const line2 = assembleLine(width, l2Plain, l2Styled, r2Plain, r2Styled);

		const lines = [line1, line2];

		// Add extension statuses on a single line, sorted by key alphabetically
		const extensionStatuses = this.footerData.getExtensionStatuses();
		if (extensionStatuses.size > 0) {
			const sortedStatuses = Array.from(extensionStatuses.entries())
				.sort(([a], [b]) => a.localeCompare(b))
				.map(([, text]) => sanitizeStatusText(text));
			const statusLine = sortedStatuses.join(" ");
			// Truncate to terminal width with dim ellipsis for consistency with footer style
			lines.push(truncateToWidth(statusLine, width, theme.fg("dim", "...")));
		}

		return lines;
	}
}
