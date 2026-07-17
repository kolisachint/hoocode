import { afterEach, describe, expect, test } from "vitest";
import { buildAgentCard, resolveActiveTools } from "../src/core/a2a/agent-card.js";
import { type A2ADiscoveryServer, startA2ADiscoveryServer } from "../src/core/a2a/server.js";

function makeCard(url: string) {
	return buildAgentCard({ url, version: "9.9.9", activeTools: resolveActiveTools() });
}

let running: A2ADiscoveryServer | undefined;

afterEach(async () => {
	if (running) {
		await running.close();
		running = undefined;
	}
});

describe("startA2ADiscoveryServer", () => {
	test("serves the AgentCard at /.well-known/agent.json", async () => {
		running = await startA2ADiscoveryServer(makeCard("http://placeholder"), { port: 0 });
		const res = await fetch(`${running.url}/.well-known/agent.json`);
		expect(res.status).toBe(200);
		expect(res.headers.get("content-type")).toContain("application/json");
		const card = await res.json();
		expect(card.name).toBe("HooCode");
		expect(card.version).toBe("9.9.9");
		expect(Array.isArray(card.skills)).toBe(true);
	});

	test("serves the card at the agent-card.json alias too", async () => {
		running = await startA2ADiscoveryServer(makeCard("http://placeholder"), { port: 0 });
		const res = await fetch(`${running.url}/.well-known/agent-card.json`);
		expect(res.status).toBe(200);
		const card = await res.json();
		expect(card.name).toBe("HooCode");
	});

	test("exposes a human-readable index at /", async () => {
		running = await startA2ADiscoveryServer(makeCard("http://placeholder"), { port: 0 });
		const res = await fetch(`${running.url}/`);
		expect(res.status).toBe(200);
		expect(res.headers.get("content-type")).toContain("text/plain");
		const body = await res.text();
		expect(body).toContain("A2A discovery");
		expect(body).toContain("shell-execution");
	});

	test("returns 404 for unknown paths", async () => {
		running = await startA2ADiscoveryServer(makeCard("http://placeholder"), { port: 0 });
		const res = await fetch(`${running.url}/nope`);
		expect(res.status).toBe(404);
	});

	test("rejects non-GET methods with 405", async () => {
		running = await startA2ADiscoveryServer(makeCard("http://placeholder"), { port: 0 });
		const res = await fetch(`${running.url}/.well-known/agent.json`, { method: "POST" });
		expect(res.status).toBe(405);
		expect(res.headers.get("allow")).toContain("GET");
	});

	test("resolves the bound port when asked for port 0", async () => {
		running = await startA2ADiscoveryServer(makeCard("http://placeholder"), { port: 0 });
		expect(running.port).toBeGreaterThan(0);
		expect(running.cardUrl).toBe(`${running.url}/.well-known/agent.json`);
	});
});
