/**
 * The Hoo mark, inlined.
 *
 * This is the only HooCode surface that renders in a browser, and it is the one
 * a user sees at the exact moment they are deciding whether to trust the thing
 * asking for their account. It showed the upstream project's mark until this
 * release, which on that page read as a different product than the one they
 * launched.
 *
 * Kept as a string rather than read from assets/symbol.svg because this module
 * is bundled into a standalone binary with no filesystem to read from, and a
 * missing logo on the auth page is not a failure anyone would debug.
 *
 * Mirrors assets/symbol.svg. Colours are literal rather than `currentColor`:
 * the page sets its own dark background, so the mark never has to adapt.
 */
const LOGO_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="16 60 168 80" aria-hidden="true"><path d="M40,72 A34,34 0 0 0 40,128" fill="none" stroke="#00F0FF" stroke-width="6" stroke-linecap="round"/><path d="M160,72 A34,34 0 0 1 160,128" fill="none" stroke="#00F0FF" stroke-width="6" stroke-linecap="round"/><circle cx="70" cy="100" r="30" fill="none" stroke="#FAFAFA" stroke-width="9"/><circle cx="130" cy="100" r="30" fill="none" stroke="#FAFAFA" stroke-width="9"/><polygon points="100,89 111,100 100,111 89,100" fill="#00F0FF"/></svg>`;

function escapeHtml(value: string): string {
	return value
		.replaceAll("&", "&amp;")
		.replaceAll("<", "&lt;")
		.replaceAll(">", "&gt;")
		.replaceAll('"', "&quot;")
		.replaceAll("'", "&#39;");
}

function renderPage(options: { title: string; heading: string; message: string; details?: string }): string {
	const title = escapeHtml(options.title);
	const heading = escapeHtml(options.heading);
	const message = escapeHtml(options.message);
	const details = options.details ? escapeHtml(options.details) : undefined;

	return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${title}</title>
  <style>
    :root {
      --text: #fafafa;
      --text-dim: #a1a1aa;
      --page-bg: #09090b;
      --font-sans: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, "Noto Sans", sans-serif, "Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji";
      --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
    }
    * { box-sizing: border-box; }
    html { color-scheme: dark; }
    body {
      margin: 0;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 24px;
      background: var(--page-bg);
      color: var(--text);
      font-family: var(--font-sans);
      text-align: center;
    }
    main {
      width: 100%;
      max-width: 560px;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
    }
    .logo {
      /* The mark is 168x80, so a square box would letterbox it. */
      width: 168px;
      height: 80px;
      display: block;
      margin-bottom: 20px;
    }
    .logo svg { width: 100%; height: 100%; }
    h1 {
      margin: 0 0 10px;
      font-size: 28px;
      line-height: 1.15;
      font-weight: 650;
      color: var(--text);
    }
    p {
      margin: 0;
      line-height: 1.7;
      color: var(--text-dim);
      font-size: 15px;
    }
    .details {
      margin-top: 16px;
      font-family: var(--font-mono);
      font-size: 13px;
      color: var(--text-dim);
      white-space: pre-wrap;
      word-break: break-word;
    }
  </style>
</head>
<body>
  <main>
    <div class="logo">${LOGO_SVG}</div>
    <h1>${heading}</h1>
    <p>${message}</p>
    ${details ? `<div class="details">${details}</div>` : ""}
  </main>
</body>
</html>`;
}

export function oauthSuccessHtml(message: string): string {
	return renderPage({
		title: "Authentication successful",
		heading: "Authentication successful",
		message,
	});
}

export function oauthErrorHtml(message: string, details?: string): string {
	return renderPage({
		title: "Authentication failed",
		heading: "Authentication failed",
		message,
		details,
	});
}
