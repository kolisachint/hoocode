import {
	Box,
	type Component,
	Container,
	getCapabilities,
	Image,
	isImageLine,
	Spacer,
	Text,
	type TUI,
	truncateToWidth,
	visibleWidth,
} from "@kolisachint/hoocode-tui";
import type { ToolDefinition, ToolRenderContext } from "../../../core/extensions/types.js";
import { createAllToolDefinitions, type ToolName } from "../../../core/tools/index.js";
import { getTextOutput as getRenderedTextOutput } from "../../../core/tools/render-utils.js";
import { convertToPng } from "../../../utils/image-convert.js";
import { theme } from "../theme/theme.js";

export interface ToolExecutionOptions {
	showImages?: boolean;
	imageWidthCells?: number;
}

/**
 * Renders a child component and prepends a prefix (e.g. a status dot) to its
 * first line, indenting continuation lines so they align under the content.
 *
 * This keeps the dot inline with the first line instead of stacking it on its
 * own line (which happens when a Container holds the dot as a separate child).
 */
class PrefixFirstLine implements Component {
	private prefix: string;
	private child: Component;
	private indentWidth: number;

	constructor(prefix: string, child: Component) {
		this.prefix = prefix;
		this.child = child;
		this.indentWidth = visibleWidth(prefix);
	}

	invalidate(): void {
		this.child.invalidate?.();
	}

	render(width: number): string[] {
		// Reserve room for the prefix so the child wraps within the remaining width.
		const childWidth = Math.max(1, width - this.indentWidth);
		const lines = this.child.render(childWidth);
		if (lines.length === 0) {
			return [this.prefix];
		}
		const indent = " ".repeat(this.indentWidth);
		return lines.map((line, i) => (i === 0 ? this.prefix + line : indent + line));
	}
}

export class ToolExecutionComponent extends Container {
	private contentBox: Box;
	private contentText: Text;
	private selfRenderContainer: Container;
	private callRendererComponent?: Component;
	private resultRendererComponent?: Component;
	private rendererState: any = {};
	private imageComponents: Image[] = [];
	private imageSpacers: Spacer[] = [];
	private toolName: string;
	private toolCallId: string;
	private args: any;
	private expanded = false;
	private showImages: boolean;
	private imageWidthCells: number;
	private isPartial = true;
	private toolDefinition?: ToolDefinition<any, any>;
	private builtInToolDefinition?: ToolDefinition<any, any>;
	private ui: TUI;
	private cwd: string;
	private executionStarted = false;
	private argsComplete = false;
	private result?: {
		content: Array<{ type: string; text?: string; data?: string; mimeType?: string }>;
		isError: boolean;
		details?: any;
	};
	private convertedImages: Map<number, { data: string; mimeType: string }> = new Map();
	private hideComponent = false;
	// Once a finished tool block has scrolled far out of view the interactive mode
	// freezes it (see freeze()): its already-rendered lines are captured and the
	// heavy source payloads (full tool result, base64 image copies, per-renderer
	// state, child component caches) are released. A frozen block is immutable and
	// no longer reflows on resize — old off-screen scrollback keeps its wrapping,
	// which self-heals on any full rebuild (theme toggle / session reload, both of
	// which reconstruct components from the intact session data).
	private frozen = false;
	private frozenLines?: string[];
	/** Width the frozen snapshot was captured at; a wider terminal is safe as-is,
	 * a narrower one needs re-truncation to avoid the TUI's over-width crash guard. */
	private frozenWidth = 0;
	private frozenTruncated?: string[];
	private frozenTruncatedWidth = -1;

	constructor(
		toolName: string,
		toolCallId: string,
		args: any,
		options: ToolExecutionOptions = {},
		toolDefinition: ToolDefinition<any, any> | undefined,
		ui: TUI,
		cwd: string,
	) {
		super();
		this.toolName = toolName;
		this.toolCallId = toolCallId;
		this.args = args;
		this.toolDefinition = toolDefinition;
		this.builtInToolDefinition = createAllToolDefinitions(cwd)[toolName as ToolName];
		this.showImages = options.showImages ?? true;
		this.imageWidthCells = options.imageWidthCells ?? 60;
		this.ui = ui;
		this.cwd = cwd;

		this.addChild(new Spacer(1));

		// Always create all shell variants. contentBox is used for default renderer-based composition.
		// selfRenderContainer is used when the tool renders its own framing.
		// contentText is reserved for generic fallback rendering when no tool definition exists.
		// Boxless: no filled background — hierarchy comes from status dots + indent.
		// paddingY is 0: the single leading Spacer(1) is the only separator between
		// tool blocks, so consecutive commands don't stack 3 blank lines between them.
		this.contentBox = new Box(1, 0);
		this.contentText = new Text("", 1, 0);
		this.selfRenderContainer = new Container();

		if (this.hasRendererDefinition()) {
			this.addChild(this.getRenderShell() === "self" ? this.selfRenderContainer : this.contentBox);
		} else {
			this.addChild(this.contentText);
		}

		this.updateDisplay();
	}

	private getCallRenderer(): ToolDefinition<any, any>["renderCall"] | undefined {
		if (!this.builtInToolDefinition) {
			return this.toolDefinition?.renderCall;
		}
		if (!this.toolDefinition) {
			return this.builtInToolDefinition.renderCall;
		}
		return this.toolDefinition.renderCall ?? this.builtInToolDefinition.renderCall;
	}

	private getResultRenderer(): ToolDefinition<any, any>["renderResult"] | undefined {
		if (!this.builtInToolDefinition) {
			return this.toolDefinition?.renderResult;
		}
		if (!this.toolDefinition) {
			return this.builtInToolDefinition.renderResult;
		}
		return this.toolDefinition.renderResult ?? this.builtInToolDefinition.renderResult;
	}

	private hasRendererDefinition(): boolean {
		return this.builtInToolDefinition !== undefined || this.toolDefinition !== undefined;
	}

	private getRenderShell(): "default" | "self" {
		if (!this.builtInToolDefinition) {
			return this.toolDefinition?.renderShell ?? "default";
		}
		if (!this.toolDefinition) {
			return this.builtInToolDefinition.renderShell ?? "default";
		}
		return this.toolDefinition.renderShell ?? this.builtInToolDefinition.renderShell ?? "default";
	}

	private getRenderContext(lastComponent: Component | undefined): ToolRenderContext {
		return {
			args: this.args,
			toolCallId: this.toolCallId,
			invalidate: () => {
				this.invalidate();
				this.ui.requestRender();
			},
			lastComponent,
			state: this.rendererState,
			cwd: this.cwd,
			executionStarted: this.executionStarted,
			argsComplete: this.argsComplete,
			isPartial: this.isPartial,
			expanded: this.expanded,
			showImages: this.showImages,
			isError: this.result?.isError ?? false,
		};
	}

	private createCallFallback(): Component {
		return new Text(theme.fg("toolTitle", theme.bold(this.toolName)), 0, 0);
	}

	private createResultFallback(): Component | undefined {
		const output = this.getTextOutput();
		if (!output) {
			return undefined;
		}
		return new Text(theme.fg("toolOutput", output), 0, 0);
	}

	updateArgs(args: any): void {
		this.args = args;
		this.updateDisplay();
	}

	markExecutionStarted(): void {
		this.executionStarted = true;
		this.updateDisplay();
		this.ui.requestRender();
	}

	setArgsComplete(): void {
		this.argsComplete = true;
		this.updateDisplay();
		this.ui.requestRender();
	}

	updateResult(
		result: {
			content: Array<{ type: string; text?: string; data?: string; mimeType?: string }>;
			details?: any;
			isError: boolean;
		},
		isPartial = false,
	): void {
		this.result = result;
		this.isPartial = isPartial;
		this.updateDisplay();
		this.maybeConvertImagesForKitty();
	}

	private maybeConvertImagesForKitty(): void {
		const caps = getCapabilities();
		if (caps.images !== "kitty") return;
		if (!this.result) return;

		const imageBlocks = this.result.content.filter((c) => c.type === "image");
		for (let i = 0; i < imageBlocks.length; i++) {
			const img = imageBlocks[i];
			if (!img.data || !img.mimeType) continue;
			if (img.mimeType === "image/png") continue;
			if (this.convertedImages.has(i)) continue;

			const index = i;
			convertToPng(img.data, img.mimeType).then((converted) => {
				if (this.frozen) return;
				if (converted) {
					this.convertedImages.set(index, converted);
					this.updateDisplay();
					this.ui.requestRender();
				}
			});
		}
	}

	setExpanded(expanded: boolean): void {
		this.expanded = expanded;
		this.updateDisplay();
	}

	setShowImages(show: boolean): void {
		this.showImages = show;
		this.updateDisplay();
	}

	setImageWidthCells(width: number): void {
		this.imageWidthCells = Math.max(1, Math.floor(width));
		this.updateDisplay();
	}

	override invalidate(): void {
		// A frozen block has released the state updateDisplay would rebuild from;
		// its captured snapshot is authoritative, so skip the rebuild entirely.
		if (this.frozen) return;
		super.invalidate();
		this.updateDisplay();
	}

	/**
	 * True when this block is a finished, visible tool result that still holds its
	 * heavy source payloads — i.e. a candidate for freeze() once it scrolls out of
	 * the live window. In-flight (partial) and already-frozen blocks are excluded.
	 */
	isFreezable(): boolean {
		return !this.frozen && !this.isPartial && !this.hideComponent && !!this.result;
	}

	/** Mark this block for freezing; the snapshot is captured on the next render
	 * (at the real render width) and the heavy state released then. */
	freeze(): void {
		if (this.isFreezable()) this.frozen = true;
	}

	private releaseHeavyState(): void {
		this.result = undefined;
		this.convertedImages.clear();
		this.imageComponents = [];
		this.imageSpacers = [];
		this.rendererState = {};
		this.callRendererComponent = undefined;
		this.resultRendererComponent = undefined;
		// Drop child components so their line caches (which embed base64 image
		// payloads and full text output) become collectable.
		this.clear();
	}

	override render(width: number): string[] {
		if (this.hideComponent) {
			return [];
		}
		if (this.frozenLines) {
			// Captured snapshot. Wider (or equal) terminal: emit as-is. Narrower:
			// re-truncate non-image lines to avoid the TUI's over-width crash guard,
			// caching the result per width so it stays O(1) on unchanged frames.
			if (width >= this.frozenWidth) return this.frozenLines;
			if (this.frozenTruncatedWidth === width && this.frozenTruncated) return this.frozenTruncated;
			this.frozenTruncated = this.frozenLines.map((line) =>
				isImageLine(line) || visibleWidth(line) <= width ? line : truncateToWidth(line, width),
			);
			this.frozenTruncatedWidth = width;
			return this.frozenTruncated;
		}
		const lines = super.render(width);
		if (this.frozen) {
			this.frozenLines = lines;
			this.frozenWidth = width;
			this.releaseHeavyState();
		}
		return lines;
	}

	private updateDisplay(): void {
		// Frozen blocks are immutable snapshots with their source state released.
		if (this.frozen) return;
		let hasContent = false;
		this.hideComponent = false;
		if (this.hasRendererDefinition()) {
			const renderContainer = this.getRenderShell() === "self" ? this.selfRenderContainer : this.contentBox;
			// Boxless: no background fill on the container.
			if (renderContainer instanceof Box) {
				renderContainer.setBgFn(undefined);
			}
			renderContainer.clear();

			// Status dot prefix: green for complete success, yellow for pending/partial, red for error.
			// The dot is prepended to the first line of the call renderer so it stays inline
			// (adding it as a separate child would stack it on its own line).
			const dotColor = this.result?.isError ? "error" : this.isPartial ? "warning" : "success";
			const dot = theme.fg(dotColor, "● ");

			const callRenderer = this.getCallRenderer();
			if (!callRenderer) {
				renderContainer.addChild(new PrefixFirstLine(dot, this.createCallFallback()));
				hasContent = true;
			} else {
				try {
					const component = callRenderer(this.args, theme, this.getRenderContext(this.callRendererComponent));
					this.callRendererComponent = component;
					renderContainer.addChild(new PrefixFirstLine(dot, component));
					hasContent = true;
				} catch {
					this.callRendererComponent = undefined;
					renderContainer.addChild(new PrefixFirstLine(dot, this.createCallFallback()));
					hasContent = true;
				}
			}

			if (this.result) {
				const resultRenderer = this.getResultRenderer();
				if (!resultRenderer) {
					const component = this.createResultFallback();
					if (component) {
						renderContainer.addChild(component);
						hasContent = true;
					}
				} else {
					try {
						const component = resultRenderer(
							{ content: this.result.content as any, details: this.result.details },
							{ expanded: this.expanded, isPartial: this.isPartial },
							theme,
							this.getRenderContext(this.resultRendererComponent),
						);
						this.resultRendererComponent = component;
						renderContainer.addChild(component);
						hasContent = true;
					} catch {
						this.resultRendererComponent = undefined;
						const component = this.createResultFallback();
						if (component) {
							renderContainer.addChild(component);
							hasContent = true;
						}
					}
				}
			}
		} else {
			// Boxless: no background fill.
			this.contentText.setCustomBgFn(undefined);
			this.contentText.setText(this.formatToolExecution());
			hasContent = true;
		}

		for (const img of this.imageComponents) {
			this.removeChild(img);
		}
		this.imageComponents = [];
		for (const spacer of this.imageSpacers) {
			this.removeChild(spacer);
		}
		this.imageSpacers = [];

		if (this.result) {
			const imageBlocks = this.result.content.filter((c) => c.type === "image");
			const caps = getCapabilities();
			for (let i = 0; i < imageBlocks.length; i++) {
				const img = imageBlocks[i];
				if (caps.images && this.showImages && img.data && img.mimeType) {
					const converted = this.convertedImages.get(i);
					const imageData = converted?.data ?? img.data;
					const imageMimeType = converted?.mimeType ?? img.mimeType;
					if (caps.images === "kitty" && imageMimeType !== "image/png") continue;

					const spacer = new Spacer(1);
					this.addChild(spacer);
					this.imageSpacers.push(spacer);
					const imageComponent = new Image(
						imageData,
						imageMimeType,
						{ fallbackColor: (s: string) => theme.fg("toolOutput", s) },
						{ maxWidthCells: this.imageWidthCells },
					);
					this.imageComponents.push(imageComponent);
					this.addChild(imageComponent);
				}
			}
		}

		if (this.hasRendererDefinition() && !hasContent && this.imageComponents.length === 0) {
			this.hideComponent = true;
		}
	}

	private getTextOutput(): string {
		return getRenderedTextOutput(this.result, this.showImages);
	}

	private formatToolExecution(): string {
		const dotColor = this.result?.isError ? "error" : this.isPartial ? "warning" : "success";
		let text = theme.fg(dotColor, "● ") + theme.fg("toolTitle", theme.bold(this.toolName));
		const content = JSON.stringify(this.args, null, 2);
		if (content) {
			text += `\n\n${content}`;
		}
		const output = this.getTextOutput();
		if (output) {
			text += `\n${output}`;
		}
		return text;
	}
}
