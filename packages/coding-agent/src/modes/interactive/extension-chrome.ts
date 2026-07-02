/**
 * Extension chrome: the widget slots above/below the editor and the custom
 * footer/header overrides that extensions can install via the
 * ExtensionUIContext. Extracted from interactive-mode.ts.
 */

import { type Component, Container, Spacer, Text, type TUI } from "@kolisachint/hoocode-tui";
import type { ExtensionWidgetOptions } from "../../core/extensions/index.js";
import type { FooterDataProvider } from "../../core/footer-data-provider.js";
import { isExpandable } from "./resource-display.js";
import { type Theme, theme } from "./theme/theme.js";

// Maximum total widget lines to prevent viewport overflow
const MAX_WIDGET_LINES = 10;

type DisposableComponent = Component & { dispose?(): void };

/** The slice of the interactive mode the chrome renders into. */
export interface ExtensionChromeDeps {
	ui: TUI;
	widgetContainerAbove: Container;
	widgetContainerBelow: Container;
	headerContainer: Container;
	/** The built-in footer component (restored when the custom footer is removed). */
	footer: Component;
	footerDataProvider: FooterDataProvider;
	/** The built-in header, once init() created it (undefined before that). */
	getBuiltInHeader(): Component | undefined;
	isToolOutputExpanded(): boolean;
}

export class ExtensionChrome {
	private widgetsAbove = new Map<string, DisposableComponent>();
	private widgetsBelow = new Map<string, DisposableComponent>();
	/** Custom footer set by an extension; undefined = built-in footer. */
	private footerOverride: DisposableComponent | undefined = undefined;
	/** Custom header set by an extension; undefined = built-in header. */
	private headerOverride: DisposableComponent | undefined = undefined;

	constructor(private readonly deps: ExtensionChromeDeps) {}

	/** The extension-installed header, if any (combined with the built-in by callers). */
	get customHeader(): DisposableComponent | undefined {
		return this.headerOverride;
	}

	setWidget(
		key: string,
		content: string[] | ((tui: TUI, thm: Theme) => DisposableComponent) | undefined,
		options?: ExtensionWidgetOptions,
	): void {
		const placement = options?.placement ?? "aboveEditor";
		const removeExisting = (map: Map<string, DisposableComponent>) => {
			const existing = map.get(key);
			if (existing?.dispose) existing.dispose();
			map.delete(key);
		};

		removeExisting(this.widgetsAbove);
		removeExisting(this.widgetsBelow);

		if (content === undefined) {
			this.renderWidgets();
			return;
		}

		let component: DisposableComponent;

		if (Array.isArray(content)) {
			// Wrap string array in a Container with Text components
			const container = new Container();
			for (const line of content.slice(0, MAX_WIDGET_LINES)) {
				container.addChild(new Text(line, 1, 0));
			}
			if (content.length > MAX_WIDGET_LINES) {
				container.addChild(new Text(theme.fg("muted", "... (widget truncated)"), 1, 0));
			}
			component = container;
		} else {
			// Factory function - create component
			component = content(this.deps.ui, theme);
		}

		const targetMap = placement === "belowEditor" ? this.widgetsBelow : this.widgetsAbove;
		targetMap.set(key, component);
		this.renderWidgets();
	}

	clearWidgets(): void {
		for (const widget of this.widgetsAbove.values()) {
			widget.dispose?.();
		}
		for (const widget of this.widgetsBelow.values()) {
			widget.dispose?.();
		}
		this.widgetsAbove.clear();
		this.widgetsBelow.clear();
		this.renderWidgets();
	}

	/**
	 * Render all extension widgets to the widget container.
	 */
	renderWidgets(): void {
		if (!this.deps.widgetContainerAbove || !this.deps.widgetContainerBelow) return;
		this.renderWidgetContainer(this.deps.widgetContainerAbove, this.widgetsAbove, true, true);
		this.renderWidgetContainer(this.deps.widgetContainerBelow, this.widgetsBelow, false, false);
		this.deps.ui.requestRender();
	}

	private renderWidgetContainer(
		container: Container,
		widgets: Map<string, DisposableComponent>,
		spacerWhenEmpty: boolean,
		leadingSpacer: boolean,
	): void {
		container.clear();

		if (widgets.size === 0) {
			if (spacerWhenEmpty) {
				container.addChild(new Spacer(1));
			}
			return;
		}

		if (leadingSpacer) {
			container.addChild(new Spacer(1));
		}
		for (const component of widgets.values()) {
			container.addChild(component);
		}
	}

	/**
	 * Set a custom footer component, or restore the built-in footer.
	 */
	setFooter(
		factory: ((tui: TUI, thm: Theme, footerData: FooterDataProvider) => DisposableComponent) | undefined,
	): void {
		// Dispose existing custom footer
		if (this.footerOverride?.dispose) {
			this.footerOverride.dispose();
		}

		// Remove current footer from UI
		if (this.footerOverride) {
			this.deps.ui.removeChild(this.footerOverride);
		} else {
			this.deps.ui.removeChild(this.deps.footer);
		}

		if (factory) {
			// Create and add custom footer, passing the data provider
			this.footerOverride = factory(this.deps.ui, theme, this.deps.footerDataProvider);
			this.deps.ui.addChild(this.footerOverride);
		} else {
			// Restore built-in footer
			this.footerOverride = undefined;
			this.deps.ui.addChild(this.deps.footer);
		}

		this.deps.ui.requestRender();
	}

	/**
	 * Set a custom header component, or restore the built-in header.
	 */
	setHeader(factory: ((tui: TUI, thm: Theme) => DisposableComponent) | undefined): void {
		// Header may not be initialized yet if called during early initialization
		const builtInHeader = this.deps.getBuiltInHeader();
		if (!builtInHeader) {
			return;
		}

		// Dispose existing custom header
		if (this.headerOverride?.dispose) {
			this.headerOverride.dispose();
		}

		// Find the index of the current header in the header container
		const currentHeader = this.headerOverride || builtInHeader;
		const index = this.deps.headerContainer.children.indexOf(currentHeader);

		if (factory) {
			// Create and add custom header
			this.headerOverride = factory(this.deps.ui, theme);
			if (isExpandable(this.headerOverride)) {
				this.headerOverride.setExpanded(this.deps.isToolOutputExpanded());
			}
			if (index !== -1) {
				this.deps.headerContainer.children[index] = this.headerOverride;
			} else {
				// If not found (e.g. builtInHeader was never added), add at the top
				this.deps.headerContainer.children.unshift(this.headerOverride);
			}
		} else {
			// Restore built-in header
			this.headerOverride = undefined;
			if (isExpandable(builtInHeader)) {
				builtInHeader.setExpanded(this.deps.isToolOutputExpanded());
			}
			if (index !== -1) {
				this.deps.headerContainer.children[index] = builtInHeader;
			}
		}

		this.deps.ui.requestRender();
	}

	/** Remove custom footer/header and all widgets (session reset). */
	reset(): void {
		this.setFooter(undefined);
		this.setHeader(undefined);
		this.clearWidgets();
	}
}
