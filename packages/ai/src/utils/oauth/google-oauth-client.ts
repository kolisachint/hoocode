/**
 * Google's desktop OAuth clients are not shipped with hoocode.
 *
 * Gemini CLI and Antigravity each authenticate with their own installed-app
 * client id and secret. Those values are public in the sense RFC 8252 §8.5
 * means it — an app the user runs cannot keep a secret, which is why the flow
 * is PKCE-protected rather than secret-protected — but they are still someone
 * else's credentials, and a repository is the wrong place for them. So hoocode
 * reads them from the environment and ships nothing.
 */

export interface GoogleOAuthClient {
	clientId: string;
	clientSecret: string;
}

export interface GoogleOAuthClientEnv {
	/** Environment variable holding the client id. */
	idVar: string;
	/** Environment variable holding the client secret. */
	secretVar: string;
	/** Provider name, as the user sees it in `/login`. */
	productName: string;
}

/**
 * Read a provider's OAuth client from the environment.
 *
 * Throws when either half is missing, naming both variables — this runs at
 * login and on refresh, so the message is the only instruction the user gets.
 */
export function readGoogleOAuthClient({ idVar, secretVar, productName }: GoogleOAuthClientEnv): GoogleOAuthClient {
	const clientId = process.env[idVar]?.trim();
	const clientSecret = process.env[secretVar]?.trim();

	if (!clientId || !clientSecret) {
		const missing = !clientId && !clientSecret ? `${idVar} and ${secretVar}` : !clientId ? idVar : secretVar;
		throw new Error(
			`${productName} needs an OAuth client, and hoocode does not ship one. Set ${missing} to the ` +
				`installed-app credentials of the client you are signing in as, then run /login again. ` +
				`See docs/providers.md for where those values come from.`,
		);
	}

	return { clientId, clientSecret };
}
