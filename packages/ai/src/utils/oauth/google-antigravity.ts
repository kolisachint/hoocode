/**
 * Antigravity OAuth flow (Gemini 3, Claude, GPT-OSS via Google Cloud)
 * Uses different OAuth credentials than google-gemini-cli for access to additional models.
 *
 * NOTE: This module uses Node.js http.createServer for the OAuth callback.
 * It is only intended for CLI use, not browser environments.
 */

import type { Server } from "node:http";
import { type GoogleOAuthClientEnv, readGoogleOAuthClient } from "./google-oauth-client.js";
import { oauthErrorHtml, oauthSuccessHtml } from "./oauth-page.js";
import { generatePKCE } from "./pkce.js";
import type { OAuthCredentials, OAuthLoginCallbacks, OAuthProviderInterface } from "./types.js";

type AntigravityCredentials = OAuthCredentials & {
	projectId: string;
};

let _createServer: typeof import("node:http").createServer | null = null;
let _httpImportPromise: Promise<void> | null = null;
if (typeof process !== "undefined" && (process.versions?.node || process.versions?.bun)) {
	_httpImportPromise = import("node:http").then((m) => {
		_createServer = m.createServer;
	});
}

const CALLBACK_HOST = process.env.HOOCODE_OAUTH_CALLBACK_HOST || "127.0.0.1";
const CALLBACK_PORT = 51121;

// Which client signs in decides which tiers Google serves: the Antigravity
// client reaches the consumer free tier that the Gemini CLI client is refused
// with UNSUPPORTED_CLIENT. Google's token endpoint also demands a
// client_secret, so this cannot run as a bare public PKCE client the way the
// Anthropic and OpenAI logins do. hoocode ships no credentials; see
// google-oauth-client.ts.
const CLIENT_ENV: GoogleOAuthClientEnv = {
	idVar: "HOOCODE_ANTIGRAVITY_CLIENT_ID",
	secretVar: "HOOCODE_ANTIGRAVITY_CLIENT_SECRET",
	productName: "Google Antigravity",
};
const REDIRECT_URI = "http://localhost:51121/oauth-callback";

// Antigravity requires additional scopes
const SCOPES = [
	"https://www.googleapis.com/auth/cloud-platform",
	"https://www.googleapis.com/auth/userinfo.email",
	"https://www.googleapis.com/auth/userinfo.profile",
	"https://www.googleapis.com/auth/cclog",
	"https://www.googleapis.com/auth/experimentsandconfigs",
];

const AUTH_URL = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL = "https://oauth2.googleapis.com/token";

const CODE_ASSIST_ENDPOINT = "https://cloudcode-pa.googleapis.com";

// Google's managed project for consumer Antigravity accounts, as handed back by
// `onboardUser` for a free-tier sign-in. Used when neither the account nor the
// environment names one: eligibility calls answer `{}` or 403 for stretches even
// on accounts that stream fine, so a discovery hiccup must not fail the login.
const MANAGED_CONSUMER_PROJECT = "aicode-consumers";

type CallbackServerInfo = {
	server: Server;
	cancelWait: () => void;
	waitForCode: () => Promise<{ code: string; state: string } | null>;
};

/**
 * Start a local HTTP server to receive the OAuth callback
 */
async function getNodeCreateServer(): Promise<typeof import("node:http").createServer> {
	if (_createServer) return _createServer;
	if (_httpImportPromise) {
		await _httpImportPromise;
	}
	if (_createServer) return _createServer;
	throw new Error("Antigravity OAuth is only available in Node.js environments");
}

async function startCallbackServer(): Promise<CallbackServerInfo> {
	const createServer = await getNodeCreateServer();

	return new Promise((resolve, reject) => {
		let settleWait: ((value: { code: string; state: string } | null) => void) | undefined;
		const waitForCodePromise = new Promise<{ code: string; state: string } | null>((resolveWait) => {
			let settled = false;
			settleWait = (value) => {
				if (settled) return;
				settled = true;
				resolveWait(value);
			};
		});

		const server = createServer((req, res) => {
			const url = new URL(req.url || "", `http://localhost:${CALLBACK_PORT}`);

			if (url.pathname === "/oauth-callback") {
				const code = url.searchParams.get("code");
				const state = url.searchParams.get("state");
				const error = url.searchParams.get("error");

				if (error) {
					res.writeHead(400, { "Content-Type": "text/html; charset=utf-8" });
					res.end(oauthErrorHtml("Google authentication did not complete.", `Error: ${error}`));
					return;
				}

				if (code && state) {
					res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
					res.end(oauthSuccessHtml("Google authentication completed. You can close this window."));
					settleWait?.({ code, state });
				} else {
					res.writeHead(400, { "Content-Type": "text/html; charset=utf-8" });
					res.end(oauthErrorHtml("Missing code or state parameter."));
				}
			} else {
				res.writeHead(404, { "Content-Type": "text/html; charset=utf-8" });
				res.end(oauthErrorHtml("Callback route not found."));
			}
		});

		server.on("error", (err) => {
			reject(err);
		});

		server.listen(CALLBACK_PORT, CALLBACK_HOST, () => {
			resolve({
				server,
				cancelWait: () => {
					settleWait?.(null);
				},
				waitForCode: () => waitForCodePromise,
			});
		});
	});
}

/**
 * Parse redirect URL to extract code and state
 */
function parseRedirectUrl(input: string): { code?: string; state?: string } {
	const value = input.trim();
	if (!value) return {};

	try {
		const url = new URL(value);
		return {
			code: url.searchParams.get("code") ?? undefined,
			state: url.searchParams.get("state") ?? undefined,
		};
	} catch {
		// Not a URL, return empty
		return {};
	}
}

interface LoadCodeAssistPayload {
	cloudaicompanionProject?: string | { id?: string };
	currentTier?: { id?: string };
	allowedTiers?: Array<{ id?: string; isDefault?: boolean }>;
}

/** Long-running operation returned by onboardUser. */
interface OnboardUserOperation {
	name?: string;
	done?: boolean;
	response?: { cloudaicompanionProject?: string | { id?: string } };
}

const ANTIGRAVITY_METADATA = {
	ideType: "ANTIGRAVITY",
	platform: "PLATFORM_UNSPECIFIED",
	pluginType: "GEMINI",
};

function readProjectId(value: string | { id?: string } | undefined): string | undefined {
	if (typeof value === "string") return value || undefined;
	return value?.id || undefined;
}

function wait(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Discover or provision the Cloud project every Antigravity request is billed to.
 *
 * A free-tier account has no project of its own: `loadCodeAssist` answers with a
 * tier and no `cloudaicompanionProject`, and it is `onboardUser` that hands back
 * Google's managed project. Both calls are best effort — they have been observed
 * answering `{}` and `403 not eligible` minutes apart on an account whose
 * requests stream fine — so every step falls through to the next rather than
 * failing the login.
 */
async function discoverProject(accessToken: string, onProgress?: (message: string) => void): Promise<string> {
	const headers = {
		Authorization: `Bearer ${accessToken}`,
		"Content-Type": "application/json",
		"User-Agent": "google-api-nodejs-client/9.15.1",
		"X-Goog-Api-Client": "google-cloud-sdk vscode_cloudshelleditor/0.1",
		"Client-Metadata": JSON.stringify(ANTIGRAVITY_METADATA),
	};
	const envProjectId = process.env.GOOGLE_CLOUD_PROJECT || process.env.GOOGLE_CLOUD_PROJECT_ID;

	onProgress?.("Checking for an existing project...");
	let tierId: string | undefined;
	try {
		const loadResponse = await fetch(`${CODE_ASSIST_ENDPOINT}/v1internal:loadCodeAssist`, {
			method: "POST",
			headers,
			body: JSON.stringify({ metadata: ANTIGRAVITY_METADATA }),
		});
		if (loadResponse.ok) {
			const data = (await loadResponse.json()) as LoadCodeAssistPayload;
			const existing = readProjectId(data.cloudaicompanionProject);
			if (existing) {
				return existing;
			}
			tierId =
				data.currentTier?.id ?? data.allowedTiers?.find((tier) => tier.isDefault)?.id ?? data.allowedTiers?.[0]?.id;
		}
	} catch {
		// Fall through to onboarding.
	}

	if (tierId) {
		try {
			onProgress?.("Provisioning the Antigravity project (this may take a moment)...");
			const onboardBody: Record<string, unknown> = { tierId, metadata: ANTIGRAVITY_METADATA };
			if (envProjectId) {
				onboardBody.cloudaicompanionProject = envProjectId;
			}

			const onboardResponse = await fetch(`${CODE_ASSIST_ENDPOINT}/v1internal:onboardUser`, {
				method: "POST",
				headers,
				body: JSON.stringify(onboardBody),
			});

			if (onboardResponse.ok) {
				let operation = (await onboardResponse.json()) as OnboardUserOperation;
				for (let attempt = 0; !operation.done && operation.name && attempt < 12; attempt++) {
					await wait(5000);
					onProgress?.(`Waiting for project provisioning (attempt ${attempt + 2})...`);
					const poll = await fetch(`${CODE_ASSIST_ENDPOINT}/v1internal/${operation.name}`, { headers });
					if (!poll.ok) break;
					operation = (await poll.json()) as OnboardUserOperation;
				}
				const provisioned = readProjectId(operation.response?.cloudaicompanionProject);
				if (provisioned) {
					return provisioned;
				}
			}
		} catch {
			// Fall through to the environment / managed project.
		}
	}

	if (envProjectId) {
		return envProjectId;
	}

	onProgress?.("Using Google's managed Antigravity project...");
	return MANAGED_CONSUMER_PROJECT;
}

/**
 * Get user email from the access token
 */
async function getUserEmail(accessToken: string): Promise<string | undefined> {
	try {
		const response = await fetch("https://www.googleapis.com/oauth2/v1/userinfo?alt=json", {
			headers: {
				Authorization: `Bearer ${accessToken}`,
			},
		});

		if (response.ok) {
			const data = (await response.json()) as { email?: string };
			return data.email;
		}
	} catch {
		// Ignore errors, email is optional
	}
	return undefined;
}

/**
 * Refresh Antigravity token
 */
export async function refreshAntigravityToken(refreshToken: string, projectId: string): Promise<OAuthCredentials> {
	const { clientId, clientSecret } = readGoogleOAuthClient(CLIENT_ENV);
	const response = await fetch(TOKEN_URL, {
		method: "POST",
		headers: { "Content-Type": "application/x-www-form-urlencoded" },
		body: new URLSearchParams({
			client_id: clientId,
			client_secret: clientSecret,
			refresh_token: refreshToken,
			grant_type: "refresh_token",
		}),
	});

	if (!response.ok) {
		const error = await response.text();
		throw new Error(`Antigravity token refresh failed: ${error}`);
	}

	const data = (await response.json()) as {
		access_token: string;
		expires_in: number;
		refresh_token?: string;
	};

	return {
		refresh: data.refresh_token || refreshToken,
		access: data.access_token,
		expires: Date.now() + data.expires_in * 1000 - 5 * 60 * 1000,
		projectId,
	};
}

/**
 * Login with Antigravity OAuth
 *
 * @param onAuth - Callback with URL and optional instructions
 * @param onProgress - Optional progress callback
 * @param onManualCodeInput - Optional promise that resolves with user-pasted redirect URL.
 *                            Races with browser callback - whichever completes first wins.
 */
export async function loginAntigravity(
	onAuth: (info: { url: string; instructions?: string }) => void,
	onProgress?: (message: string) => void,
	onManualCodeInput?: () => Promise<string>,
): Promise<OAuthCredentials> {
	const { verifier, challenge } = await generatePKCE();

	// Read before the browser opens: a missing client should fail here, not after
	// the user has signed in.
	const { clientId, clientSecret } = readGoogleOAuthClient(CLIENT_ENV);

	// Start local server for callback
	onProgress?.("Starting local server for OAuth callback...");
	const server = await startCallbackServer();

	let code: string | undefined;

	try {
		// Build authorization URL
		const authParams = new URLSearchParams({
			client_id: clientId,
			response_type: "code",
			redirect_uri: REDIRECT_URI,
			scope: SCOPES.join(" "),
			code_challenge: challenge,
			code_challenge_method: "S256",
			state: verifier,
			access_type: "offline",
			prompt: "consent",
		});

		const authUrl = `${AUTH_URL}?${authParams.toString()}`;

		// Notify caller with URL to open
		onAuth({
			url: authUrl,
			instructions: "Complete the sign-in in your browser.",
		});

		// Wait for the callback, racing with manual input if provided
		onProgress?.("Waiting for OAuth callback...");

		if (onManualCodeInput) {
			// Race between browser callback and manual input
			let manualInput: string | undefined;
			let manualError: Error | undefined;
			const manualPromise = onManualCodeInput()
				.then((input) => {
					manualInput = input;
					server.cancelWait();
				})
				.catch((err) => {
					manualError = err instanceof Error ? err : new Error(String(err));
					server.cancelWait();
				});

			const result = await server.waitForCode();

			// If manual input was cancelled, throw that error
			if (manualError) {
				throw manualError;
			}

			if (result?.code) {
				// Browser callback won - verify state
				if (result.state !== verifier) {
					throw new Error("OAuth state mismatch - possible CSRF attack");
				}
				code = result.code;
			} else if (manualInput) {
				// Manual input won
				const parsed = parseRedirectUrl(manualInput);
				if (parsed.state && parsed.state !== verifier) {
					throw new Error("OAuth state mismatch - possible CSRF attack");
				}
				code = parsed.code;
			}

			// If still no code, wait for manual promise and try that
			if (!code) {
				await manualPromise;
				if (manualError) {
					throw manualError;
				}
				if (manualInput) {
					const parsed = parseRedirectUrl(manualInput);
					if (parsed.state && parsed.state !== verifier) {
						throw new Error("OAuth state mismatch - possible CSRF attack");
					}
					code = parsed.code;
				}
			}
		} else {
			// Original flow: just wait for callback
			const result = await server.waitForCode();
			if (result?.code) {
				if (result.state !== verifier) {
					throw new Error("OAuth state mismatch - possible CSRF attack");
				}
				code = result.code;
			}
		}

		if (!code) {
			throw new Error("No authorization code received");
		}

		// Exchange code for tokens
		onProgress?.("Exchanging authorization code for tokens...");
		const tokenResponse = await fetch(TOKEN_URL, {
			method: "POST",
			headers: {
				"Content-Type": "application/x-www-form-urlencoded",
			},
			body: new URLSearchParams({
				client_id: clientId,
				client_secret: clientSecret,
				code,
				grant_type: "authorization_code",
				redirect_uri: REDIRECT_URI,
				code_verifier: verifier,
			}),
		});

		if (!tokenResponse.ok) {
			const error = await tokenResponse.text();
			throw new Error(`Token exchange failed: ${error}`);
		}

		const tokenData = (await tokenResponse.json()) as {
			access_token: string;
			refresh_token: string;
			expires_in: number;
		};

		if (!tokenData.refresh_token) {
			throw new Error("No refresh token received. Please try again.");
		}

		// Get user email
		onProgress?.("Getting user info...");
		const email = await getUserEmail(tokenData.access_token);

		// Discover project
		const projectId = await discoverProject(tokenData.access_token, onProgress);

		// Calculate expiry time (current time + expires_in seconds - 5 min buffer)
		const expiresAt = Date.now() + tokenData.expires_in * 1000 - 5 * 60 * 1000;

		const credentials: OAuthCredentials = {
			refresh: tokenData.refresh_token,
			access: tokenData.access_token,
			expires: expiresAt,
			projectId,
			email,
		};

		return credentials;
	} finally {
		server.server.close();
	}
}

export const antigravityOAuthProvider: OAuthProviderInterface = {
	id: "google-antigravity",
	name: "Google Antigravity (Gemini, Claude, GPT-OSS)",
	usesCallbackServer: true,

	async login(callbacks: OAuthLoginCallbacks): Promise<OAuthCredentials> {
		return loginAntigravity(callbacks.onAuth, callbacks.onProgress, callbacks.onManualCodeInput);
	},

	async refreshToken(credentials: OAuthCredentials): Promise<OAuthCredentials> {
		const creds = credentials as AntigravityCredentials;
		if (!creds.projectId) {
			throw new Error("Antigravity credentials missing projectId");
		}
		return refreshAntigravityToken(creds.refresh, creds.projectId);
	},

	getApiKey(credentials: OAuthCredentials): string {
		const creds = credentials as AntigravityCredentials;
		return JSON.stringify({ token: creds.access, projectId: creds.projectId });
	},
};
