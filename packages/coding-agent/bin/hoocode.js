#!/usr/bin/env node
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const distDir = join(__dirname, "..", "dist");

const { initConfig } = await import(join(distDir, "init.js"));
await initConfig();

const { default: hooCore } = await import(join(distDir, "extensions", "core", "hoo-core.js"));
const { main } = await import(join(distDir, "main.js"));

await main(process.argv.slice(2), { extensionFactories: [hooCore] });
