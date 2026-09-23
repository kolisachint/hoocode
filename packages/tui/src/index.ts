// Core TUI interfaces and classes

// Autocomplete support
export {
	type AutocompleteItem,
	type AutocompleteProvider,
	type AutocompleteSuggestions,
	CombinedAutocompleteProvider,
	type SlashCommand,
} from "./autocomplete.js";
// Components
export { Box, type PaperSheet } from "./components/box.js";
export { CancellableLoader } from "./components/cancellable-loader.js";
export {
	DEFAULT_EDITOR_BORDER_CHARS,
	Editor,
	type EditorBorderChars,
	type EditorBorderStyle,
	type EditorOptions,
	type EditorTheme,
	type EditorTopBorderLabel,
} from "./components/editor.js";
export {
	DEFAULT_FRAME_BORDER_CHARS,
	Frame,
	type FrameBorderChars,
	type FrameBorderStyle,
	type FrameEdgeOptions,
	type FrameLabel,
	type FrameOptions,
	renderFrameEdge,
} from "./components/frame.js";
export { Image, type ImageOptions, type ImageTheme } from "./components/image.js";
export { DEFAULT_INPUT_PROMPT, Input } from "./components/input.js";
export { Loader, type LoaderIndicatorOptions } from "./components/loader.js";
export { type DefaultTextStyle, Markdown, type MarkdownTheme } from "./components/markdown.js";
export {
	type SelectItem,
	SelectList,
	type SelectListLayoutOptions,
	type SelectListTheme,
	type SelectListTruncatePrimaryContext,
} from "./components/select-list.js";
export { type SettingItem, SettingsList, type SettingsListTheme } from "./components/settings-list.js";
export { FlexSpacer, Spacer } from "./components/spacer.js";
export { Text } from "./components/text.js";
export { TruncatedText } from "./components/truncated-text.js";
// Editor component interface (for custom editors)
export type { EditorComponent } from "./editor-component.js";
// Fuzzy matching
export { type FuzzyMatch, fuzzyFilter, fuzzyMatch } from "./fuzzy.js";
// Keybindings
export {
	getKeybindings,
	type Keybinding,
	type KeybindingConflict,
	type KeybindingDefinition,
	type KeybindingDefinitions,
	type Keybindings,
	type KeybindingsConfig,
	KeybindingsManager,
	setKeybindings,
	TUI_KEYBINDINGS,
} from "./keybindings.js";
// Keyboard input handling
export {
	decodeKittyPrintable,
	isKeyRelease,
	isKeyRepeat,
	isKittyProtocolActive,
	Key,
	type KeyEventType,
	type KeyId,
	matchesKey,
	parseKey,
	setKittyProtocolActive,
} from "./keys.js";
// Input buffering for batch splitting
// Mouse reporting
export {
	isMouseSequence,
	MOUSE_DISABLE,
	MOUSE_ENABLE,
	type MouseEvent,
	type MouseEventKind,
	mouseSequenceLength,
	parseMouseEvent,
} from "./mouse.js";
export { StdinBuffer, type StdinBufferEventMap, type StdinBufferOptions } from "./stdin-buffer.js";
// Terminal interface and implementations
export { ProcessTerminal, type Terminal } from "./terminal.js";
// Terminal image support
export {
	allocateImageId,
	type CellDimensions,
	calculateImageRows,
	deleteAllKittyImages,
	deleteKittyImage,
	detectCapabilities,
	encodeITerm2,
	encodeKitty,
	encodeSixel,
	getCapabilities,
	getCellDimensions,
	getGifDimensions,
	getImageDimensions,
	getImageRasterizer,
	getJpegDimensions,
	getPngDimensions,
	getWebpDimensions,
	hyperlink,
	type ImageDimensions,
	type ImageProtocol,
	type ImageRasterizer,
	type ImageRenderOptions,
	imageFallback,
	isImageLine,
	type RgbaImage,
	renderImage,
	resetCapabilitiesCache,
	setCapabilities,
	setCellDimensions,
	setImageRasterizer,
	type TerminalCapabilities,
} from "./terminal-image.js";
export {
	type Component,
	Container,
	CURSOR_MARKER,
	type Focusable,
	isFocusable,
	type OverlayAnchor,
	type OverlayHandle,
	type OverlayMargin,
	type OverlayOptions,
	type ScrollSearchStatus,
	type ScrollStatus,
	type ScrollStatusFormatter,
	type SizeValue,
	Slot,
	TUI,
} from "./tui.js";
// Utilities
export {
	applyBackgroundToLine,
	bareUrlAt,
	hyperlinkAt,
	truncateToWidth,
	visibleWidth,
	wrapTextWithAnsi,
} from "./utils.js";
