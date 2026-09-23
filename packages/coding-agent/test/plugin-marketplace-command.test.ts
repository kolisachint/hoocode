/**
 * `/plugin marketplace add <path>`, through the command handler a person types into.
 *
 * The command used to `join(cwd, loc)` a local location, which glues an absolute
 * path onto the workspace: `/plugin marketplace add /home/me/drawio-canvas` looked
 * for `<cwd>/home/me/drawio-canvas` and reported "Path not found". Relative
 * paths happened to work, which is why it went unnoticed.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import { marketplaceStorePath } from "../src/core/extensions/plugins/locations.js";
import { readMarketplaceStore } from "../src/core/extensions/plugins/marketplace.js";
import { setupMarketplace } from "../src/extensions/core/marketplace.js";

type Handler = (args: string, ctx: unknown) => Promise<void>;

describe("/plugin marketplace add with a local path", () => {
	let root: string;
	let cwd: string;
	let market: string;
	let priorAgentDir: string | undefined;
	let handler: Handler;
	let notifications: { message: string; type?: string }[];

	beforeEach(() => {
		root = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-market-cmd-"));
		cwd = path.join(root, "workspace");
		market = path.join(root, "elsewhere", "my-market");
		fs.mkdirSync(cwd, { recursive: true });
		fs.mkdirSync(path.join(market, ".agents-plugin"), { recursive: true });
		fs.writeFileSync(
			path.join(market, ".agents-plugin", "marketplace.json"),
			JSON.stringify({ name: "my-market", owner: { name: "me" }, plugins: [{ name: "thing", source: "./" }] }),
		);
		priorAgentDir = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = path.join(root, "home", ".hoocode");
		fs.mkdirSync(process.env[ENV_AGENT_DIR] as string, { recursive: true });
		notifications = [];
		const hoo = {
			registerCommand: (_name: string, def: { handler: Handler }) => {
				handler = def.handler;
			},
		} as never;
		setupMarketplace(hoo);
	});

	afterEach(() => {
		if (priorAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = priorAgentDir;
		fs.rmSync(root, { recursive: true, force: true });
	});

	const run = (args: string) =>
		handler(args, {
			cwd,
			hasUI: false,
			ui: { notify: (message: string, type?: string) => notifications.push({ message, type }) },
		});

	it("adds an absolute path as it is, not joined onto the workspace", async () => {
		await run(`marketplace add ${market}`);
		expect(notifications.map((note) => note.message).join("\n")).toContain('Added marketplace "my-market"');
		const records = readMarketplaceStore(marketplaceStorePath());
		expect(records.find((record) => record.location === market)?.dir).toBe(market);
	});

	it("still resolves a relative path against the workspace", async () => {
		await run(`marketplace add ${path.relative(cwd, market)}`);
		expect(notifications.map((note) => note.message).join("\n")).toContain('Added marketplace "my-market"');
		const records = readMarketplaceStore(marketplaceStorePath());
		expect(records[0]?.dir).toBe(market);
	});
});
