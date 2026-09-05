/**
 * `hoo` → `/new` → `/reload`, in every order: the frame must say the same thing
 * every time.
 *
 * Startup, a session swap and a reload each rebuild the session's resources and
 * repaint the chrome, and they used to do it from two hand-kept lists — so an
 * edit to `settings.json` landed on one path and not the other, and the banner
 * kept the colours of a theme that was no longer loaded. These tests compare
 * rendered frames, because a list that has drifted is only visible in the
 * frame.
 *
 * The reference every chain is measured against is a *second* mode started with
 * the edit already on disk: whatever chain you took, you must end up looking at
 * what `hoo` would show you now.
 */

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import { createSurfaceHarness, type SurfaceHarness } from "./interactive-surface-harness.js";

const PROJECT = {
	"AGENTS.md": "project rules\n",
	".hoocode/skills/demo/SKILL.md": "---\nname: demo\ndescription: demo skill\n---\nbody\n",
	".hoocode/commands/greet.md": "Say hello to the user\n",
	".hoocode/agents/helper.md": "---\nname: helper\ndescription: helper agent\n---\nyou help\n",
};

/**
 * The edit each chain runs into: one setting the footer draws, one the whole
 * frame is coloured with.
 */
const EDITED_SETTINGS = { compaction: { enabled: false }, theme: "light" };

/** Every chain of `/new` and `/reload` up to three commands deep. */
function chains(depth: number): string[][] {
	const all: string[][] = [];
	const walk = (prefix: string[]) => {
		if (prefix.length > 0) all.push(prefix);
		if (prefix.length === depth) return;
		for (const command of ["/new", "/reload"]) walk([...prefix, command]);
	};
	walk([]);
	return all;
}

describe("session surface stays in sync across /new and /reload", () => {
	const open: SurfaceHarness[] = [];

	async function harness(options?: Parameters<typeof createSurfaceHarness>[0]) {
		const created = await createSurfaceHarness({ project: PROJECT, ...options });
		open.push(created);
		return created;
	}

	afterEach(() => {
		while (open.length > 0) open.pop()?.cleanup();
	});

	test("a fresh start reports the project's resources", async () => {
		const startup = (await harness()).surface();
		expect(startup).toContain("1 skill");
		expect(startup).toContain("1 command");
		expect(startup).toContain("context AGENTS.md");
		expect(startup).toContain("auto@");
	});

	test.each(chains(3).map((chain) => [chain.join(" "), chain] as const))(
		"hoo %s lands on the surface a fresh hoo would show",
		async (_label, chain) => {
			// What `hoo` shows when the edited settings were there all along.
			const fresh = await harness({ settings: EDITED_SETTINGS });
			const reference = fresh.surface();
			const referencePalette = fresh.palette();
			expect(reference).not.toContain("auto@");

			// A session that was already running when the edit landed.
			const running = await harness();
			expect(running.surface()).toContain("auto@");
			expect(running.palette()).not.toEqual(referencePalette);
			writeFileSync(join(running.agentDir, "settings.json"), JSON.stringify(EDITED_SETTINGS));

			for (const command of chain) {
				await running.submit(command);
				const where = `after "${command}" in [${chain.join(" ")}]`;
				expect(running.surface(), where).toBe(reference);
				expect(running.palette(), where).toEqual(referencePalette);
			}
		},
		30000,
	);

	test("a theme edit repaints the banner through either path", async () => {
		const running = await harness();
		const banner = () => running.rawFrame().split("\n").slice(0, 3).join("\n");
		const darkBanner = banner();

		writeFileSync(join(running.agentDir, "settings.json"), JSON.stringify({ theme: "light" }));

		// The banner holds its text with the colours already baked in, so a theme
		// swap has to rebuild it rather than merely invalidate it.
		await running.submit("/new");
		const repainted = banner();
		expect(repainted).not.toBe(darkBanner);

		await running.submit("/reload");
		expect(banner()).toBe(repainted);
	});

	test("a command added on disk appears through either path", async () => {
		const running = await harness();
		expect(running.surface()).toContain("1 command");

		writeFileSync(join(running.cwd, ".hoocode/commands/second.md"), "Second command\n");

		await running.submit("/reload");
		const afterReload = running.surface();
		expect(afterReload).toContain("2 commands");

		await running.submit("/new");
		expect(running.surface()).toBe(afterReload);
	});

	test("the resource listing is drawn once per session change", async () => {
		const running = await harness();
		// One listing means one split point in the frame.
		const listings = () => running.screenshot().split("context AGENTS.md").length - 1;
		expect(listings()).toBe(1);

		await running.submit("/new");
		expect(listings()).toBe(1);

		await running.submit("/reload");
		expect(listings()).toBe(1);
	});
});
