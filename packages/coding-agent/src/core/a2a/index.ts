/**
 * A2A (Agent2Agent) discovery for HooCode.
 *
 * Publishes an AgentCard describing the instance's active tools and skills so
 * other agents can discover what this HooCode can do. See ./types.ts for the
 * protocol shapes and ./agent-card.ts for how the card is assembled.
 */

export {
	type BuildAgentCardOptions,
	type BuiltinToolName,
	buildAgentCard,
	type DiscoverableSkill,
	type ResolveActiveToolsOptions,
	resolveActiveTools,
} from "./agent-card.js";
export {
	type A2ADiscoveryServer,
	type A2ADiscoveryServerOptions,
	handleRequest,
	startA2ADiscoveryServer,
} from "./server.js";
export {
	A2A_PROTOCOL_VERSION,
	AGENT_CARD_WELL_KNOWN_PATH,
	AGENT_CARD_WELL_KNOWN_PATH_ALIAS,
	type AgentCapabilities,
	type AgentCard,
	type AgentProvider,
	type AgentSkill,
	type SecurityScheme,
} from "./types.js";
