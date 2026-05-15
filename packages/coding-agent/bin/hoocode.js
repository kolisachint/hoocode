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

const { default: hooCore } = await import(toUrl(join(distDir, "extensions", "core", "hoo-core.js")));
const { main } = await import(toUrl(join(distDir, "main.js")));

await main(process.argv.slice(2), { extensionFactories: [hooCore] });
