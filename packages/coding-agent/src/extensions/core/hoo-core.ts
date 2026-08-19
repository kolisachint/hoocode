/**
 * hoo-core — HooCode built-in core extension (composition root).
 *
 * Each concern lives in its own module in this directory:
 *   - permission-gate.ts    — prompts before bash/write/edit; hard tool/command policy
 *   - mcp-loader.ts         — discovers MCP server configs, connects, registers tools
 *   - modes.ts              — active mode resolution, mode prompts, /mode /plan /approve
 *   - cost.ts               — /cost session token + cost totals
 *   - scaffold.ts           — /new-skill /new-agent /new-command
 *   - ask-options.ts        — the ask_options tool (inline decision pane)
 *   - thinking-escalation.ts — raise thinking after tool errors, then restore
 *   - loop.ts               — /loop cron scheduling + autonomous continuation
 *   - marketplace.ts        — /plugin marketplace + install/remove
 *   - canvas.ts             — /canvas list/open/close for canvas extensions
 *   - prompt-reactive/      — runtime plugin-reuse nudge (reactive to tool/turn cues)
 *   - config.ts             — hoo-config.json types, I/O, and merge rules
 *
 * `bin/hoocode.js` loads this module's default export as the built-in
 * extension factory.
 */

import type { ExtensionAPI } from "../../core/extensions/types.js";
import { setupAskOptions } from "./ask-options.js";
import { setupCanvas } from "./canvas.js";
import { setupCost } from "./cost.js";
import { setupLearn } from "./learn.js";
import { setupLoop } from "./loop.js";
import { setupMarketplace } from "./marketplace.js";
import { setupMcpLoader } from "./mcp-loader.js";
import { setupMode } from "./modes.js";
import { setupPermissionGate } from "./permission-gate.js";
import { setupPromptReactiveNudges } from "./prompt-reactive/nudges.js";
import { setupScaffold } from "./scaffold.js";
import { setupThinkingEscalation } from "./thinking-escalation.js";

function hooCore(pi: ExtensionAPI): void {
	setupPermissionGate(pi);
	setupMcpLoader(pi);
	setupMode(pi);
	setupCost(pi);
	setupScaffold(pi);
	setupAskOptions(pi);
	setupThinkingEscalation(pi);
	setupLoop(pi);
	setupMarketplace(pi);
	setupCanvas(pi);
	setupPromptReactiveNudges(pi);
	setupLearn(pi);
}

hooCore.displayName = "hoo-core";
// Built-in plumbing, not a user-installed extension: it is always present, so
// listing it in the startup resource summary is noise, not information.
hooCore.internal = true;
export default hooCore;
