/**
 * A tiny HTTP server that publishes a HooCode instance's {@link AgentCard} for
 * A2A discovery.
 *
 * This implements only the discovery contract: it serves the card as JSON at
 * the well-known paths and exposes a small human-readable index at `/`. It is
 * intentionally not a full A2A JSON-RPC endpoint — the card's `capabilities`
 * advertise that task execution is not available here.
 */

import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AgentCard } from "./types.js";
import { AGENT_CARD_WELL_KNOWN_PATH, AGENT_CARD_WELL_KNOWN_PATH_ALIAS } from "./types.js";

export interface A2ADiscoveryServerOptions {
	/** Port to listen on. Use 0 to let the OS pick a free port. */
	port?: number;
	/** Host/interface to bind. Defaults to loopback (127.0.0.1). */
	host?: string;
}

export interface A2ADiscoveryServer {
	/** The underlying Node HTTP server. */
	server: Server;
	/** Host the server is bound to. */
	host: string;
	/** Port the server is listening on (resolved, so never 0). */
	port: number;
	/** Base URL the card is reachable at. */
	url: string;
	/** Fully-qualified URL of the AgentCard document. */
	cardUrl: string;
	/** Stop the server and resolve once closed. */
	close(): Promise<void>;
}

const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_PORT = 41411;

function sendJson(res: ServerResponse, status: number, body: unknown): void {
	const payload = JSON.stringify(body, null, 2);
	res.writeHead(status, {
		"content-type": "application/json; charset=utf-8",
		"content-length": Buffer.byteLength(payload),
		// The card is public metadata; allow browser-based agents to read it.
		"access-control-allow-origin": "*",
	});
	res.end(payload);
}

function sendText(res: ServerResponse, status: number, body: string): void {
	res.writeHead(status, {
		"content-type": "text/plain; charset=utf-8",
		"content-length": Buffer.byteLength(body),
	});
	res.end(body);
}

/**
 * Route a single request. Exported so the routing logic can be unit-tested
 * without binding a socket.
 */
export function handleRequest(card: AgentCard, req: IncomingMessage, res: ServerResponse): void {
	// Discovery is a read-only surface; anything but GET/HEAD is rejected.
	if (req.method !== "GET" && req.method !== "HEAD") {
		res.setHeader("allow", "GET, HEAD");
		sendText(res, 405, "Method Not Allowed");
		return;
	}

	const path = (req.url ?? "/").split("?")[0];

	if (path === AGENT_CARD_WELL_KNOWN_PATH || path === AGENT_CARD_WELL_KNOWN_PATH_ALIAS) {
		sendJson(res, 200, card);
		return;
	}

	if (path === "/" || path === "/index.html") {
		const skillLines = card.skills.map((skill) => `  - ${skill.name} (${skill.id})`).join("\n");
		sendText(
			res,
			200,
			`${card.name} v${card.version} — A2A discovery\n\n` +
				`AgentCard: ${AGENT_CARD_WELL_KNOWN_PATH}\n\n` +
				`Skills (${card.skills.length}):\n${skillLines}\n`,
		);
		return;
	}

	sendText(res, 404, "Not Found");
}

/**
 * Start an A2A discovery server that serves the given card. Resolves once the
 * server is listening.
 */
export function startA2ADiscoveryServer(
	card: AgentCard,
	options: A2ADiscoveryServerOptions = {},
): Promise<A2ADiscoveryServer> {
	const host = options.host ?? DEFAULT_HOST;
	const requestedPort = options.port ?? DEFAULT_PORT;

	const server = createServer((req, res) => {
		try {
			handleRequest(card, req, res);
		} catch {
			if (!res.headersSent) sendText(res, 500, "Internal Server Error");
			else res.end();
		}
	});

	return new Promise<A2ADiscoveryServer>((resolve, reject) => {
		server.once("error", reject);
		server.listen(requestedPort, host, () => {
			server.removeListener("error", reject);
			const address = server.address();
			const port = typeof address === "object" && address ? address.port : requestedPort;
			// Bracket IPv6 hosts so the URL is well-formed.
			const hostForUrl = host.includes(":") ? `[${host}]` : host;
			const url = `http://${hostForUrl}:${port}`;
			resolve({
				server,
				host,
				port,
				url,
				cardUrl: `${url}${AGENT_CARD_WELL_KNOWN_PATH}`,
				close: () =>
					new Promise<void>((closeResolve, closeReject) => {
						server.close((err) => (err ? closeReject(err) : closeResolve()));
					}),
			});
		});
	});
}
