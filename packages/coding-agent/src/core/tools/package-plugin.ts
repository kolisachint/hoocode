/**
 * PackagePlugin — the autonomous half of the publish lane (spec §3.2).
 *
 * Everything before the button: run the eval gates at publish strictness, write
 * a README if the plugin has none, produce the marketplace index entry, and hand
 * back the instructions for the step the model must not take. The tool touches no
 * remote and cannot publish; `/plugin publish` (a human command) is where the
 * artifact actually leaves the machine.
 *
 * It is autonomous because none of it is irreversible — it writes two files into
 * a directory the plugin already owns — and because the value of the lane is that
 * the human's remaining decision is *just* the approval, with the mechanics
 * already done and the checks already run. It is in
 * {@link PLUGIN_SYSTEM_TOOL_NAMES} all the same: an authored subagent that could
 * package is one step from a subagent that could publish.
 */

import { type Static, Type } from "typebox";
import { getPlugin } from "../extensions/plugins/authoring.js";
import { normalizePlatformToken } from "../extensions/plugins/formats/platform-targets.js";
import { entryNeedsSource, packagePlugin, resolvePackagePlatform } from "../extensions/plugins/packaging.js";
import { defineTool, type ToolDefinition } from "../extensions/types.js";
import { PACKAGE_PLUGIN_TOOL_NAME } from "./plugin-tool-names.js";

const packageParams = Type.Object(
	{
		id: Type.String({ description: "Id of the installed or authored plugin to package." }),
		platform: Type.Optional(
			Type.Union([Type.Literal("claude"), Type.Literal("github")], {
				description:
					"Target ecosystem. Omit to use the plugin's own — where it lives decides, then what it declares. " +
					"`agents` is not a choice here: it belongs to no marketplace, so nothing can be published for it.",
			}),
		),
	},
	{ additionalProperties: false },
);

interface PackagePluginDetails {
	id: string;
	packaged: boolean;
	/** Whether the gates were green — i.e. whether the plugin may be published at all. */
	ready: boolean;
	platform?: string;
}

export function createPackagePluginToolDefinition(): ToolDefinition {
	return defineTool<typeof packageParams, PackagePluginDetails>({
		name: PACKAGE_PLUGIN_TOOL_NAME,
		label: PACKAGE_PLUGIN_TOOL_NAME,
		description:
			"Prepare a local plugin for publication: run its checks at publish strictness (structural, safety, and a " +
			"behavioral smoke run of any hooks/MCP servers), generate a README if it has none, and produce the " +
			"marketplace.json entry. Writes only inside the plugin's own directory and contacts no remote — it cannot " +
			"publish. Use it to find out whether a plugin is publishable and to hand the user a ready-to-submit bundle; " +
			"they run /plugin publish to do the actual submission.",
		promptSnippet: "Package a plugin for publication (gates + README + index entry; never publishes).",
		promptGuidelines: [
			"Packaging is safe and autonomous — it only writes into the plugin's own directory. Publishing is not: never open a PR, push, or submit a plugin to a marketplace yourself, even when asked to 'just publish it'. Package it, show the result, and point at /plugin publish.",
			"Its checks are stricter than authoring's, so a plugin that authored fine can still fail here. Report what failed and offer to fix it rather than presenting a red result as done.",
		],
		parameters: packageParams,
		async execute(_id, params: Static<typeof packageParams>, _signal, _onUpdate, ctx) {
			const fail = (text: string) => ({
				content: [{ type: "text" as const, text }],
				details: { id: params.id, packaged: false, ready: false },
			});

			const plugin = getPlugin(ctx.cwd, params.id);
			if (!plugin) return fail(`No plugin named "${params.id}" is installed or authored here.`);

			// The schema already excludes `agents`, so this only folds the aliases
			// (`copilot`, `gh`) a model might reasonably reach for.
			const requested = params.platform ? normalizePlatformToken(params.platform) : undefined;
			const platform = requested === "claude" || requested === "github" ? requested : resolvePackagePlatform(plugin);

			const result = await packagePlugin(plugin, platform);
			const text = [
				`Packaged "${params.id}" for ${result.platform} in ${result.dir}`,
				`Wrote: ${result.written.join(", ")}`,
				"",
				result.instructions,
			].join("\n");
			ctx.ui.notify(
				result.ok
					? `Packaged "${params.id}" for ${result.platform} — ready to publish with /plugin publish ${params.id}.`
					: `Packaged "${params.id}" for ${result.platform}, but its checks failed — not publishable yet.`,
				result.ok ? "info" : "warning",
			);
			return {
				content: [
					{
						type: "text" as const,
						text:
							result.ok && entryNeedsSource(result.entry)
								? `${text}\n\nThe entry's \`source\` is a placeholder — the user must say where the plugin will be hosted.`
								: text,
					},
				],
				details: { id: params.id, packaged: true, ready: result.ok, platform: result.platform },
			};
		},
	});
}
