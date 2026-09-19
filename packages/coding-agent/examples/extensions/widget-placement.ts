import type { ExtensionAPI } from "@kolisachint/hoocode-agent";

export default function widgetPlacementExtension(hoo: ExtensionAPI) {
	hoo.on("session_start", (_event, ctx) => {
		if (!ctx.hasUI) return;
		ctx.ui.setWidget("widget-above", ["Above editor widget"]);
		ctx.ui.setWidget("widget-below", ["Below editor widget"], { placement: "belowEditor" });
	});
}
