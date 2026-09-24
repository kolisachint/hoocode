/**
 * Context a canvas attaches to the person's next message — the SDK's
 * `sendAttachmentsToMessage`, whose `extension_context` entries the host shows as
 * pills and sends along with what the person types.
 *
 * It answers the question a person at two surfaces keeps running into: they
 * select something on the canvas and type "make this blue" in the terminal, and
 * "this" means nothing to the agent. With the selection attached, it does. The
 * person sees the pill before sending, so nothing reaches the model unseen, and
 * it goes once: sending the message spends it.
 *
 * Rendered as upstream renders it into the prompt, an `<extension_context>`
 * block per attachment.
 */

import type { CanvasAttachment, JsonValue } from "./protocol.js";

/** Largest payload carried per attachment, serialized; more is cut and says so. */
export const CANVAS_ATTACHMENT_MAX_CHARS = 4_000;
/** Most attachments one extension may have pending. */
export const CANVAS_ATTACHMENTS_PER_EXTENSION = 4;

/** One pending attachment, with where it came from. */
export interface PendingCanvasAttachment extends CanvasAttachment {
	extensionId: string;
	instanceId: string | undefined;
}

function escapeAttribute(text: string): string {
	return text.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function serialize(payload: JsonValue): string {
	const text = typeof payload === "string" ? payload : JSON.stringify(payload);
	if (text === undefined) return "";
	return text.length <= CANVAS_ATTACHMENT_MAX_CHARS
		? text
		: `${text.slice(0, CANVAS_ATTACHMENT_MAX_CHARS)}… [cut at ${CANVAS_ATTACHMENT_MAX_CHARS} characters]`;
}

export class CanvasAttachments {
	private readonly pending = new Map<string, PendingCanvasAttachment[]>();

	/** An extension's attachments, replacing what it pushed before. Empty withdraws them. */
	set(extensionId: string, attachments: CanvasAttachment[], instanceId?: string): void {
		const items = attachments
			.filter((item) => item.title.trim().length > 0)
			.slice(0, CANVAS_ATTACHMENTS_PER_EXTENSION)
			.map((item) => ({ extensionId, instanceId, title: item.title.trim().slice(0, 200), payload: item.payload }));
		if (items.length === 0) this.pending.delete(extensionId);
		else this.pending.set(extensionId, items);
	}

	/** Everything pending, for the pill band. */
	list(): PendingCanvasAttachment[] {
		return [...this.pending.values()].flat();
	}

	/** Drop everything an extension attached, as when it closes. */
	clear(extensionId?: string): void {
		if (extensionId === undefined) this.pending.clear();
		else this.pending.delete(extensionId);
	}

	/**
	 * The person is sending a message: return the blocks to append to it, and
	 * forget them. Undefined when nothing is pending.
	 */
	take(): string | undefined {
		const items = this.list();
		if (items.length === 0) return undefined;
		this.pending.clear();
		return items
			.map((item) => {
				const instance = item.instanceId ? ` instance="${escapeAttribute(item.instanceId)}"` : "";
				return `<extension_context source="${escapeAttribute(item.extensionId)}"${instance} title="${escapeAttribute(item.title)}">\n${serialize(item.payload)}\n</extension_context>`;
			})
			.join("\n");
	}
}
