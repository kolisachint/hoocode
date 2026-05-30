/* HooCode UI kit — presentational components.
   All visuals derive from the design system tokens. Exported to window. */

const { useState, useEffect, useRef } = React;

/* Welcome banner shown at the top of a fresh session. */
function Banner() {
  const art =
`  __  __            ______          __
 / / / /___  ____  / ____/___  ____/ /__
/ /_/ / __ \\/ __ \\/ /   / __ \\/ __  / _ \\
\\__,_/\\____/\\____/\\____/\\____/\\__,_/\\___/`;
  return (
    <div className="turn">
      <div className="banner">{art}</div>
      <div className="banner-sub">deterministic terminal coding agent · v0.4.1</div>
      <div className="banner-tips">
        <b>/help</b> commands &nbsp; <b>/model</b> switch model &nbsp; <b>!</b> bash mode &nbsp;
        <b>ctrl+r</b> expand tool output &nbsp; <b>ctrl+c</b> exit
      </div>
    </div>
  );
}

/* Status-icon task panel, pinned above the prompt. */
const TASK_ICON = { pending: "\u25CF", in_progress: "\u25D0", done: "\u2713", failed: "\u2717" };
function TaskPanel({ tasks }) {
  const active = tasks.filter((t) => t.status === "pending" || t.status === "in_progress");
  if (active.length === 0) return null;
  return (
    <div className="taskpanel">
      {active.map((t) => (
        <div className="task" key={t.id}>
          <span className={"ic " + t.status}>{TASK_ICON[t.status]}</span>
          <span className="id">#{t.id}</span>
          <span>{t.title}</span>
          {t.mode && <span className="mode"> [{t.mode}]</span>}
        </div>
      ))}
    </div>
  );
}

/* Two-line footer: cwd/mode, then token+context+model stats. */
function fmtTok(n) {
  if (n < 1000) return String(n);
  if (n < 10000) return (n / 1000).toFixed(1) + "k";
  if (n < 1000000) return Math.round(n / 1000) + "k";
  return (n / 1000000).toFixed(1) + "M";
}
function Footer({ cwd, branch, session, mode, stats, ctxPercent, ctxWindow, model, thinking }) {
  const ctxClass = ctxPercent >= 87 ? "ctx-err" : ctxPercent >= 80 ? "ctx-warn" : "l";
  const parts = [];
  if (stats.input) parts.push("\u2191" + fmtTok(stats.input));
  if (stats.output) parts.push("\u2193" + fmtTok(stats.output));
  if (stats.cacheRead) parts.push("R" + fmtTok(stats.cacheRead));
  if (stats.cacheWrite) parts.push("W" + fmtTok(stats.cacheWrite));
  if (stats.cost) parts.push("$" + stats.cost.toFixed(3) + (stats.sub ? " (sub)" : ""));
  return (
    <div className="footer">
      <div className="line">
        <span className="l">
          {cwd}{branch ? ` (${branch})` : ""}{session ? ` \u2022 ${session}` : ""}
        </span>
        <span className="r mode">{mode}</span>
      </div>
      <div className="line">
        <span className="l">
          {parts.join(" ")}{parts.length ? " " : ""}
          <span className={ctxClass}>{ctxPercent.toFixed(1)}%/{fmtTok(ctxWindow)} (auto @ 90%)</span>
        </span>
        <span className="r">{model}{thinking ? ` \u2022 ${thinking}` : ""}</span>
      </div>
    </div>
  );
}

/* Tinted tool-execution block. */
function ToolBlock({ status = "success", title, path, range, command, children, took, elapsed }) {
  return (
    <div className={"tool " + status}>
      <span>
        {command != null ? (
          <span className="tool-title">$ {command}</span>
        ) : (
          <>
            <span className="tool-title">{title}</span>
            {path != null && <> <span className="tool-path">{path}</span></>}
            {range != null && <span className="range">{range}</span>}
          </>
        )}
      </span>
      {children}
      {(took || elapsed) && (
        <span className="took">{"\n"}{elapsed ? `Elapsed ${elapsed}` : `Took ${took}`}</span>
      )}
    </div>
  );
}

/* Truncated-output hint line, e.g. "... (210 more lines, ctrl+r to expand)" */
function MoreHint({ n, word = "more" }) {
  return (
    <span className="out">
      <span className="hint">... ({n} {word} lines, </span>
      <span className="keyhint">ctrl+r</span>
      <span className="hint"> to expand)</span>
    </span>
  );
}

/* Diff body. lines: [{type:'ctx'|'add'|'rem', n, parts:[{t,v}]}] */
function Diff({ lines }) {
  const cls = { ctx: "d-ctx", add: "d-add", rem: "d-rem" };
  const sign = { ctx: " ", add: "+", rem: "-" };
  return (
    <span className="out" style={{ marginTop: "6px" }}>
      {lines.map((ln, i) => (
        <span className={cls[ln.type]} key={i}>
          {sign[ln.type]}{ln.n}{" "}
          {ln.parts.map((p, j) =>
            p.t === "inv" ? <span className="inv" key={j}><span>{p.v}</span></span> : <span key={j}>{p.v}</span>
          )}
        </span>
      ))}
    </span>
  );
}

/* Skill / custom collapsed block. */
function CustomBlock({ label, text }) {
  return (
    <div className="custom">
      <span className="custom-label">[{label}] </span>
      <span className="custom-text">{text}</span>
      <span className="keyhint"> (ctrl+r to expand)</span>
    </div>
  );
}

function UserMsg({ text }) { return <div className="user-msg">{text}</div>; }
function AssistantText({ children }) { return <div className="assistant-text">{children}</div>; }
function Thinking({ text }) { return <div className="thinking">{text}</div>; }

/* Inline selector (model picker, resource config, etc.). */
function Selector({ title, hints, sub, rows, index, kind = "list" }) {
  const rule = "\u2500".repeat(60);
  return (
    <div className="turn">
      <div className="rule">{rule}</div>
      <div className="sel">
        <div className="sel-head">
          <span className="sel-title">{title}</span>
          <span className="sel-hints">{hints}</span>
        </div>
        {sub && <div className="sel-sub">{sub}</div>}
        <div style={{ height: "8px" }}></div>
        {rows.map((r, i) => (
          <div className={"sel-row" + (i === index ? " active" : "")} key={i}>
            <span className="cur">{i === index ? ">" : " "}</span>
            {kind === "toggle" && (
              <span className={"sel-check " + (r.on ? "on" : "off")}>{r.on ? "[x]" : "[ ]"}</span>
            )}
            <span className="label">{r.label}</span>
            {r.desc && <span className="desc">  {r.desc}</span>}
          </div>
        ))}
        <div className="sel-count">({index + 1}/{rows.length})</div>
      </div>
      <div className="rule">{rule}</div>
    </div>
  );
}

Object.assign(window, {
  Banner, TaskPanel, Footer, ToolBlock, MoreHint, Diff, CustomBlock,
  UserMsg, AssistantText, Thinking, Selector, fmtTok,
});
