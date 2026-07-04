/**
 * Compact, terminal-friendly duration formatting shared by every surface that
 * shows a subagent's elapsed time (task panel rows/header, TaskOutput roster),
 * so the same run never reads "94s" in one place and "1m34s" in another.
 */
export function formatDurationSecs(secs: number): string {
	const s = Math.max(0, secs);
	if (s < 10) return `${s.toFixed(1)}s`;
	if (s < 60) return `${Math.round(s)}s`;
	const mins = Math.floor(s / 60);
	const rem = Math.round(s % 60);
	return `${mins}m${rem.toString().padStart(2, "0")}s`;
}
