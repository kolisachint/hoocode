#!/usr/bin/env node
// Generates assets/demo.svg — an animated terminal recording of a real hoocode
// plan → /approve → build session. Pure CSS keyframes so it plays inline on the
// GitHub README (GitHub renders <img src="*.svg"> with CSS animation, no JS).
//
// Colours, prompt glyphs, the "Allow:" permission gate, the status dots, the
// diff format and the footer all mirror the real TUI:
//   theme:  packages/coding-agent/src/modes/interactive/theme/dark.json
//   gate:   packages/coding-agent/src/extensions/core/hoo-core.ts  (Allow: …)
//   footer: packages/coding-agent/src/modes/interactive/components/footer.ts
//
// Re-run after editing: `node scripts/gen-demo-svg.mjs`
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

// ── palette (from dark.json) ──────────────────────────────────────────────
const C = {
  bg: "#13131a",          // terminal body
  chrome: "#1e1e24",      // title bar (export.cardBg)
  border: "#2a2a33",
  text: "#c9cdd3",        // default terminal text
  dim: "#666666",
  muted: "#808080",
  accent: "#8abeb7",      // mode label / active option cursor
  cyan: "#00d7ff",        // borderAccent — "Allow:" / rules
  green: "#b5bd68",       // success dot, diff +, ✓
  red: "#cc6666",         // error / diff -
  yellow: "#e6db74",      // pending dot (softened #ffff00)
  blue: "#5f87ff",
  heading: "#f0c674",
  userBg: "#1c1c26",      // user-message background
};

// ── layout ────────────────────────────────────────────────────────────────
const W = 840;
const PADX = 22;
const HEAD = 46;          // title bar height
const LH = 19;            // line height
const FS = 13.5;          // font size
const CW = FS * 0.6;      // monospace char advance (~8.1px)
const TOP = HEAD + 22;    // first body line baseline

// Loop timing
const T = 15.5;           // full loop seconds
const FADE = 0.18;        // per-line fade-in

// helper to build a coloured run
const s = (text, fill = C.text, opts = {}) => ({ text, fill, ...opts });

// Each row: { t: cueSeconds, runs: [seg…], user?:bool, gap?:px-before }
// user:true paints the dark user-message background behind the row.
const rows = [
  { t: 0.4,  user: true,  runs: [s("> ", C.muted), s("/plan")] },
  { t: 1.0,  runs: [s("  plan mode", C.accent), s(" — explore and design, no source edits", C.dim)] },

  { t: 1.8,  gap: 8, user: true, runs: [s("> ", C.muted), s("Add a /healthz endpoint to the Express server")] },
  { t: 2.5,  runs: [s("● ", C.green), s("read  ", C.muted), s("src/app.ts")] },
  { t: 2.8,  runs: [s("● ", C.green), s("read  ", C.muted), s("src/routes/index.ts")] },
  { t: 3.1,  runs: [s("● ", C.green), s("write ", C.muted), s(".hoocode/plan.md")] },
  { t: 3.8,  gap: 6, runs: [s("Plan written to "), s(".hoocode/plan.md", C.accent), s(" — run "), s("/approve", C.cyan), s(" to begin execution.")] },

  { t: 4.7,  gap: 8, user: true, runs: [s("> ", C.muted), s("/approve")] },
  { t: 5.3,  runs: [s("  build mode", C.accent), s(" — executing plan", C.dim)] },

  { t: 6.0,  gap: 8, runs: [s("Allow: ", C.cyan, { bold: true }), s("edit src/routes/health.ts")] },
  { t: 6.3,  runs: [s("> ", C.accent), s("1 ", C.accent), s("Yes (once)", C.text, { bold: true })] },
  { t: 6.45, runs: [s("  ", C.dim), s("2 ", C.dim), s("No (block)", C.muted)] },
  { t: 6.6,  runs: [s("  ", C.dim), s("3 ", C.dim), s("Always (add to auto-allow for this mode)", C.muted)] },

  { t: 7.4,  gap: 8, runs: [s("● ", C.green), s("edit  ", C.muted), s("src/routes/health.ts")] },
  { t: 7.8,  runs: [s("   + ", C.green), s("router.get('/healthz', (_req, res) =>", C.green)] },
  { t: 8.0,  runs: [s("   + ", C.green), s("  res.json({ status: 'ok', uptime: process.uptime() })", C.green)] },
  { t: 8.2,  runs: [s("   + ", C.green), s("})", C.green)] },

  { t: 9.0,  gap: 8, runs: [s("Allow: ", C.cyan, { bold: true }), s("$ npm test")] },
  { t: 9.4,  runs: [s("● ", C.green), s("$ npm test", C.green, { bold: true })] },
  { t: 10.2, runs: [s("  ✓ ", C.green), s("14 passing", C.muted)] },

  { t: 11.1, gap: 8, runs: [s("Done — "), s("/healthz", C.accent), s(" is live.")] },
  { t: 12.0, gap: 4, runs: [s("> ", C.muted)], cursor: true },
];

// ── build SVG ───────────────────────────────────────────────────────────
const root = dirname(dirname(fileURLToPath(import.meta.url)));
let y = TOP;
const lineEls = [];
const kf = [];

const esc = (t) =>
  t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

rows.forEach((row, i) => {
  if (row.gap) y += row.gap;
  const cls = `l${i}`;
  // keyframe: hidden until cue, visible, then fade out at very end to reset
  const p = ((row.t / T) * 100).toFixed(2);
  const pEnd = (((row.t + FADE) / T) * 100).toFixed(2);
  kf.push(
    `@keyframes ${cls}{0%,${p}%{opacity:0}${pEnd}%,96%{opacity:1}100%{opacity:0}}` +
      `.${cls}{opacity:0;animation:${cls} ${T}s linear infinite}`,
  );

  let parts = "";
  // user-message background band
  if (row.user) {
    parts += `<rect x="10" y="${(y - LH + 4).toFixed(1)}" width="${W - 20}" height="${LH}" rx="3" fill="${C.userBg}"/>`;
  }
  // text runs as tspans
  let tspans = "";
  for (const r of row.runs) {
    const w = r.bold ? ' font-weight="600"' : "";
    tspans += `<tspan fill="${r.fill}"${w}>${esc(r.text)}</tspan>`;
  }
  parts += `<text x="${PADX}" y="${y.toFixed(1)}" font-size="${FS}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace" xml:space="preserve">${tspans}</text>`;

  // blinking cursor block after the prompt on the final line
  if (row.cursor) {
    const cx = PADX + 2 * CW;
    parts += `<rect class="cur" x="${cx.toFixed(1)}" y="${(y - FS + 2).toFixed(1)}" width="${(CW * 0.9).toFixed(1)}" height="${FS}" fill="${C.cyan}"/>`;
  }

  lineEls.push(`<g class="${cls}">${parts}</g>`);
  y += LH;
});

const bodyBottom = y + 6;

// footer: pwd + git left, mode right (crossfade plan→build); stats line.
const footY1 = bodyBottom + 22;
const footY2 = footY1 + LH;
const ruleY = bodyBottom + 6;
const Hsvg = footY2 + 18;

const footer = `
  <line x1="0" y1="${ruleY}" x2="${W}" y2="${ruleY}" stroke="${C.border}" stroke-width="1"/>
  <text x="${PADX}" y="${footY1}" font-size="${FS}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">
    <tspan fill="${C.muted}">~/myproject </tspan><tspan fill="${C.dim}">(main)</tspan>
  </text>
  <text x="${W - PADX}" y="${footY1}" text-anchor="end" font-size="${FS}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">
    <tspan class="mPlan" fill="${C.accent}">plan</tspan><tspan class="mBuild" fill="${C.accent}">build</tspan>
  </text>
  <text x="${PADX}" y="${footY2}" font-size="${FS}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">
    <tspan fill="${C.muted}">↑1.2k ↓340 $0.004  6.1%/200k </tspan><tspan fill="${C.dim}">(auto @ 92%)</tspan>
  </text>
  <text x="${W - PADX}" y="${footY2}" text-anchor="end" font-size="${FS}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">
    <tspan fill="${C.dim}">(anthropic) </tspan><tspan fill="${C.muted}">claude-sonnet-4-6</tspan>
  </text>`;

// crossfade the mode word at the /approve moment (~ t 4.7 → 5.3)
const mPlanEnd = ((4.9 / T) * 100).toFixed(1);
const mBuildStart = ((5.0 / T) * 100).toFixed(1);

const css = `
    text{dominant-baseline:alphabetic}
    @keyframes blink{0%,49%{opacity:1}50%,100%{opacity:0}}
    .cur{animation:blink 1.06s steps(1,end) infinite}
    @keyframes mp{0%,${mPlanEnd}%{opacity:1}${mBuildStart}%,100%{opacity:0}}
    @keyframes mb{0%,${mPlanEnd}%{opacity:0}${mBuildStart}%,96%{opacity:1}100%{opacity:0}}
    .mPlan{animation:mp ${T}s linear infinite}
    .mBuild{opacity:0;animation:mb ${T}s linear infinite}
    ${kf.join("\n    ")}
`;

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${Hsvg}" width="${W}" height="${Hsvg}" role="img" aria-label="HooCode terminal session: plan, approve, then build with a permission gate">
  <title>HooCode — plan → /approve → build</title>
  <style>${css}</style>
  <rect width="${W}" height="${Hsvg}" rx="12" fill="${C.bg}"/>
  <rect width="${W}" height="${HEAD}" rx="12" fill="${C.chrome}"/>
  <rect y="${HEAD - 12}" width="${W}" height="12" fill="${C.chrome}"/>
  <line x1="0" y1="${HEAD}" x2="${W}" y2="${HEAD}" stroke="${C.border}" stroke-width="1"/>
  <circle cx="24" cy="${HEAD / 2}" r="6" fill="#cc6666"/>
  <circle cx="44" cy="${HEAD / 2}" r="6" fill="#e6c547"/>
  <circle cx="64" cy="${HEAD / 2}" r="6" fill="#7fb069"/>
  <text x="${W / 2}" y="${HEAD / 2 + 4}" text-anchor="middle" font-size="12.5" fill="${C.muted}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">~/myproject — hoocode</text>
  ${lineEls.join("\n  ")}
  ${footer}
</svg>
`;

const out = join(root, "assets", "demo.svg");
writeFileSync(out, svg);
console.log(`wrote ${out} (${svg.length} bytes, ${rows.length} lines, viewBox 0 0 ${W} ${Hsvg})`);
