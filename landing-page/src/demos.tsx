import { useEffect, useRef, useState } from "react";

const reduced = () =>
  typeof window !== "undefined" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

export function DummyTerm({ title, starter, termKey, clearSignal }: { title: string; starter: string[]; termKey: string; clearSignal: number }) {
  const [lines, setLines] = useState<string[]>(starter);
  const [val, setVal] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    setLines(starter);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clearSignal]);
  const run = (e: React.FormEvent) => {
    e.preventDefault();
    const cmd = val;
    const name = cmd.trim().split(/\s+/)[0] ?? "";
    setLines((l) => [
      ...l,
      `$ ${cmd}`,
      cmd.trim() === "" ? "" : `sh: ${name}: demo terminal — nothing runs here`,
    ]);
    setVal("");
  };
  return (
    <div className="tty" data-term={termKey} onClick={() => inputRef.current?.focus()}>
      {lines.map((l, i) => (
        <p key={i} className={l.startsWith("$ ") ? "in" : "out"}>
          {l.startsWith("$ ") ? (
            <>
              <span className="ps">$ </span>
              {l.slice(2)}
            </>
          ) : (
            l || " "
          )}
        </p>
      ))}
      <form className="tin" onSubmit={run}>
        <span className="ps">$ </span>
        <input
          ref={inputRef}
          value={val}
          onChange={(e) => setVal(e.target.value)}
          aria-label={`Type in ${title} (demo, commands do nothing)`}
          spellCheck={false}
          autoComplete="off"
        />
      </form>
    </div>
  );
}

export function AgentsPane() {
  const [cards, setCards] = useState([
    { id: "c1", state: "needs permission", text: "Wants to edit crates/app/src/workspace.rs", live: true },
    { id: "c2", state: "completed", text: "Finished review pass, scrollback saved.", live: false },
  ]);
  if (cards.length === 0)
    return <p className="empty">Inbox zero. Hooks report here when an agent needs you.</p>;
  return (
    <div className="cards">
      {cards.map((c) => (
        <article key={c.id} className={c.live ? "acard" : "acard dim"}>
          <p className="atitle">
            <span className="adot" />
            {c.state}
          </p>
          <p className="atext">{c.text}</p>
          <div className="abtns">
            <button className="abtn go" onClick={() => setCards((cs) => cs.filter((x) => x.id !== c.id))}>go</button>
            <button className="abtn" onClick={() => setCards((cs) => cs.filter((x) => x.id !== c.id))}>snooze</button>
            <button className="abtn" onClick={() => setCards((cs) => cs.filter((x) => x.id !== c.id))}>dismiss</button>
          </div>
        </article>
      ))}
    </div>
  );
}

export function ProjectsDemo() {
  const [counts, setCounts] = useState<Record<string, number>>({
    terminator: 2,
    api: 1,
    dotfiles: 0,
  });
  const [sel, setSel] = useState("terminator");
  return (
    <div className="demo">
      <div className="drow">
        {Object.entries(counts).map(([p, c]) => (
          <button key={p} className={p === sel ? "pill on" : "pill"} onClick={() => setSel(p)}>
            {p} · {c} {c === 1 ? "tab" : "tabs"}
          </button>
        ))}
      </div>
      <div className="drow">
        <button className="abtn go" onClick={() => setCounts((m) => ({ ...m, [sel]: m[sel] + 1 }))}>
          + tab in {sel}
        </button>
        <button
          className="abtn"
          disabled={counts[sel] === 0}
          onClick={() => setCounts((m) => ({ ...m, [sel]: Math.max(0, m[sel] - 1) }))}
        >
          close tab
        </button>
      </div>
      <p className="dnote">Sessions live in the daemon — close this page and they would keep running.</p>
    </div>
  );
}

const SCROLLBACK = [
  ["api:dev", "VITE ready in 312 ms"],
  ["api:dev", "Local: http://localhost:5173/"],
  ["web:test", "test result: ok. 42 passed; 0 failed"],
  ["api:dev", "GET /health 200 2ms"],
  ["dotfiles", "$ stow -R zsh"],
  ["web:test", "warning: unused import `VecDeque`"],
  ["api:dev", "POST /agents/hook 200 11ms"],
  ["dotfiles", "$ git commit -m 'sync wezterm config'"],
];

export function HistoryDemo() {
  const [q, setQ] = useState("");
  const [jump, setJump] = useState<string | null>(null);
  const hits = SCROLLBACK.map((s, i) => ({ s, i })).filter(({ s: [sess, line] }) => {
    const needle = q.trim().toLowerCase();
    return !needle || `${sess} ${line}`.toLowerCase().includes(needle);
  });
  return (
    <div className="demo">
      <input
        className="dinput"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Search terminal output…"
        aria-label="Search demo scrollback"
      />
      <ul className="hlist">
        {hits.map(({ s: [sess, line], i }) => (
          <li key={i}>
            <button onClick={() => setJump(`Jumped back to ${sess} @ line ${i + 1}`)}>
              <span className="hsess">{sess}</span>
              <span className="hline">{line}</span>
            </button>
          </li>
        ))}
        {hits.length === 0 && <li className="dnote">No matches.</li>}
      </ul>
      {jump && <p className="dnote ok">{jump}</p>}
    </div>
  );
}

export function InfoDemo() {
  const [cpu, setCpu] = useState(34);
  const [mem, setMem] = useState(974);
  useEffect(() => {
    if (reduced()) return;
    const t = window.setInterval(() => {
      setCpu((c) => Math.max(4, Math.min(96, c + Math.round(Math.random() * 14 - 7))));
      setMem((m) => Math.max(800, Math.min(1200, m + Math.round(Math.random() * 40 - 20))));
    }, 1200);
    return () => window.clearInterval(t);
  }, []);
  return (
    <div className="demo">
      <p className="dmono">cwd ~/terminator · sh · pid 4120</p>
      <pre className="mini"><code>{`terminator(4120)\n├─ zsh(4121)\n└─ nvim README.md(4188)`}</code></pre>
      <div className="meter">
        <span>cpu</span>
        <div className="bar"><i style={{ width: `${cpu}%` }} /></div>
        <span className="dmono">{cpu}%</span>
      </div>
      <div className="meter">
        <span>mem</span>
        <div className="bar"><i style={{ width: `${Math.round(((mem - 800) / 400) * 100)}%` }} /></div>
        <span className="dmono">{mem} MB</span>
      </div>
    </div>
  );
}

export function NvimDemo() {
  const saved = "fn main() {\n    println!(\"hello, workspace\");\n}";
  const [text, setText] = useState(saved);
  const [closing, setClosing] = useState(false);
  const dirty = text !== saved;
  const resolve = (how: "save" | "discard" | "cancel") => {
    if (how === "cancel") return setClosing(false);
    if (how === "discard") setText(saved);
    setClosing(false);
  };
  return (
    <div className="demo">
      <div className="editor">
        <div className="ed-h">
          <span>nvim README.md</span>
          {dirty && <span className="dirty" title="Unsaved changes">●</span>}
          <span className="ed-x">
            <button onClick={() => (dirty ? setClosing(true) : setClosing(false))} title="Close editor">×</button>
          </span>
        </div>
        <textarea value={text} onChange={(e) => setText(e.target.value)} rows={4} spellCheck={false} aria-label="Demo Neovim buffer" />
        {closing && dirty && (
          <div className="guard">
            <p>Unsaved changes. Nothing is discarded silently.</p>
            <div className="abtns">
              <button className="abtn go" onClick={() => resolve("save")}>Save and close</button>
              <button className="abtn" onClick={() => resolve("discard")}>Discard changes</button>
              <button className="abtn" onClick={() => resolve("cancel")}>Cancel</button>
            </div>
          </div>
        )}
        {closing && !dirty && <p className="dnote ok">Clean buffer — closed directly.</p>}
      </div>
      <p className="dnote">Your real Neovim config loads in the app. This one is just for typing.</p>
    </div>
  );
}

function renderMd(src: string, toggle: (i: number) => void, checks: boolean[]) {
  let ci = 0;
  return src.split("\n").map((line, i) => {
    const check = line.match(/^-\s*\[( |x)\]\s*(.*)$/);
    if (check) {
      const idx = ci++;
      return (
        <label key={i} className="md-check">
          <input type="checkbox" checked={checks[idx] ?? check[1] === "x"} onChange={() => toggle(idx)} />
          {check[2]}
        </label>
      );
    }
    if (line.startsWith("## ")) return <h4 key={i}>{line.slice(3)}</h4>;
    if (line.startsWith("# ")) return <h3 key={i}>{line.slice(2)}</h3>;
    if (line.startsWith("`") && line.endsWith("`")) return <code key={i} className="md-code">{line.slice(1, -1)}</code>;
    const bold = line.split("**");
    return (
      <p key={i}>
        {bold.map((part, j) => (j % 2 === 1 ? <strong key={j}>{part}</strong> : part)) || " "}
      </p>
    );
  });
}

export function MarkdownDemo() {
  const [mode, setMode] = useState<"edit" | "preview" | "split">("split");
  const [src, setSrc] = useState("# Atlas web\n\n- [x] Auth\n- [ ] Billing\n\n**Ships** with `cargo run`");
  const initialChecks = src.split("\n").map((l) => l.includes("[x]"));
  const [checks, setChecks] = useState<boolean[]>(initialChecks);
  const toggle = (i: number) => setChecks((c) => c.map((v, j) => (j === i ? !v : v)));
  return (
    <div className="demo">
      <div className="drow">
        {(["edit", "preview", "split"] as const).map((m) => (
          <button key={m} className={mode === m ? "pill on" : "pill"} onClick={() => setMode(m)}>
            {m}
          </button>
        ))}
      </div>
      <div className={mode === "split" ? "md-split" : "md-single"}>
        {(mode === "edit" || mode === "split") && (
          <textarea value={src} onChange={(e) => setSrc(e.target.value)} rows={7} spellCheck={false} aria-label="Demo markdown source" />
        )}
        {(mode === "preview" || mode === "split") && (
          <div className="md-prev">{renderMd(src, toggle, checks)}</div>
        )}
      </div>
    </div>
  );
}

const FILES: [string, string][] = [
  ["hero.png", "image → preview"],
  ["docs.html", "html → browser tab"],
  ["README.md", "markdown → editor preview"],
  ["tone.wav", "audio → player"],
  ["notes.txt", "text → editor"],
];

export function ExplorerDemo() {
  const [q, setQ] = useState("");
  const [open, setOpen] = useState("hero.png");
  const hits = FILES.filter(([n]) => n.toLowerCase().includes(q.trim().toLowerCase()));
  const viewer = (n: string) => {
    if (n.endsWith(".png")) return "Image preview · fit to pane · press 1 for actual size.";
    if (n.endsWith(".html")) return "Browser tab · GUI-only, dies with the window.";
    if (n.endsWith(".md")) return "Markdown preview side by side with Edit.";
    if (n.endsWith(".wav")) return "Now playing in the chrome player — keeps going while you work.";
    return "Opened as text in a new editor tab.";
  };
  return (
    <div className="demo">
      <input
        className="dinput"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Search names…"
        aria-label="Search demo files"
      />
      <ul className="hlist">
        {hits.map(([n, how]) => (
          <li key={n}>
            <button className={open === n ? "on" : ""} onClick={() => setOpen(n)}>
              <span className="hsess">{n}</span>
              <span className="hline">{how}</span>
            </button>
          </li>
        ))}
        {hits.length === 0 && <li className="dnote">No files match.</li>}
      </ul>
      <p className="dnote ok">{open}: {viewer(open)}</p>
    </div>
  );
}

const GIT_GROUPS: [string, [string, string][]][] = [
  ["staged", [["workspace.rs", "M"], ["editor_close.rs", "A"]]],
  ["working", [["pty.rs", "M"], ["theme.rs", "M"]]],
  ["untracked", [["REFERENCE.md", "?"]]],
];

const MINI_DIFF = [
  [" ", "fn reconnect(sessions: &[Session]) {"],
  ["-", "    restart_all(sessions);"],
  ["+", "    attach_running(sessions); // never restart"],
  [" ", "}"],
] as [string, string][];

export function GitSidebarDemo() {
  const [open, setOpen] = useState<string | null>("workspace.rs");
  return (
    <div className="demo">
      {GIT_GROUPS.map(([g, files]) => (
        <div key={g}>
          <p className="sh">{g}</p>
          <ul className="hlist">
            {files.map(([f, st]) => (
              <li key={f}>
                <button className={open === f ? "on" : ""} onClick={() => setOpen(open === f ? null : f)}>
                  <span className="hsess">{st}</span>
                  <span className="hline">{f}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      ))}
      {open && (
        <div className="diff-mini">
          <p className="dmono">{open} — click again to close the diff</p>
          {MINI_DIFF.map(([s, l], i) => (
            <p key={i} className={s === "-" ? "dl-del" : s === "+" ? "dl-add" : "dl-ctx"}>
              {s} {l}
            </p>
          ))}
        </div>
      )}
    </div>
  );
}

const STAGED = [
  ["-", "    restart_all(sessions);"],
  ["+", "    attach_running(sessions);"],
] as [string, string][];
const WORKING = [
  ["-", "    attach_running(sessions);"],
  ["+", "    attach_running(sessions); // keep scrollback"],
  ["+", "    focus_existing_tab();"],
] as [string, string][];

export function GitDiffDemo() {
  const [which, setWhich] = useState<"staged" | "working">("staged");
  const rows = which === "staged" ? STAGED : WORKING;
  const head = which === "staged" ? "HEAD → index" : "index → disk";
  return (
    <div className="demo">
      <div className="drow">
        <button className={which === "staged" ? "pill on" : "pill"} onClick={() => setWhich("staged")}>
          staged
        </button>
        <button className={which === "working" ? "pill on" : "pill"} onClick={() => setWhich("working")}>
          working
        </button>
        <span className="dnote">{head} · read-only snapshot, repo untouched</span>
      </div>
      <div className="diff-side">
        <div>
          <p className="dmono">before</p>
          {rows.filter(([s]) => s !== "+").map(([s, l], i) => (
            <p key={i} className={s === "-" ? "dl-del" : "dl-ctx"}>{l.replace(/^ +/, "")}</p>
          ))}
        </div>
        <div>
          <p className="dmono">after</p>
          {rows.filter(([s]) => s !== "-").map(([s, l], i) => (
            <p key={i} className={s === "+" ? "dl-add" : "dl-ctx"}>{l.replace(/^ +/, "")}</p>
          ))}
        </div>
      </div>
    </div>
  );
}

export function IdeDemo() {
  const [ide, setIde] = useState(true);
  return (
    <div className="demo">
      <div className="drow">
        <button className={ide ? "pill on" : "pill"} onClick={() => setIde(true)}>IDE: Cmd+E</button>
        <button className={!ide ? "pill on" : "pill"} onClick={() => setIde(false)}>workspace</button>
      </div>
      <div className={ide ? "ide-mock on" : "ide-mock"}>
        <div className="im-ed">editor · README.md</div>
        <div className="im-term">terminal strip · dev server</div>
      </div>
      <p className="dnote">{ide ? "Editor centered, terminal strip docked below." : "Terminals in the main dock."}</p>
    </div>
  );
}

const TRACKS = ["tone-01.wav", "tone-02.wav", "icecast: synthwave"];
const STATIONS = ["synthwave fm", "lofi hip-hop", "classic rock"];

export function PlayerDemo() {
  const [tab, setTab] = useState<"playlist" | "radio">("playlist");
  const [track, setTrack] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [pos, setPos] = useState(0);
  const [shuffle, setShuffle] = useState(() => localStorage.getItem("demo-shuffle") === "1");
  const [repeat, setRepeat] = useState(() => localStorage.getItem("demo-repeat") === "1");
  useEffect(() => {
    if (!playing || reduced()) return;
    const t = window.setInterval(() => setPos((p) => (p >= 100 ? 0 : p + 2)), 200);
    return () => window.clearInterval(t);
  }, [playing]);
  const flip = (which: "shuffle" | "repeat") => {
    if (which === "shuffle") {
      setShuffle((v) => {
        localStorage.setItem("demo-shuffle", v ? "0" : "1");
        return !v;
      });
    } else {
      setRepeat((v) => {
        localStorage.setItem("demo-repeat", v ? "0" : "1");
        return !v;
      });
    }
  };
  const step = (dir: 1 | -1) => {
    if (shuffle) {
      setTrack(Math.floor(Math.random() * TRACKS.length));
    } else {
      setTrack((t) => (t + dir + TRACKS.length) % TRACKS.length);
    }
    setPos(0);
  };
  return (
    <div className="demo">
      <div className="drow">
        <button className={tab === "playlist" ? "pill on" : "pill"} onClick={() => setTab("playlist")}>playlist</button>
        <button className={tab === "radio" ? "pill on" : "pill"} onClick={() => setTab("radio")}>radio</button>
      </div>
      {tab === "playlist" ? (
        <ul className="hlist">
          {TRACKS.map((t, i) => (
            <li key={t}>
              <button className={i === track ? "on" : ""} onClick={() => { setTrack(i); setPos(0); setPlaying(true); }}>
                <span className="hsess">{i === track && playing ? "▶" : "·"}</span>
                <span className="hline">{t}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <ul className="hlist">
          {STATIONS.map((s) => (
            <li key={s}>
              <button onClick={() => { setPlaying(true); setPos(0); }}>
                <span className="hsess">fm</span>
                <span className="hline">{s}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className={`spec ${playing ? "play" : ""}`} aria-hidden="true">
        {Array.from({ length: 12 }).map((_, i) => (
          <i key={i} style={{ animationDelay: `${i * 0.09}s` }} />
        ))}
      </div>
      <div className="drow">
        <button className="abtn" onClick={() => step(-1)}>prev</button>
        <button className="abtn go" onClick={() => setPlaying((p) => !p)}>{playing ? "pause" : "play"}</button>
        <button className="abtn" onClick={() => step(1)}>next</button>
        <button className={shuffle ? "pill on" : "pill"} onClick={() => flip("shuffle")}>shuffle</button>
        <button className={repeat ? "pill on" : "pill"} onClick={() => flip("repeat")}>repeat</button>
      </div>
      <div className="meter">
        <span className="dmono">{TRACKS[track]}</span>
        <div className="bar"><i style={{ width: `${pos}%` }} /></div>
      </div>
    </div>
  );
}

const EVENTS = ["needs input", "needs permission", "completed", "failed"];
const ROUTES = ["in app", "OS", "both", "off"] as const;

export function NotifyDemo() {
  const [routes, setRoutes] = useState<Record<string, string>>({
    "needs input": "both",
    "needs permission": "both",
    completed: "in app",
    failed: "OS",
  });
  const [sound, setSound] = useState(true);
  const [burst, setBurst] = useState(false);
  return (
    <div className="demo">
      {EVENTS.map((e) => (
        <div key={e} className="nrow">
          <span>{e}</span>
          <span className="drow">
            {ROUTES.map((r) => (
              <button
                key={r}
                className={routes[e] === r ? "pill on" : "pill"}
                onClick={() => setRoutes((m) => ({ ...m, [e]: r }))}
              >
                {r}
              </button>
            ))}
          </span>
        </div>
      ))}
      <div className="drow">
        <label className="md-check">
          <input type="checkbox" checked={sound} onChange={() => setSound((v) => !v)} />
          notification sound{sound ? " (on)" : " (off)"}
        </label>
        <button className="abtn go" onClick={() => setBurst(true)}>send test burst</button>
      </div>
      {burst && (
        <p className="dnote ok">3 alerts arrived, time-gated — your phone buzzed once.</p>
      )}
    </div>
  );
}

const HOOK_AGENTS = ["Claude Code", "Codex", "OpenCode", "Muse", "Grok"];

export function HooksDemo() {
  const [installed, setInstalled] = useState<string[]>(["Claude Code"]);
  const toggle = (a: string) =>
    setInstalled((s) => (s.includes(a) ? s.filter((x) => x !== a) : [...s, a]));
  return (
    <div className="demo">
      <ul className="hlist">
        {HOOK_AGENTS.map((a) => (
          <li key={a}>
            <button onClick={() => toggle(a)}>
              <span className="hsess">{installed.includes(a) ? "✓" : "·"}</span>
              <span className="hline">{a} — {installed.includes(a) ? "installed" : "not installed"}</span>
            </button>
          </li>
        ))}
      </ul>
      <div className="drow">
        <button className="abtn" onClick={() => setInstalled(HOOK_AGENTS)}>install all</button>
        <button className="abtn" onClick={() => setInstalled([])}>remove all</button>
      </div>
      <p className="dnote">Existing config is preserved — a backup is written before any change.</p>
    </div>
  );
}

const ACCENTS = ["#3871e1", "#79baa3", "#e2c08d", "#c74e39"];

export function SettingsDemo() {
  const [density, setDensity] = useState<"comfortable" | "compact">("comfortable");
  const paint = (hex: string) =>
    document.documentElement.style.setProperty("--accent", hex);
  const flipDensity = (d: "comfortable" | "compact") => {
    setDensity(d);
    document.querySelector(".window")?.classList.toggle("compact", d === "compact");
  };
  return (
    <div className="demo">
      <p className="dmono">accent — live on this page</p>
      <div className="drow">
        {ACCENTS.map((a) => (
          <button
            key={a}
            className="swatch"
            style={{ background: a }}
            onClick={() => paint(a)}
            aria-label={`Set accent ${a}`}
            title={a}
          />
        ))}
        <button className="abtn" onClick={() => paint("#3871e1")}>reset</button>
      </div>
      <p className="dmono">density</p>
      <div className="drow">
        <button className={density === "comfortable" ? "pill on" : "pill"} onClick={() => flipDensity("comfortable")}>
          comfortable
        </button>
        <button className={density === "compact" ? "pill on" : "pill"} onClick={() => flipDensity("compact")}>
          compact
        </button>
      </div>
      <p className="dnote">Settings open in the main pane here, not a popup — same as the app.</p>
    </div>
  );
}

export function DemoFor({ id }: { id: string }) {
  switch (id) {
    case "projects": return <ProjectsDemo />;
    case "history": return <HistoryDemo />;
    case "info": return <InfoDemo />;
    case "nvim": return <NvimDemo />;
    case "markdown": return <MarkdownDemo />;
    case "explorer": return <ExplorerDemo />;
    case "git-sidebar": return <GitSidebarDemo />;
    case "git-diff": return <GitDiffDemo />;
    case "ide-mode": return <IdeDemo />;
    case "player": return <PlayerDemo />;
    case "notifications": return <NotifyDemo />;
    case "hooks": return <HooksDemo />;
    case "settings": return <SettingsDemo />;
    default: return <AgentsPane />;
  }
}

