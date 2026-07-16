/**
 * Helpers for constraining tool-call argument decoding.
 *
 * OpenAI-style strict function calling ("structured outputs") guarantees the
 * model can only emit arguments matching the tool's JSON schema, but it
 * requires a "closed" schema: every property listed in `required`,
 * `additionalProperties: false` on every object, and only a subset of JSON
 * Schema keywords. TypeBox-generated tool schemas are open (optionals,
 * validation keywords), so they must be transformed before `strict: true` can
 * be sent — otherwise the API rejects the request.
 */

const STRICT_KEYWORD_ALLOWLIST = new Set([
	"$defs",
	"$ref",
	"additionalProperties",
	"anyOf",
	"const",
	"definitions",
	"description",
	"enum",
	"items",
	"properties",
	"required",
	"title",
	"type",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Optional properties cannot exist in strict mode (everything is required),
 * so a formerly-optional property is expressed as "this type or null".
 */
function makeNullable(schema: unknown): unknown {
	if (!isRecord(schema)) {
		return schema;
	}
	if (typeof schema.type === "string") {
		return schema.type === "null" ? schema : { ...schema, type: [schema.type, "null"] };
	}
	if (Array.isArray(schema.type)) {
		return schema.type.includes("null") ? schema : { ...schema, type: [...schema.type, "null"] };
	}
	if (Array.isArray(schema.enum)) {
		return schema.enum.includes(null) ? schema : { ...schema, enum: [...schema.enum, null] };
	}
	if (Array.isArray(schema.anyOf)) {
		const hasNull = schema.anyOf.some((variant) => isRecord(variant) && variant.type === "null");
		return hasNull ? schema : { ...schema, anyOf: [...schema.anyOf, { type: "null" }] };
	}
	return { anyOf: [schema, { type: "null" }] };
}

function transformNode(node: unknown): unknown {
	if (!isRecord(node)) {
		return node;
	}

	// Keep only keywords strict mode accepts; validation-only keywords
	// (minLength, pattern, format, default, ...) get the request rejected.
	const out: Record<string, unknown> = {};
	for (const [key, value] of Object.entries(node)) {
		if (STRICT_KEYWORD_ALLOWLIST.has(key)) {
			out[key] = value;
		}
	}

	if (isRecord(out.properties)) {
		const originallyRequired = new Set(
			Array.isArray(out.required) ? out.required.filter((name) => typeof name === "string") : [],
		);
		const properties: Record<string, unknown> = {};
		for (const [name, propertySchema] of Object.entries(out.properties)) {
			const transformed = transformNode(propertySchema);
			properties[name] = originallyRequired.has(name) ? transformed : makeNullable(transformed);
		}
		out.properties = properties;
		out.required = Object.keys(properties);
		out.additionalProperties = false;
	} else if (out.type === "object") {
		out.properties = {};
		out.required = [];
		out.additionalProperties = false;
	}

	if (out.items !== undefined) {
		out.items = Array.isArray(out.items) ? out.items.map(transformNode) : transformNode(out.items);
	}
	if (Array.isArray(out.anyOf)) {
		out.anyOf = out.anyOf.map(transformNode);
	}
	for (const defsKey of ["$defs", "definitions"] as const) {
		const defs = out[defsKey];
		if (isRecord(defs)) {
			out[defsKey] = Object.fromEntries(Object.entries(defs).map(([name, def]) => [name, transformNode(def)]));
		}
	}

	return out;
}

/**
 * Transform a JSON schema into the closed form OpenAI strict function calling
 * accepts: all properties required (formerly-optional ones become nullable),
 * `additionalProperties: false` on every object, and unsupported keywords
 * stripped. The input is deep-cloned via JSON round-trip, which also drops
 * TypeBox's symbol keys.
 */
export function toStrictJsonSchema(schema: unknown): Record<string, unknown> {
	const cloned: unknown = JSON.parse(JSON.stringify(schema ?? {}));
	const transformed = transformNode(cloned);
	return isRecord(transformed) ? transformed : {};
}
