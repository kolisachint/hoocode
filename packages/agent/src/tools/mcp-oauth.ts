/**
 * OAuth support for remote MCP servers: a file-backed
 * {@link OAuthClientProvider} for the official MCP SDK plus a loopback
 * callback server for the browser-based authorization-code + PKCE flow.
 *
 * The SDK's `auth()` orchestrates discovery (RFC 9728 / RFC 8414), dynamic
 * client registration, PKCE, token exchange, and refresh; this module supplies
 * the persistence and user-interaction pieces:
 *  - per-server-URL state (client registration, tokens, code verifier) stored
 *    as 0600 JSON files under `~/.hoocode/mcp-auth/` by default;
 *  - a localhost HTTP server that receives the authorization redirect. It is
 *    started lazily inside `clientInformation()` — the first provider call of
 *    every interactive `auth()` run that happens before the authorization URL
 *    is built — so no listener exists unless an OAuth flow is actually running.
 *    Re-binding the port used at registration keeps persisted dynamic client
 *    registrations valid; if the port is taken, the registration is dropped so
 *    the SDK re-registers with the fresh redirect URI.
 */

import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import type { OAuthClientProvider } from "@modelcontextprotocol/sdk/client/auth.js";
import type {
	OAuthClientInformationMixed,
	OAuthClientMetadata,
	OAuthTokens,
} from "@modelcontextprotocol/sdk/shared/auth.js";

/** Default directory for persisted MCP OAuth state. */
export function defaultMcpAuthDir(): string {
	return join(homedir(), ".hoocode", "mcp-auth");
}

/** Open a URL in the user's browser (best-effort, detached). */
export function openBrowserDefault(url: string): void {
	const [cmd, args] =
		process.platform === "darwin"
			? ["open", [url]]
			: process.platform === "win32"
				? ["cmd", ["/c", "start", "", url]]
				: ["xdg-open", [url]];
	try {
		spawn(cmd, args as string[], { detached: true, stdio: "ignore" }).unref();
	} catch {
		// best-effort — the authorization URL is also surfaced via onAuthorizationUrl
	}
}

interface PersistedAuthState {
	serverUrl: string;
	clientInformation?: OAuthClientInformationMixed;
	/** Redirect URI the dynamic client registration was performed with. */
	redirectUrl?: string;
	tokens?: OAuthTokens;
	codeVerifier?: string;
}

const CALLBACK_PATH = "/callback";

interface CallbackWaiter {
	resolve: (code: string) => void;
	reject: (err: Error) => void;
	expectedState?: string;
}

/** Loopback HTTP server that receives the OAuth authorization redirect. */
class OAuthCallbackServer {
	private server: Server;
	private waiter?: CallbackWaiter;
	/** Redirect that arrived before anyone called waitForCode (fast redirects). */
	private buffered?: { code?: string; state?: string; error?: string };
	readonly url: string;

	private constructor(server: Server, port: number) {
		this.server = server;
		this.url = `http://127.0.0.1:${port}${CALLBACK_PATH}`;
	}

	/** Bind the callback server; `preferredPort` first, any free port otherwise. */
	static async start(preferredPort?: number): Promise<{ server: OAuthCallbackServer; reboundPort: boolean }> {
		const listen = (port: number) =>
			new Promise<Server>((resolve, reject) => {
				const srv = createServer();
				srv.once("error", reject);
				srv.listen(port, "127.0.0.1", () => {
					srv.removeAllListeners("error");
					resolve(srv);
				});
			});

		let server: Server;
		let reboundPort = true;
		if (preferredPort) {
			try {
				server = await listen(preferredPort);
			} catch {
				server = await listen(0);
				reboundPort = false;
			}
		} else {
			server = await listen(0);
			reboundPort = false;
		}

		const cb = new OAuthCallbackServer(server, (server.address() as AddressInfo).port);
		server.on("request", (req, res) => {
			const url = new URL(req.url ?? "/", cb.url);
			if (url.pathname !== CALLBACK_PATH) {
				res.writeHead(404).end();
				return;
			}
			const code = url.searchParams.get("code") ?? undefined;
			const state = url.searchParams.get("state") ?? undefined;
			const error = url.searchParams.get("error") ?? undefined;
			if (error || !code) {
				res.writeHead(400, { "content-type": "text/html" }).end("<h3>Authorization failed.</h3>");
			} else {
				res.writeHead(200, { "content-type": "text/html" }).end(
					"<h3>Authorization complete.</h3><p>You can return to hoocode.</p>",
				);
			}
			const waiter = cb.waiter;
			if (waiter) {
				cb.waiter = undefined;
				cb.deliver(waiter, { code, state, error });
			} else {
				// Redirect can land before the client starts waiting (instant
				// redirects, test drivers); hold it for the next waitForCode.
				cb.buffered = { code, state, error };
			}
		});
		return { server: cb, reboundPort };
	}

	private deliver(waiter: CallbackWaiter, result: { code?: string; state?: string; error?: string }): void {
		if (result.error || !result.code) {
			waiter.reject(new Error(`OAuth authorization failed: ${result.error ?? "no code returned"}`));
		} else if (waiter.expectedState && result.state !== waiter.expectedState) {
			waiter.reject(new Error("OAuth authorization failed: state mismatch"));
		} else {
			waiter.resolve(result.code);
		}
	}

	waitForCode(expectedState: string | undefined, timeoutMs: number): Promise<string> {
		return new Promise<string>((resolve, reject) => {
			const timer = setTimeout(() => {
				this.waiter = undefined;
				reject(new Error(`Timed out waiting ${timeoutMs}ms for OAuth authorization`));
			}, timeoutMs);
			timer.unref?.();
			const waiter: CallbackWaiter = {
				expectedState,
				resolve: (code) => {
					clearTimeout(timer);
					resolve(code);
				},
				reject: (err) => {
					clearTimeout(timer);
					reject(err);
				},
			};
			const buffered = this.buffered;
			if (buffered) {
				this.buffered = undefined;
				this.deliver(waiter, buffered);
				return;
			}
			this.waiter = waiter;
		});
	}

	close(): void {
		this.waiter?.reject(new Error("OAuth callback server closed"));
		this.waiter = undefined;
		this.server.close();
		// Sever keep-alive connections so close() doesn't linger.
		this.server.closeAllConnections?.();
	}
}

export interface McpOAuthProviderOptions {
	/** Directory for persisted auth state (default `~/.hoocode/mcp-auth`). */
	storageDir?: string;
	/** Open the authorization URL (default: spawn the platform browser opener). */
	openBrowser?: (url: string) => void | Promise<void>;
}

/**
 * File-backed OAuth provider for one remote MCP server, keyed by server URL.
 * Implements the SDK's {@link OAuthClientProvider} so `auth()` handles the
 * protocol; this class owns persistence, the loopback redirect listener, and
 * opening the user's browser.
 */
export class McpFileOAuthProvider implements OAuthClientProvider {
	private readonly serverUrl: string;
	private readonly statePath: string;
	private readonly openBrowser: (url: string) => void | Promise<void>;
	private callback?: OAuthCallbackServer;
	private callbackStarting?: Promise<OAuthCallbackServer>;
	private currentState?: string;
	/** Last authorization URL handed to `redirectToAuthorization`. */
	lastAuthorizationUrl?: string;

	constructor(serverUrl: string, options: McpOAuthProviderOptions = {}) {
		this.serverUrl = serverUrl;
		this.openBrowser = options.openBrowser ?? openBrowserDefault;
		const dir = options.storageDir ?? defaultMcpAuthDir();
		const hash = createHash("sha256").update(serverUrl).digest("hex").slice(0, 12);
		const host = (URL.canParse(serverUrl) ? new URL(serverUrl).hostname : "server").replace(/[^a-zA-Z0-9.-]/g, "_");
		this.statePath = join(dir, `${host}-${hash}.json`);
	}

	private read(): PersistedAuthState {
		try {
			return JSON.parse(readFileSync(this.statePath, "utf8")) as PersistedAuthState;
		} catch {
			return { serverUrl: this.serverUrl };
		}
	}

	private write(mutate: (state: PersistedAuthState) => void): void {
		const state = this.read();
		mutate(state);
		mkdirSync(dirname(this.statePath), { recursive: true });
		writeFileSync(this.statePath, JSON.stringify(state, null, 2), { mode: 0o600 });
	}

	/**
	 * Ensure the loopback redirect listener is running. Prefers the port used at
	 * dynamic client registration so the persisted client stays valid; when that
	 * port can't be re-bound the registration is dropped and the SDK registers a
	 * fresh client against the new redirect URI.
	 */
	async ensureCallbackServer(): Promise<OAuthCallbackServer> {
		if (this.callback) return this.callback;
		if (!this.callbackStarting) {
			this.callbackStarting = (async () => {
				const persisted = this.read();
				const preferredPort =
					persisted.redirectUrl && URL.canParse(persisted.redirectUrl)
						? Number(new URL(persisted.redirectUrl).port) || undefined
						: undefined;
				const { server, reboundPort } = await OAuthCallbackServer.start(preferredPort);
				if (preferredPort && !reboundPort) {
					this.write((s) => {
						s.clientInformation = undefined;
						s.redirectUrl = undefined;
					});
				}
				this.callback = server;
				return server;
			})();
			this.callbackStarting.catch(() => {
				this.callbackStarting = undefined;
			});
		}
		return this.callbackStarting;
	}

	/** Wait for the browser redirect to deliver an authorization code. */
	async waitForAuthorizationCode(timeoutMs: number): Promise<string> {
		const server = await this.ensureCallbackServer();
		return server.waitForCode(this.currentState, timeoutMs);
	}

	/** Stop the loopback listener (auth finished, failed, or connection closed). */
	closeCallbackServer(): void {
		this.callback?.close();
		this.callback = undefined;
		this.callbackStarting = undefined;
	}

	// ---- OAuthClientProvider ----

	get redirectUrl(): string | undefined {
		// Must stay defined even before the listener is up: an undefined
		// redirectUrl tells the SDK to run a non-interactive grant instead of the
		// authorization-code flow. The real port is bound in clientInformation()
		// before the SDK builds the authorization URL.
		return this.callback?.url ?? this.read().redirectUrl ?? "http://127.0.0.1:0/callback";
	}

	get clientMetadata(): OAuthClientMetadata {
		return {
			client_name: "hoocode",
			redirect_uris: [String(this.redirectUrl)],
			grant_types: ["authorization_code", "refresh_token"],
			response_types: ["code"],
			token_endpoint_auth_method: "none",
		};
	}

	state(): string {
		this.currentState ??= randomBytes(16).toString("hex");
		return this.currentState;
	}

	async clientInformation(): Promise<OAuthClientInformationMixed | undefined> {
		// First provider call of every auth() run that precedes building the
		// authorization URL — bind the redirect listener here so redirectUrl is
		// live for registration and the authorization request.
		await this.ensureCallbackServer();
		return this.read().clientInformation;
	}

	saveClientInformation(clientInformation: OAuthClientInformationMixed): void {
		const redirectUrl = String(this.redirectUrl);
		this.write((s) => {
			s.clientInformation = clientInformation;
			s.redirectUrl = redirectUrl;
		});
	}

	tokens(): OAuthTokens | undefined {
		return this.read().tokens;
	}

	saveTokens(tokens: OAuthTokens): void {
		this.write((s) => {
			s.tokens = tokens;
		});
	}

	async redirectToAuthorization(authorizationUrl: URL): Promise<void> {
		this.lastAuthorizationUrl = authorizationUrl.toString();
		await this.openBrowser(this.lastAuthorizationUrl);
	}

	saveCodeVerifier(codeVerifier: string): void {
		this.write((s) => {
			s.codeVerifier = codeVerifier;
		});
	}

	codeVerifier(): string {
		const verifier = this.read().codeVerifier;
		if (!verifier) throw new Error(`No PKCE code verifier stored for ${this.serverUrl}`);
		return verifier;
	}

	invalidateCredentials(scope: "all" | "client" | "tokens" | "verifier" | "discovery"): void {
		if (scope === "discovery") return;
		if (scope === "all" && existsSync(this.statePath)) {
			try {
				unlinkSync(this.statePath);
			} catch {
				// fall through to field-level clearing
			}
			return;
		}
		this.write((s) => {
			if (scope === "all" || scope === "client") {
				s.clientInformation = undefined;
				s.redirectUrl = undefined;
			}
			if (scope === "all" || scope === "tokens") s.tokens = undefined;
			if (scope === "all" || scope === "verifier") s.codeVerifier = undefined;
		});
	}
}
