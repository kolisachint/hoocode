#!/usr/bin/env node
import { APP_NAME } from "../config.js";

process.title = APP_NAME;
process.emitWarning = (() => {}) as typeof process.emitWarning;

import { restoreSandboxEnv } from "./restore-sandbox-env.js";

restoreSandboxEnv();

const { initConfig } = await import("../init.js");
await initConfig();
await import("../cli.js");
