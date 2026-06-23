#!/usr/bin/env node
/**
 * CLI entry point for the refactored coding agent.
 * Uses main.ts with AgentSession and new mode modules.
 *
 * Test with: npx tsx src/cli-new.ts [args...]
 */
import { EnvHttpProxyAgent, setGlobalDispatcher } from "undici";
import { APP_NAME } from "./config.js";
import { main } from "./main.js";
import { configureGlobalTLS } from "./utils/tls-ca.js";

process.title = APP_NAME;
process.env.HOOCODE_CODING_AGENT = "true";
process.env.PI_CODING_AGENT = "true";
process.emitWarning = (() => {}) as typeof process.emitWarning;

// App-level TLS CA trust: install any custom/system CA additively on the global
// HTTPS agent (verification stays ON) and thread the same trust set into the
// undici dispatcher below. The two flags (--ca-cert / --use-system-ca) are read
// via a process.argv pre-scan inside configureGlobalTLS, because the dispatcher
// is built here before main() parses args.
const ca = configureGlobalTLS();

// bodyTimeout/headersTimeout default to 300s in undici; long local-LLM stalls
// (e.g. vLLM buffering a large tool call) exceed that and abort the SSE stream
// with UND_ERR_BODY_TIMEOUT. Disable both — provider SDKs enforce their own
// AbortController-based deadlines via retry.provider.timeoutMs.
setGlobalDispatcher(new EnvHttpProxyAgent({ bodyTimeout: 0, headersTimeout: 0, connect: { ca } }));

main(process.argv.slice(2));
