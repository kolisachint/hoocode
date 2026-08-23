/**
 * Canonical built-in mode prompts and the default active mode.
 *
 * These are the fallback system prompts hoocode ships for its four built-in
 * modes (ask / plan / build / debug). They are resolved only when a project or
 * user has not supplied a `modes/{name}/system.md` override.
 *
 * The prompts themselves live in `templates/modes/<mode>/system.md` and reach
 * this module through the build-time embed. They used to be a second, hand-
 * written copy right here, and the two had already drifted: `/init` scaffolds
 * the template copy into a project, so a user who ran it got one set of mode
 * rules and a user who did not got another, differently worded set. Prose has
 * one home now; this module is the typed accessor for it.
 *
 * Kept as a standalone module (rather than living inside the internal `hoo-core`
 * extension's `modes` module) so downstream apps embedding hoocode can import,
 * inspect, and extend the shipped prompts without copy-pasting them.
 */

import { EMBEDDED_MODES } from "../init-templates.generated.js";

/** Mode used when no `active_mode` is configured. */
export const DEFAULT_MODE = "build";

/**
 * Built-in fallback system prompts keyed by mode name.
 *
 * The plan-mode prompt carries a `{{PLAN_PATH}}` token that the mode extension
 * substitutes per session; a consumer rendering these itself must do the same.
 */
export const DEFAULT_MODE_PROMPTS: Record<string, string> = EMBEDDED_MODES;
