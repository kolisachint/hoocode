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

let sandbox: string | undefined;
let prior: string | undefined;

beforeAll(() => {
	prior = process.env[ENV_AGENT_DIR];
	sandbox = mkdtempSync(join(tmpdir(), "hoo-test-home-"));
	process.env[ENV_AGENT_DIR] = join(sandbox, ".hoocode");
});

afterAll(() => {
	if (prior === undefined) delete process.env[ENV_AGENT_DIR];
	else process.env[ENV_AGENT_DIR] = prior;
	if (sandbox) rmSync(sandbox, { recursive: true, force: true });
});
