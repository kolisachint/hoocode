/**
 * HTTP transports for remote MCP servers.
 *
 * Implements both remote transports from the MCP spec:
 *  - Streamable HTTP (`{ "type": "http", "url": ... }`, spec 2025-03-26+):
 *    every JSON-RPC message is POSTed to the server URL; the response body is
 *    either a plain JSON response or a short SSE stream carrying it. The
 *    `Mcp-Session-Id` response header, when present, is echoed on subsequent
 *    requests, and the negotiated protocol version is sent as
 *    `MCP-Protocol-Version` after initialize.
 *  - Legacy HTTP+SSE (`{ "type": "sse", "url": ... }`, spec 2024-11-05): a
 *    long-lived GET stream delivers an `endpoint` event naming the POST URL;
 *    requests are POSTed there and responses arrive on the stream.
 *
 * Both Claude Code (`mcpServers`) and Copilot / VS Code (`servers`) mcp.json
 * shapes use these `type`/`url`/`headers` fields. The returned connection has
 * the same rpc/notify/terminate surface as the stdio spawn in mcp-tools.ts so
 * loaders can treat local and remote servers uniformly.
 */

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
	/** Extra HTTP headers (e.g. Authorization) sent on every request. */
	headers?: Record<string, string>;
	/** "http" = Streamable HTTP (default); "sse" = legacy HTTP+SSE transport. */
	type?: "http" | "sse";
}

interface JsonRpcMessage {
	id?: number | string;
	result?: unknown;
	error?: { message: string };
}

/**
 * Incrementally parse an SSE byte stream, invoking `onEvent` for each complete
 * event that carries data. Return true from `onEvent` to stop reading; the
 * stream is cancelled on exit either way.
 */
async function readSseStream(
	body: ReadableStream<Uint8Array>,
	onEvent: (event: string, data: string) => boolean | undefined,
): Promise<void> {
	const reader = body.getReader();
	const decoder = new TextDecoder();
	let buffer = "";

	const dispatch = (rawEvent: string): boolean => {
		let event = "message";
		const data: string[] = [];
		for (const line of rawEvent.split(/\r?\n/)) {
			if (line.startsWith("event:")) event = line.slice(6).trim();
			else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
		}
		if (data.length === 0) return false;
		return onEvent(event, data.join("\n")) === true;
	};

	try {
		for (;;) {
			const { done, value } = await reader.read();
			if (done) break;
			buffer += decoder.decode(value, { stream: true });
			// Events are separated by a blank line.
			let sep = buffer.match(/\r?\n\r?\n/);
			while (sep && sep.index !== undefined) {
				const rawEvent = buffer.slice(0, sep.index);
				buffer = buffer.slice(sep.index + sep[0].length);
				if (dispatch(rawEvent)) return;
				sep = buffer.match(/\r?\n\r?\n/);
			}
		}
		if (buffer.trim()) dispatch(buffer);
	} finally {
		try {
			await reader.cancel();
		} catch {
			// stream already closed
		}
	}
}

function toError(err: unknown): Error {
	return err instanceof Error ? err : new Error(String(err));
}

/** Drain a response body we don't care about without leaking the stream. */
function drain(res: Response): void {
	res.body?.cancel().catch(() => {});
}

function connectStreamableHttp(config: McpHttpServerConfig): McpTransport {
	let nextId = 1;
	let sessionId: string | undefined;
	let protocolVersion: string | undefined;
	const abort = new AbortController();

	function buildHeaders(): Record<string, string> {
		const headers: Record<string, string> = {
			...(config.headers ?? {}),
			"content-type": "application/json",
			accept: "application/json, text/event-stream",
		};
		if (sessionId) headers["mcp-session-id"] = sessionId;
		if (protocolVersion) headers["mcp-protocol-version"] = protocolVersion;
		return headers;
	}

	function rpc(method: string, params?: unknown, timeoutMs?: number): Promise<unknown> {
		const id = nextId++;
		const request = (async (): Promise<unknown> => {
			const res = await fetch(config.url, {
				method: "POST",
				headers: buildHeaders(),
				body: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
				signal: abort.signal,
			});
			const sid = res.headers.get("mcp-session-id");
			if (sid) sessionId = sid;
			if (!res.ok) {
				const detail = await res.text().catch(() => "");
				throw new Error(
					`MCP server "${config.name}" returned HTTP ${res.status} for ${method}` +
						(detail ? `: ${detail.slice(0, 200)}` : ""),
				);
			}
			let msg: JsonRpcMessage | undefined;
			if ((res.headers.get("content-type") ?? "").includes("text/event-stream")) {
				if (!res.body) throw new Error(`MCP server "${config.name}" returned an empty SSE body for ${method}`);
				await readSseStream(res.body, (_event, data) => {
					try {
						const parsed = JSON.parse(data) as JsonRpcMessage;
						if (parsed.id === id) {
							msg = parsed;
							return true;
						}
					} catch {
						// ignore non-JSON events
					}
					return false;
				});
			} else {
				msg = (await res.json()) as JsonRpcMessage;
			}
			if (!msg) throw new Error(`MCP server "${config.name}" closed the stream without responding to ${method}`);
			if (msg.error) throw new Error(msg.error.message);
			if (method === "initialize") {
				const negotiated = (msg.result as { protocolVersion?: string } | undefined)?.protocolVersion;
				if (negotiated) protocolVersion = negotiated;
			}
			return msg.result;
		})();
		if (!timeoutMs || timeoutMs <= 0) return request;
		return new Promise<unknown>((resolve, reject) => {
			const timer = setTimeout(
				() => reject(new Error(`MCP server "${config.name}" timed out after ${timeoutMs}ms on ${method}`)),
				timeoutMs,
			);
			timer.unref?.();
			request.then(
				(r) => {
					clearTimeout(timer);
					resolve(r);
				},
				(e) => {
					clearTimeout(timer);
					reject(toError(e));
				},
			);
		});
	}

	function notify(method: string, params?: unknown): void {
		fetch(config.url, {
			method: "POST",
			headers: buildHeaders(),
			body: JSON.stringify({ jsonrpc: "2.0", method, params }),
			signal: abort.signal,
		})
			.then(drain)
			.catch(() => {
				// fire-and-forget
			});
	}

	function terminate(): void {
		// Per spec the client SHOULD end the session explicitly; best-effort,
		// issued before abort so it isn't cancelled with the in-flight requests.
		if (sessionId) {
			fetch(config.url, {
				method: "DELETE",
				headers: { ...(config.headers ?? {}), "mcp-session-id": sessionId },
			})
				.then(drain)
				.catch(() => {});
		}
		abort.abort();
	}

	return { rpc, notify, terminate };
}

function connectLegacySse(config: McpHttpServerConfig): McpTransport {
	let nextId = 1;
	const abort = new AbortController();
	const pending = new Map<number, { resolve: (r: unknown) => void; reject: (e: Error) => void }>();

	let resolveEndpoint!: (url: string) => void;
	let rejectEndpoint!: (e: Error) => void;
	const endpointPromise = new Promise<string>((resolve, reject) => {
		resolveEndpoint = resolve;
		rejectEndpoint = reject;
	});
	// rpc() awaits this with its own error handling; avoid an unhandled
	// rejection when the stream fails before anyone is waiting.
	endpointPromise.catch(() => {});

	function failAll(err: Error): void {
		for (const cb of pending.values()) cb.reject(err);
		pending.clear();
	}

	(async () => {
		const res = await fetch(config.url, {
			headers: { ...(config.headers ?? {}), accept: "text/event-stream" },
			signal: abort.signal,
		});
		if (!res.ok || !res.body) {
			drain(res);
			throw new Error(`MCP server "${config.name}" SSE connect failed with HTTP ${res.status}`);
		}
		await readSseStream(res.body, (event, data) => {
			if (event === "endpoint") {
				resolveEndpoint(new URL(data, config.url).toString());
				return false;
			}
			try {
				const msg = JSON.parse(data) as JsonRpcMessage;
				if (typeof msg.id !== "number") return false;
				const cb = pending.get(msg.id);
				if (!cb) return false;
				pending.delete(msg.id);
				if (msg.error) cb.reject(new Error(msg.error.message));
				else cb.resolve(msg.result);
			} catch {
				// ignore non-JSON events
			}
			return false;
		});
		throw new Error(`MCP server "${config.name}" closed the SSE stream`);
	})().catch((err: unknown) => {
		const e = toError(err);
		rejectEndpoint(e);
		failAll(e);
	});

	async function post(message: unknown): Promise<void> {
		const endpoint = await endpointPromise;
		const res = await fetch(endpoint, {
			method: "POST",
			headers: { ...(config.headers ?? {}), "content-type": "application/json" },
			body: JSON.stringify(message),
			signal: abort.signal,
		});
		drain(res);
		if (!res.ok) {
			throw new Error(`MCP server "${config.name}" returned HTTP ${res.status} on POST`);
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
			post({ jsonrpc: "2.0", id, method, params }).catch((err: unknown) => {
				const cb = pending.get(id);
				if (cb) {
					pending.delete(id);
					cb.reject(toError(err));
				}
			});
		});
	}

	function notify(method: string, params?: unknown): void {
		post({ jsonrpc: "2.0", method, params }).catch(() => {
			// fire-and-forget
		});
	}

	function terminate(): void {
		abort.abort();
		const err = new Error(`MCP server "${config.name}" connection terminated`);
		rejectEndpoint(err);
		failAll(err);
	}

	return { rpc, notify, terminate };
}

/**
 * Open a connection to a remote MCP server. `type: "sse"` selects the legacy
 * HTTP+SSE transport; anything else (including unset) uses Streamable HTTP.
 * Connection setup is lazy — errors surface on the first rpc (the initialize
 * handshake), same as a stdio spawn.
 */
export function connectHttpMcpServer(config: McpHttpServerConfig): McpTransport {
	return config.type === "sse" ? connectLegacySse(config) : connectStreamableHttp(config);
}
