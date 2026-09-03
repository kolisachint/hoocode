import { Container, type SelectItem, SelectList, type SelectListLayoutOptions } from "@kolisachint/hoocode-tui";
import { SESSION_COLOR_SLOTS, sessionColorName } from "../../../core/session-identity.js";
import { getSelectListTheme } from "../theme/theme.js";
import { DynamicBorder } from "./dynamic-border.js";
import { renderSessionChip } from "./session-chip.js";

const COLOR_SELECT_LIST_LAYOUT: SelectListLayoutOptions = {
	minPrimaryColumnWidth: 26,
	maxPrimaryColumnWidth: 40,
};

/**
 * Picker for the session's colour slot.
 *
 * Each row *is* the chip it would produce, in the session's current name, because
 * the slots have no useful names of their own: which of six hues reads best in
 * this theme on this terminal is a thing you look at, not a thing you can be told.
 */
export class SessionColorSelectorComponent extends Container {
	private selectList: SelectList;

	constructor(
		sessionName: string,
		currentSlot: number,
		onSelect: (slot: number) => void,
		onCancel: () => void,
		onPreview?: (slot: number) => void,
	) {
		super();

		const items: SelectItem[] = [];
		for (let slot = 1; slot <= SESSION_COLOR_SLOTS; slot++) {
			const chip = renderSessionChip(sessionName, slot);
			// The row carries its name as well as its swatch: the name is what
			// `/color <name>` takes, so the picker is also where you learn it.
			const name = sessionColorName(slot) ?? String(slot);
			items.push({
				value: String(slot),
				label: chip?.styled ?? sessionName,
				description: slot === currentSlot ? `${name} · current` : name,
			});
		}

		this.addChild(new DynamicBorder());

		this.selectList = new SelectList(items, items.length, getSelectListTheme(), COLOR_SELECT_LIST_LAYOUT);

		const currentIndex = items.findIndex((item) => item.value === String(currentSlot));
		if (currentIndex !== -1) {
			this.selectList.setSelectedIndex(currentIndex);
		}

		this.selectList.onSelect = (item) => {
			onSelect(Number(item.value));
		};
		this.selectList.onCancel = () => {
			onCancel();
		};
		if (onPreview) {
			this.selectList.onSelectionChange = (item) => {
				onPreview(Number(item.value));
			};
		}

		this.addChild(this.selectList);
		this.addChild(new DynamicBorder());
	}

	getSelectList(): SelectList {
		return this.selectList;
	}
}
