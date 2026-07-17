/**
 * TypeScript definitions for the subset of the Agent2Agent (A2A) protocol used
 * by HooCode's discovery layer.
 *
 * HooCode only implements the *discovery* half of A2A: it publishes an
 * {@link AgentCard} at `/.well-known/agent.json` so other agents (or a
 * HooTeams orchestrator) can learn which skills this instance exposes. The
 * task-execution half of the protocol (JSON-RPC `message/send`, streaming,
 * push notifications) is not implemented here — the card advertises the
 * capabilities honestly as `false`.
 *
 * Shapes follow the A2A specification's AgentCard object. See
 * https://a2a-protocol.org for the full schema.
 */

/** Well-known path where an AgentCard is published, per the A2A spec. */
export const AGENT_CARD_WELL_KNOWN_PATH = "/.well-known/agent.json";

/**
 * Newer alias for the well-known path introduced by later revisions of the
 * spec. HooCode serves both so clients on either revision can discover it.
 */
export const AGENT_CARD_WELL_KNOWN_PATH_ALIAS = "/.well-known/agent-card.json";

/** A2A protocol revision this card conforms to. */
export const A2A_PROTOCOL_VERSION = "0.2.5";

/**
 * Optional-feature flags an agent advertises. HooCode's discovery-only server
 * does not stream tasks or deliver push notifications, so these default to
 * `false`.
 */
export interface AgentCapabilities {
	streaming: boolean;
	pushNotifications: boolean;
	stateTransitionHistory: boolean;
}

/** Identifies the organization operating the agent. */
export interface AgentProvider {
	organization: string;
	url: string;
}

/**
 * A single capability the agent advertises. Maps to an A2A `AgentSkill`.
 *
 * `id` is a stable machine identifier; `tags` group related skills so a client
 * can filter (e.g. all `code` skills). `examples` are natural-language prompts
 * the skill can handle, useful for routing.
 */
export interface AgentSkill {
	id: string;
	name: string;
	description: string;
	tags: string[];
	examples?: string[];
	inputModes?: string[];
	outputModes?: string[];
}

/** A named HTTP security scheme referenced by {@link AgentCard.security}. */
export interface SecurityScheme {
	type: string;
	scheme?: string;
	description?: string;
}

/**
 * The public metadata document an A2A agent publishes so peers can discover it.
 */
export interface AgentCard {
	/** A2A revision this document conforms to. */
	protocolVersion: string;
	/** Human-readable agent name. */
	name: string;
	/** Human-readable summary of what the agent does. */
	description: string;
	/** Base URL where the agent's A2A service is reachable. */
	url: string;
	/** Agent (not protocol) version — HooCode's package version. */
	version: string;
	provider?: AgentProvider;
	capabilities: AgentCapabilities;
	/** Content types the agent accepts when a skill does not override them. */
	defaultInputModes: string[];
	/** Content types the agent emits when a skill does not override them. */
	defaultOutputModes: string[];
	/** Named security schemes a client may authenticate with. */
	securitySchemes?: Record<string, SecurityScheme>;
	/** Which schemes (by name) are required, per the A2A `security` array. */
	security?: Array<Record<string, string[]>>;
	/** Discoverable capabilities offered by this agent. */
	skills: AgentSkill[];
	/** Optional URL to the agent's documentation. */
	documentationUrl?: string;
}
