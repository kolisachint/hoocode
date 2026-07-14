/**
 * Native `.agents-plugin` format — hoocode's own layout and a strict superset of
 * the Claude Code format. Wins precedence when several formats coexist, and is
 * the only format that honors `providers`.
 */

import { createJsonManifestAdapter } from "./jsonManifest.js";

export const agentsFormat = createJsonManifestAdapter({
	id: "agents",
	manifestDir: ".agents-plugin",
	workspaceRoot: ".agents",
	precedence: 0,
	label: "Native (.agents-plugin)",
	supportsProviders: true,
});
