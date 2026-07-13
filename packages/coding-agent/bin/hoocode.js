#!/usr/bin/env node
import { fileURLToPath, pathToFileURL } from "url";
import { dirname, join } from "path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const distDir = join(__dirname, "..", "dist");

// Wrap path -> file:// URL so dynamic import() works on Windows.
// Node's ESM loader rejects raw "C:\\..." paths with ERR_UNSUPPORTED_ESM_URL_SCHEME.
const toUrl = (p) => pathToFileURL(p).href;

const { initConfig } = await import(toUrl(join(distDir, "init.js")));
await initConfig();

// No extensionFactories passed: main() defaults to the built-in hoo-core
// (DEFAULT_EXTENSION_FACTORIES), the single source of truth shared with the
// compiled binary entry (src/cli.ts). This avoids a second hoo-core reference
// here and any risk of double-registration.
const { main } = await import(toUrl(join(distDir, "main.js")));

await main(process.argv.slice(2));
