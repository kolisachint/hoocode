/**
 * Remote MCP server connections over the official @modelcontextprotocol/sdk
 * client transports.
 *
 *  - `{ "type": "http", "url": ... }` (and url-only configs) use
 *    StreamableHTTPClientTransport, falling back to SSEClientTransport when the
 *    endpoint rejects streamable HTTP with a 4xx (e.g. 405 from a legacy
 *    2024-11-05 server) before the first successful response.
 *  - `{ "type": "sse", "url": ... }` selects the legacy HTTP+SSE transport
 *    directly.
 *
 * OAuth (MCP authorization spec) is handled by the SDK's `auth()` through a
 * file-backed provider (see mcp-oauth.ts): on 401 the SDK discovers the
 * authorization server, refreshes persisted tokens when possible, and
 * otherwise starts a browser-based authorization-code + PKCE flow. This module
 * waits on the loopback redirect, exchanges the code, and retries the request
 * — so an rpc issued before authorization completes succeeds once the user
 * finishes signing in (or times out at its own deadline while the background
 * flow keeps running and persists tokens for the next connect).
 *
 * Both Claude Code (`mcpServers`) and Copilot / VS Code (`servers`) mcp.json
 * shapes use these `type`/`url`/`headers` fields. The returned connection has
 * the same rpc/notify/terminate surface as the stdio spawn in mcp-tools.ts so
 * loaders can treat local and remote servers uniformly.
 */

import { auth, UnauthorizedError } from "@modelcontextprotocol/sdk/client/auth.js";
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import { StreamableHTTPClientTransport, StreamableHTTPError } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { Transport } from "@modelcontextprotocol/sdk/shared/transport.js";
import type { JSONRPCMessage } from "@modelcontextprotocol/sdk/types.js";
import { McpFileOAuthProvider } from "./mcp-oauth.js";

/** Transport-agnostic MCP connection surface (matches the stdio loaders'). */
export interface McpTransport {
	rpc(method: string, params?: unknown, timeoutMs?: number): Promise<unknown>;
	/** Send a JSON-RPC notification (no id, no response expected). */
	notify(method: string, params?: unknown): void;
	terminate(): void;
}

export interface McpHttpServerConfig {
	/** Unique server identifier (used in error messages and tool-name prefixes). */
	name: string;
	/** Remote server URL. */
	url: string;
	/** Extra HTTP headers (e.g. a static Authorization) sent on every request. */
	headers?: Record<string, string>;
	/** "http" = Streamable HTTP with SSE fallback (default); "sse" = legacy HTTP+SSE. */
	type?: "http" | "sse";
}

export interface McpRemoteOptions {
	/** Directory for persisted OAuth state (default `~/.hoocode/mcp-auth`). */
	authStorageDir?: string;
	/** Open the authorization URL (default: platform browser opener). */
	openBrowser?: (url: string) => void | Promise<void>;
	/** Max time to wait for the browser redirect (default 5 minutes). */
	authTimeoutMs?: number;
	/**
	 * Invoked when an interactive authorization flow starts. `completed`
	 * resolves once the code was exchanged and tokens persisted — callers can
	 * use it to retry the connection — and rejects on timeout/denial.
	 */
	onAuthRequired?: (authorizationUrl: string | undefined, completed: Promise<void>) => void;
}

const DEFAULT_AUTH_TIMEOUT_MS = 300_000;

/** 4xx responses (other than auth errors) mean "endpoint doesn't speak streamable HTTP". */
function isFallbackError(err: unknown): boolean {
	return (
		err instanceof StreamableHTTPError &&
		err.code !== undefined &&
		err.code >= 400 &&
		err.code < 500 &&
		err.code !== 401 &&
		err.code !== 403
	);
}

function toError(err: unknown): Error {
	return err instanceof Error ? err : new Error(String(err));
}

/**
 * Open a connection to a remote MCP server. `type: "sse"` selects the legacy
 * HTTP+SSE transport; anything else uses Streamable HTTP with automatic SSE
 * fallback. Connection setup is lazy — errors surface on the first rpc (the
 * initialize handshake), same as a stdio spawn.
 */
export function connectHttpMcpServer(config: McpHttpServerConfig, options: McpRemoteOptions = {}): McpTransport {
	let nextId = 1;
	const pending = new Map<number, { resolve: (r: unknown) => void; reject: (e: Error) => void }>();
	let kind: "http" | "sse" = config.type === "sse" ? "sse" : "http";
	let fellBack = false;
	let gotResponse = false;
	let closed = false;
	let transport: Transport | undefined;
	let starting: Promise<Transport> | undefined;
	let authInFlight: Promise<void> | undefined;

	const provider = new McpFileOAuthProvider(config.url, {
		storageDir: options.authStorageDir,
		openBrowser: options.openBrowser,
	});

	function routeMessage(msg: JSONRPCMessage): void {
		if (!("id" in msg) || msg.id === undefined || msg.id === null) return;
		if (!("result" in msg) && !("error" in msg)) return;
		// The initialize result carries the negotiated protocol version; the raw
		// transport doesn't track it itself (the SDK Client normally does), so
		// feed it back for the MCP-Protocol-Version request header.
		if ("result" in msg) {
			const negotiated = (msg.result as { protocolVersion?: string } | undefined)?.protocolVersion;
			if (negotiated && transport?.setProtocolVersion) transport.setProtocolVersion(negotiated);
		}
		gotResponse = true;
		const cb = pending.get(Number(msg.id));
		if (!cb) return;
		pending.delete(Number(msg.id));
		if ("error" in msg) cb.reject(new Error(msg.error.message));
		else cb.resolve(msg.result);
	}

	function makeTransport(): Promise<Transport> {
		if (!starting) {
			starting = (async () => {
				const url = new URL(config.url);
				const opts = {
					authProvider: provider,
					requestInit: config.headers ? { headers: config.headers } : undefined,
				};
				const t: Transport =
					kind === "sse" ? new SSEClientTransport(url, opts) : new StreamableHTTPClientTransport(url, opts);
				t.onmessage = routeMessage;
				t.onerror = () => {
					// Non-fatal transport notices (e.g. a server that 405s the optional
					// GET notification stream). Request failures reject via send().
				};
				t.onclose = () => {
					if (transport === t) transport = undefined;
				};
				await t.start();
				transport = t;
				return t;
			})();
			starting
				.catch(() => {})
				.finally(() => {
					starting = undefined;
				});
		}
		return starting;
	}

	async function discardTransport(): Promise<void> {
		const t = transport;
		transport = undefined;
		try {
			await t?.close();
		} catch {
			// already closed
		}
	}

	/**
	 * Complete a browser-based authorization flow the SDK just initiated (it
	 * opened the browser via the provider before throwing UnauthorizedError):
	 * wait for the loopback redirect, exchange the code, persist tokens.
	 * Deduplicated so concurrent sends share one flow.
	 */
	function completeInteractiveAuth(): Promise<void> {
		if (!authInFlight) {
			authInFlight = (async () => {
				const code = await provider.waitForAuthorizationCode(options.authTimeoutMs ?? DEFAULT_AUTH_TIMEOUT_MS);
				const result = await auth(provider, { serverUrl: config.url, authorizationCode: code });
				if (result !== "AUTHORIZED") throw new Error(`MCP server "${config.name}" authorization did not complete`);
			})();
			authInFlight
				.catch(() => {})
				.finally(() => {
					provider.closeCallbackServer();
					authInFlight = undefined;
				});
			options.onAuthRequired?.(provider.lastAuthorizationUrl, authInFlight);
		}
		return authInFlight;
	}

	async function transportSend(message: JSONRPCMessage): Promise<void> {
		let authRetries = 0;
		for (;;) {
			if (closed) throw new Error(`MCP server "${config.name}" connection terminated`);
			try {
				const t = transport ?? (await makeTransport());
				await t.send(message);
				return;
			} catch (err) {
				if (err instanceof UnauthorizedError && authRetries === 0) {
					authRetries++;
					await completeInteractiveAuth();
					// Fresh transport so both kinds re-handshake with the new tokens.
					await discardTransport();
					continue;
				}
				if (kind === "http" && !fellBack && !gotResponse && config.type !== "sse" && isFallbackError(err)) {
					fellBack = true;
					kind = "sse";
					await discardTransport();
					continue;
				}
				throw toError(err);
			}
		}
	}

	function rpc(method: string, params?: unknown, timeoutMs?: number): Promise<unknown> {
		const id = nextId++;
		return new Promise<unknown>((resolve, reject) => {
			let timer: NodeJS.Timeout | undefined;
			if (timeoutMs && timeoutMs > 0) {
				timer = setTimeout(() => {
					if (pending.delete(id)) {
						reject(new Error(`MCP server "${config.name}" timed out after ${timeoutMs}ms on ${method}`));
					}
				}, timeoutMs);
				timer.unref?.();
			}
			pending.set(id, {
				resolve: (r) => {
					if (timer) clearTimeout(timer);
					resolve(r);
				},
				reject: (e) => {
					if (timer) clearTimeout(timer);
					reject(e);
				},
			});
			transportSend({ jsonrpc: "2.0", id, method, params } as JSONRPCMessage).catch((err: unknown) => {
				const cb = pending.get(id);
				if (cb) {
					pending.delete(id);
					cb.reject(toError(err));
				}
			});
		});
	}

	function notify(method: string, params?: unknown): void {
		transportSend({ jsonrpc: "2.0", method, params } as JSONRPCMessage).catch(() => {
			// fire-and-forget
		});
	}

	function terminate(): void {
		closed = true;
		provider.closeCallbackServer();
		const t = transport;
		transport = undefined;
		void (async () => {
			try {
				// Streamable HTTP sessions should be ended explicitly (HTTP DELETE).
				const st = t as StreamableHTTPClientTransport | undefined;
				if (st && typeof st.terminateSession === "function") await st.terminateSession();
			} catch {
				// best-effort
			}
			try {
				await t?.close();
			} catch {
				// already closed
			}
		})();
		const err = new Error(`MCP server "${config.name}" connection terminated`);
		for (const cb of pending.values()) cb.reject(err);
		pending.clear();
	}

	return { rpc, notify, terminate };
}
