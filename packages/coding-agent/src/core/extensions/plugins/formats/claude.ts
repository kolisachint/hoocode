/**
 * Claude Code `.claude-plugin` format. Same on-disk layout as the native format
 * (skills/, commands/, agents/, hooks/hooks.json, inline mcpServers) but under
 * the `.claude-plugin/` marker directory and without `providers` (native-only).
 *
 * This is one of the two vendor formats `ProposePlugin` can scaffold into; keep
 * anything Claude-specific here so it can track upstream changes in isolation.
 */

import { createJsonManifestAdapter } from "./jsonManifest.js";

export const claudeFormat = createJsonManifestAdapter({
	id: "claude",
	manifestDir: ".claude-plugin",
	precedence: 1,
	label: "Claude Code (.claude-plugin)",
	supportsProviders: false,
});
