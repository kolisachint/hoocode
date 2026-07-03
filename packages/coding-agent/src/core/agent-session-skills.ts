/**
 * Skill-block parsing and inline skill-command expansion for AgentSession.
 *
 * `parseSkillBlock` recognizes the `<skill …>` envelope embedded in user
 * messages; `expandSkillCommand` turns a `/skill:name args` invocation into that
 * envelope by reading the skill file. Both are pure aside from the file read,
 * which reports failures through the supplied error callback.
 */

import { readFileSync } from "node:fs";
import { stripFrontmatter } from "../utils/frontmatter.js";
import type { Skill } from "./skills.js";

/** Parsed skill block from a user message */
export interface ParsedSkillBlock {
	name: string;
	location: string;
	content: string;
	userMessage: string | undefined;
}

/**
 * Parse a skill block from message text.
 * Returns null if the text doesn't contain a skill block.
 */
export function parseSkillBlock(text: string): ParsedSkillBlock | null {
	const match = text.match(/^<skill name="([^"]+)" location="([^"]+)">\n([\s\S]*?)\n<\/skill>(?:\n\n([\s\S]+))?$/);
	if (!match) return null;
	return {
		name: match[1],
		location: match[2],
		content: match[3],
		userMessage: match[4]?.trim() || undefined,
	};
}

/** Error reported while reading a skill file during expansion. */
export interface SkillExpansionError {
	filePath: string;
	error: string;
}

/**
 * Expand a skill command (`/skill:name args`) to its full skill-block content.
 * Returns the expanded text, or the original text when it is not a skill command
 * or the named skill is unknown. File-read failures are surfaced via `onError`
 * and leave the original text unchanged.
 */
export function expandSkillCommand(text: string, skills: Skill[], onError: (err: SkillExpansionError) => void): string {
	if (!text.startsWith("/skill:")) return text;

	const spaceIndex = text.indexOf(" ");
	const skillName = spaceIndex === -1 ? text.slice(7) : text.slice(7, spaceIndex);
	const args = spaceIndex === -1 ? "" : text.slice(spaceIndex + 1).trim();

	const skill = skills.find((s) => s.name === skillName);
	if (!skill) return text; // Unknown skill, pass through

	try {
		const content = readFileSync(skill.filePath, "utf-8");
		const body = stripFrontmatter(content).trim();
		const skillBlock = `<skill name="${skill.name}" location="${skill.filePath}">\nReferences are relative to ${skill.baseDir}.\n\n${body}\n</skill>`;
		return args ? `${skillBlock}\n\n${args}` : skillBlock;
	} catch (err) {
		onError({ filePath: skill.filePath, error: err instanceof Error ? err.message : String(err) });
		return text; // Return original on error
	}
}
