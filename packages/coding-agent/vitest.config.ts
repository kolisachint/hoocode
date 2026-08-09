import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

const aiSrcIndex = fileURLToPath(new URL("../ai/src/index.ts", import.meta.url));
const aiSrcOAuth = fileURLToPath(new URL("../ai/src/oauth.ts", import.meta.url));
const agentSrcIndex = fileURLToPath(new URL("../agent/src/index.ts", import.meta.url));
const tuiSrcIndex = fileURLToPath(new URL("../tui/src/index.ts", import.meta.url));

export default defineConfig({
	test: {
		globals: true,
		environment: "node",
		testTimeout: 30000,
		// Plugin locations are user-scoped, so an un-isolated test writes real
		// plugins into the developer's ~/.claude and ~/.agents. See the setup file.
		setupFiles: ["./test/setup/isolate-home.ts"],
		server: {
			deps: {
				external: [/@silvia-odwyer\/photon-node/],
			},
		},
	},
	resolve: {
		alias: [
			{ find: /^@kolisachint\/hoocode-ai$/, replacement: aiSrcIndex },
			{ find: /^@kolisachint\/hoocode-ai\/oauth$/, replacement: aiSrcOAuth },
			{ find: /^@kolisachint\/hoocode-agent-core$/, replacement: agentSrcIndex },
			// Resolve to source like the siblings above: without this, any suite that
			// reaches the extension loader fails to load unless packages/tui is built.
			{ find: /^@kolisachint\/hoocode-tui$/, replacement: tuiSrcIndex },
		],
	},
});
