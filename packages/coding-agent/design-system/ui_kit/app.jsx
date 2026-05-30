/* HooCode UI kit — interactive shell.
   A faithful, click-through recreation of one HooCode session: type a prompt,
   watch a deterministic agent turn stream in (thinking, read, edit+diff, bash),
   with the task panel and footer token counters updating live. Slash commands
   open inline selectors. `!` enters bash mode. Nothing here calls a real model. */

const { useState, useEffect, useRef, useCallback } = React;

/* Braille spinner, 80ms, accent-colored — the only animated glyph. */
const FRAMES = ["\u280B","\u2819","\u2839","\u2838","\u283C","\u2834","\u2826","\u2827","\u2807","\u280F"];
function Spinner() {
  const [f, setF] = useState(0);
  useEffect(() => { const t = setInterval(() => setF((x) => (x + 1) % FRAMES.length), 80); return () => clearInterval(t); }, []);
  return <span className="accent">{FRAMES[f]}</span>;
}
window.Spinner = Spinner;

/* ---- scripted turn content (presentational React nodes) ---- */
function readOutput() {
  return (
    <span className="out">{"\n"}
      <span className="s-cm">{"// footer clamps the context warning at the auto-compact point"}</span>{"\n"}
      <span className="s-kw">it</span>(<span className="s-st">"warns within 10pp of the threshold"</span>, () =&gt; {"{"}{"\n"}
      {"  "}<span className="s-kw">const</span> <span className="s-va">f</span> = <span className="s-fn">render</span>(<span className="s-nu">82</span>);{"\n"}
      {"  "}<span className="s-fn">expect</span>(<span className="s-va">f</span>.<span className="s-va">cls</span>).<span className="s-fn">toBe</span>(<span className="s-st">"warning"</span>);{"\n"}
      {"}"});
      {"\n"}<MoreHint n={210} />
    </span>
  );
}
const diffLines = [
  { type: "ctx", n: "41", parts: [{ t: "text", v: "const pct = usage.percent;" }] },
  { type: "rem", n: "42", parts: [{ t: "text", v: "if (pct > " }, { t: "inv", v: "90" }, { t: "text", v: ") return 'warning';" }] },
  { type: "add", n: "42", parts: [{ t: "text", v: "if (pct > " }, { t: "inv", v: "threshold - 10" }, { t: "text", v: ") return 'warning';" }] },
  { type: "ctx", n: "43", parts: [{ t: "text", v: "return 'normal';" }] },
];

let _id = 0;
const nextId = () => ++_id;

function App() {
  const [entries, setEntries] = useState([]);          // {id, node}
  const [tasks, setTasks] = useState([]);
  const [input, setInput] = useState("");
  const [bash, setBash] = useState(false);
  const [busy, setBusy] = useState(false);
  const [selector, setSelector] = useState(null);      // {kind,...} | null
  const [selIndex, setSelIndex] = useState(0);
  const [footer, setFooter] = useState({
    cwd: "~/work/hoocode", branch: "main", session: "", mode: "build",
    model: "claude-sonnet-4", thinking: "high",
    stats: { input: 8200, output: 1400, cacheRead: 21000, cacheWrite: 900, cost: 0.031, sub: true },
    ctxPercent: 12.4, ctxWindow: 200000,
  });

  const screenRef = useRef(null);
  const inputRef = useRef(null);
  const timers = useRef([]);

  const scrollDown = () => {
    const el = screenRef.current;
    if (el) requestAnimationFrame(() => { el.scrollTop = el.scrollHeight; });
  };
  useEffect(scrollDown, [entries, tasks, selector]);
  useEffect(() => () => timers.current.forEach(clearTimeout), []);

  const push = (node) => { const id = nextId(); setEntries((e) => [...e, { id, node }]); return id; };
  const update = (id, node) => setEntries((e) => e.map((x) => (x.id === id ? { id, node } : x)));
  const after = (ms, fn) => { const t = setTimeout(fn, ms); timers.current.push(t); };
  const bump = (d) => setFooter((f) => ({
    ...f,
    stats: {
      input: f.stats.input + (d.input || 0), output: f.stats.output + (d.output || 0),
      cacheRead: f.stats.cacheRead + (d.cacheRead || 0), cacheWrite: f.stats.cacheWrite + (d.cacheWrite || 0),
      cost: +(f.stats.cost + (d.cost || 0)).toFixed(3), sub: true,
    },
    ctxPercent: Math.min(99, +(f.ctxPercent + (d.ctx || 0)).toFixed(1)),
  }));

  /* The deterministic agent turn. Echoes the user's real text, then streams. */
  function runTurn(text) {
    setBusy(true);
    push(<UserMsg text={text} />);

    after(350, () => {
      push(<Thinking text={"Let me read the failing test, then the footer component it covers."} />);
      bump({ output: 90, ctx: 0.6 });
    });

    after(950, () => {
      setTasks([
        { id: 1, title: "Read footer test + component", status: "in_progress" },
        { id: 2, title: "Patch the warning threshold", status: "pending" },
        { id: 3, title: "Run the test suite", status: "pending", mode: "subagent" },
      ]);
      push(<ToolBlock status="success" title="read" path="test/footer.test.ts" range=":1-20">{readOutput()}</ToolBlock>);
      bump({ input: 1800, cacheRead: 4200, cost: 0.006, ctx: 1.1 });
    });

    after(1750, () => {
      push(<AssistantText>The warning should trip relative to the auto-compact point, not a hard 90%. I'll read the threshold from settings.</AssistantText>);
      setTasks((t) => t.map((x) => x.id === 1 ? { ...x, status: "done" } : x.id === 2 ? { ...x, status: "in_progress" } : x));
      bump({ output: 120, ctx: 0.5 });
    });

    after(2550, () => {
      push(<ToolBlock status="success" title="edit" path="packages/coding-agent/.../footer.ts"><Diff lines={diffLines} /></ToolBlock>);
      setTasks((t) => t.map((x) => x.id === 2 ? { ...x, status: "done" } : x.id === 3 ? { ...x, status: "in_progress" } : x));
      bump({ input: 2100, output: 240, cacheWrite: 1200, cost: 0.009, ctx: 1.2 });
    });

    // bash: pending → success
    after(3300, () => {
      const id = push(
        <ToolBlock status="pending" command="npm test -- footer">
          <span className="out"><Spinner /> <span className="hint">running…</span></span>
          <span className="took">{"\n"}Elapsed 0.0s</span>
        </ToolBlock>
      );
      after(2100, () => {
        update(id,
          <ToolBlock status="success" command="npm test -- footer" took="2.1s">
            <span className="out">{"\n"}PASS  test/footer.test.ts{"\n"}<span className="success">  ✓ warns within 10pp of the threshold</span>{"\n"}Tests: 6 passed, 6 total</span>
          </ToolBlock>
        );
        setTasks((t) => t.map((x) => x.id === 3 ? { ...x, status: "done" } : x));
        bump({ input: 900, output: 80, cost: 0.004, ctx: 0.4 });
      });
    });

    after(5900, () => {
      push(
        <AssistantText>
          <span className="md-h"># Done</span>{"\n"}
          The footer now derives the warning band from{" "}
          <span className="md-code">contextWindow - reserveTokens</span>, so it tracks the real
          auto-compact trip point. All 6 footer tests pass.
        </AssistantText>
      );
      setTasks([]);
      bump({ output: 160, ctx: 0.3 });
      setBusy(false);
    });
  }

  /* slash commands → inline selectors / blocks */
  function handleSlash(cmd) {
    if (cmd === "/model") {
      setSelIndex(0);
      setSelector({
        kind: "model", title: "Select model", hints: <><span className="keyhint">↑↓</span><span className="hint"> move</span> <span className="hint">·</span> <span className="keyhint">enter</span><span className="hint"> select</span> <span className="hint">·</span> <span className="keyhint">esc</span><span className="hint"> close</span></>,
        sub: "Type to filter · Ctrl+P cycles scope",
        rows: [
          { label: "claude-sonnet-4", desc: "anthropic · thinking high" },
          { label: "claude-opus-4", desc: "anthropic" },
          { label: "gpt-5", desc: "openai" },
          { label: "gemini-2.5-pro", desc: "google" },
          { label: "qwen3-coder:30b", desc: "ollama · local" },
        ],
      });
      return;
    }
    if (cmd === "/config") {
      setSelIndex(0);
      setSelector({
        kind: "toggle", title: "Resource Configuration",
        hints: <><span className="keyhint">space</span><span className="hint"> toggle</span> <span className="hint">·</span> <span className="keyhint">esc</span><span className="hint"> close</span></>,
        sub: "Type to filter resources",
        rows: [
          { label: "make-deck", desc: "skill", on: true },
          { label: "read-pdf", desc: "skill", on: false },
          { label: "hifi-design", desc: "skill", on: true },
          { label: "web-search", desc: "extension", on: true },
        ],
      });
      return;
    }
    if (cmd === "/help") {
      push(
        <div className="custom">
          <span className="custom-label">commands{"\n"}</span>
          <span className="custom-text">
            {"  "}/model{"    "}switch model{"\n"}
            {"  "}/config{"   "}enable / disable skills &amp; extensions{"\n"}
            {"  "}/sessions{" "}resume a past session{"\n"}
            {"  "}/clear{"    "}clear the transcript{"\n"}
            {"  "}!cmd{"      "}run a bash command{"\n"}
            {"  "}ctrl+r{"    "}expand the last tool output
          </span>
        </div>
      );
      return;
    }
    if (cmd === "/clear") { setEntries([]); setTasks([]); return; }
    push(<AssistantText><span className="error">Unknown command: {cmd}</span> — try <span className="md-code">/help</span>.</AssistantText>);
  }

  function submit() {
    const text = input.trim();
    if (!text || busy) return;
    setInput("");
    if (text.startsWith("/")) { handleSlash(text.split(" ")[0]); return; }
    if (bash || text.startsWith("!")) {
      const cmd = text.replace(/^!/, "");
      setBash(false);
      const id = push(
        <ToolBlock status="pending" command={cmd}>
          <span className="out"><Spinner /> <span className="hint">running…</span></span>
        </ToolBlock>
      );
      after(1200, () => update(id,
        <ToolBlock status="success" command={cmd} took="0.4s">
          <span className="out">{"\n"}<span className="s-cm"># (demo) command completed</span></span>
        </ToolBlock>
      ));
      return;
    }
    runTurn(text);
  }

  function onKeyDown(e) {
    if (selector) {
      if (e.key === "ArrowDown") { e.preventDefault(); setSelIndex((i) => Math.min(selector.rows.length - 1, i + 1)); }
      else if (e.key === "ArrowUp") { e.preventDefault(); setSelIndex((i) => Math.max(0, i - 1)); }
      else if (e.key === "Enter") {
        e.preventDefault();
        if (selector.kind === "model") {
          const m = selector.rows[selIndex];
          setFooter((f) => ({ ...f, model: m.label, thinking: m.desc.includes("high") ? "high" : f.thinking }));
        }
        setSelector(null);
      } else if (e.key === "Escape") { e.preventDefault(); setSelector(null); }
      else if (e.key === " " && selector.kind === "toggle") {
        e.preventDefault();
        setSelector((s) => ({ ...s, rows: s.rows.map((r, i) => i === selIndex ? { ...r, on: !r.on } : r) }));
      }
      return;
    }
    if (e.key === "Enter") { e.preventDefault(); submit(); }
  }

  function onChange(e) {
    const v = e.target.value;
    setInput(v);
    setBash(v.startsWith("!"));
  }

  return (
    <div className="term-window" onClick={() => inputRef.current && inputRef.current.focus()}>
      <div className="titlebar">
        <div className="lights"><span className="light r"></span><span className="light y"></span><span className="light g"></span></div>
        <div className="title">hoo — {footer.cwd}</div>
        <div style={{ width: "52px" }}></div>
      </div>

      <div className="screen" ref={screenRef}>
        <Banner />
        {entries.map((e) => <div className="turn" key={e.id}>{e.node}</div>)}
        {selector && <Selector {...selector} index={selIndex} />}
      </div>

      <div className="dock">
        <TaskPanel tasks={tasks} />
        <div className={"editor" + (bash ? " bash" : "")}>
          <span className="prompt">{bash ? "!" : ">"}</span>
          <input
            ref={inputRef}
            value={input}
            onChange={onChange}
            onKeyDown={onKeyDown}
            placeholder={selector ? "selector open — ↑↓ then enter, esc to close" : busy ? "working… (esc interrupts)" : "ask hoocode to do something, or /help"}
            autoFocus
            spellCheck={false}
          />
        </div>
        <Footer {...footer} />
      </div>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
