/**
 * Module resolution for a forked canvas extension.
 *
 * Design: `docs/canvas-extensions-design.md` §4.1. A canvas extension's only
 * external import is `@github/copilot-sdk/extension`, and GitHub's own
 * `docs/extensions.md` says that import "is resolved automatically — you don't
 * install it". So hoocode has to make the specifier resolvable inside the child
 * without writing anything into the extension directory: a catalog canvas must
 * stay byte-identical to upstream (tier 1 of the design's ownership split).
 *
 * The mechanism is a Node module-resolution hook (`node:module`'s `register`)
 * delivered entirely through `--import` **data: URLs**. Nothing is written to
 * disk and no `.mjs` asset has to be copied into `dist/` or embedded in the
 * packaged binary — the whole resolver is two generated modules that exist only
 * as command-line arguments.
 */

/** The specifier a canvas extension imports, and the package it belongs to. */
export const CANVAS_SDK_PACKAGE = "@github/copilot-sdk";

/**
 * Resolution hooks, as source. `initialize` receives the shim URL through
 * `register`'s `data` channel rather than the environment, because the hooks run
 * on a separate loader thread and the documented `data` path is the supported way
 * to parameterise them.
 */
const HOOKS_SOURCE = `
let shimUrl;
export function initialize(data) {
	shimUrl = data.shimUrl;
}
export async function resolve(specifier, context, next) {
	if (specifier === ${JSON.stringify(CANVAS_SDK_PACKAGE)} || specifier.startsWith(${JSON.stringify(`${CANVAS_SDK_PACKAGE}/`)})) {
		return { url: shimUrl, shortCircuit: true };
	}
	return next(specifier, context);
}
`;

function dataModule(source: string): string {
	return `data:text/javascript,${encodeURIComponent(source)}`;
}

/**
 * Build the `--import` argument that makes `@github/copilot-sdk` resolve to
 * `shimUrl` inside the child.
 *
 * @param shimUrl Absolute `file:` URL of the module implementing the SDK surface.
 */
export function canvasResolverImportArg(shimUrl: string): string {
	if (!shimUrl.startsWith("file://")) {
		throw new Error(`Canvas shim URL must be an absolute file: URL, received "${shimUrl}".`);
	}
	const hooksUrl = dataModule(HOOKS_SOURCE);
	const registerSource = `
import { register } from "node:module";
register(${JSON.stringify(hooksUrl)}, { parentURL: import.meta.url, data: ${JSON.stringify({ shimUrl })} });
`;
	return dataModule(registerSource);
}
