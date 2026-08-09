/**
 * Force every test process to treat a throwaway directory as the user's home.
 *
 * Plugin locations are user-scoped: marketplace installs land in
 * `~/.agents/plugins/`, and an authored plugin lands in `~/.claude/skills/<id>/`
 * — a directory Claude Code loads on its next session. A test that exercises
 * authoring without redirecting the agent dir therefore writes real plugins into
 * the developer's real home, where they stay installed. That is not theoretical:
 * it happened once, and the sixteen fixture directories it left behind were only
 * distinguishable from genuine skills by their `.authored.json` marker.
 *
 * Individual suites may still override `HOOCODE_CODING_AGENT_DIR` for their own
 * fixtures. This is the floor, not the mechanism — it guarantees that forgetting
 * to set it pollutes a temp dir instead of `$HOME`.
 */

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll } from "vitest";

const ENV_AGENT_DIR = "HOOCODE_CODING_AGENT_DIR";
const ENV_SKIP_VENDOR_VALIDATE = "HOOCODE_PLUGIN_SKIP_VENDOR_VALIDATE";

let sandbox: string | undefined;
let prior: string | undefined;
let priorSkip: string | undefined;

beforeAll(() => {
	prior = process.env[ENV_AGENT_DIR];
	sandbox = mkdtempSync(join(tmpdir(), "hoo-test-home-"));
	process.env[ENV_AGENT_DIR] = join(sandbox, ".hoocode");

	// Plugin eval shells out to `claude plugin validate` when the CLI is present.
	// That is right in production and ruinous here: it turned one suite from 0.1s
	// into 12s. Tests that mean to exercise the vendor path clear this themselves.
	priorSkip = process.env[ENV_SKIP_VENDOR_VALIDATE];
	process.env[ENV_SKIP_VENDOR_VALIDATE] = "1";
});

afterAll(() => {
	if (prior === undefined) delete process.env[ENV_AGENT_DIR];
	else process.env[ENV_AGENT_DIR] = prior;
	if (priorSkip === undefined) delete process.env[ENV_SKIP_VENDOR_VALIDATE];
	else process.env[ENV_SKIP_VENDOR_VALIDATE] = priorSkip;
	if (sandbox) rmSync(sandbox, { recursive: true, force: true });
});
