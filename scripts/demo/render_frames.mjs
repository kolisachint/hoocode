// Screenshot each recorded screen of the README demo (scripts/demo/record.py).
// Usage: NODE_PATH=$(npm root -g) node render_frames.mjs page.html frames.json out-dir
// Writes out-dir/0000.png, 0001.png, ... one per entry of frames.json (HTML rows).
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");
const [, , page, framesPath, outDir] = process.argv;
const frames = JSON.parse(readFileSync(framesPath, "utf8"));
const executablePath =
	process.env.PLAYWRIGHT_CHROMIUM || "/opt/pw-browsers/chromium-1194/chrome-linux/chrome";
const browser = await chromium.launch({ executablePath });
const tab = await browser.newPage({ viewport: { width: 1400, height: 900 }, deviceScaleFactor: 1 });
await tab.goto(pathToFileURL(page).href);
await tab.evaluate(() => document.fonts.ready);
const term = tab.locator("#t");
for (let i = 0; i < frames.length; i++) {
	await tab.evaluate((body) => {
		document.getElementById("t").innerHTML = body;
	}, frames[i]);
	await term.screenshot({ path: join(outDir, `${String(i).padStart(4, "0")}.png`) });
}
await browser.close();
