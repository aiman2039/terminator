import type * as React from "react";
import { useEffect, useMemo, useState } from "react";
import { Icon } from "./icons";
import { DOWNLOAD, REPO, diffFor } from "./data";

/* ============================ Tour (marketing) ============================ */
export function TourView({ onClose }: { onClose: () => void }) {
  return (
    <>
      <div className="md-toolbar">
        <span className="md-tab">Tour.md</span>
        <span className="md-tab on">Preview</span>
        <span style={{ flex: 1 }} />
        <button className="md-tab" title="Refresh">
          <Icon name="RefreshCw" size={13} />
        </button>
        <button className="md-tab" onClick={onClose} title="Close pane">
          <Icon name="X" size={14} />
        </button>
      </div>
      <div className="md-preview">
        <p className="tag">// welcome</p>
        <h1>Terminals, AI agents, and project files in one place.</h1>
        <p>
          Terminator is a native terminal workspace for macOS and Linux. Projects own tabs and split
          layouts, sessions live in a background daemon, and hooks tell you the moment an agent needs
          you. Everything below runs in this page — right inside the real layout.
        </p>
        <div className="hero-btns">
          <a className="btn" href={DOWNLOAD}>
            <Icon name="ArrowDown" size={15} /> Download for macOS / Linux
          </a>
          <a className="btn ghost" href={REPO}>
            Star on GitHub
          </a>
        </div>
        <p className="tag">// try it</p>
        <p>
          Click around the chrome. The <strong>left sidebar</strong> lists projects and live sessions,
          the <strong>right sidebar</strong> is the tool panel (Explorer, Agents, Git, History, Info),
          and the <strong>terminal</strong> in the next pane is a working demo shell. Press{" "}
          <span className="kbd">⌘K</span> for the command palette, <span className="kbd">⌘E</span> for
          IDE mode, <span className="kbd">⌘T</span> for a new terminal tab.
        </p>
        <hr />
        <div className="cards">
          {[
            ["Persistent sessions", "Close the app; shells and editors keep running in the daemon."],
            ["Agent inbox", "Needs input, needs permission, done, failed — from real hooks."],
            ["Git reviews", "Native diffs, read-only snapshots. CodeDiff with Neovim if you want it."],
            ["Command palette", "Jump across projects, sessions, files, and settings."],
          ].map(([h, p]) => (
            <div className="card" key={h}>
              <h3>{h}</h3>
              <p>{p}</p>
            </div>
          ))}
        </div>
        <h2 id="workspace">One workspace per project</h2>
        <p>
          Each project owns its tabs and split layouts. Right-click a terminal to split in any
          direction. Sessions live in a background daemon, so closing the app never kills them.
        </p>
        <ul>
          <li>Reconnect to running sessions after restart</li>
          <li>Sort and filter projects</li>
          <li>Rename, reorder, and close tabs in bulk</li>
        </ul>
        <h2 id="agents">Know the moment an agent needs you</h2>
        <p>
          Hooks for Claude Code, Codex, OpenCode, Muse, and Grok report real lifecycle events. One
          card per agent run, with go, snooze, and dismiss. Switch the right sidebar to "Agents" to
          see it.
        </p>
        <h2 id="editing">Files open the right way</h2>
        <p>
          Images preview, HTML opens in a browser tab, Markdown previews, audio plays, code opens in
          your own Neovim. Open as text bypasses any viewer. Try the files in the Explorer.
        </p>
        <h2 id="git">Native diffs, never destructive</h2>
        <p>
          Side-by-side by default, CodeDiff in Neovim in Settings. Reviews are read-only snapshots:
          HEAD to index, index to disk. They never touch your repo.
        </p>
        <h2 id="media">A built-in player. And internet radio.</h2>
        <p>
          Open a track from the Explorer and the Chrome player appears. Named playlists, shuffle,
          repeat, an equalizer, and a bundled Icecast/Shoutcast catalog.
        </p>
        <h2 id="notify">Alerts everywhere</h2>
        <p>
          In app, on the desktop, on the macOS menu bar, or pushed to your phone through ntfy. Bursts
          are time-gated so your phone buzzes once. Prompt text is never uploaded.
        </p>
        <div className="hero-btns">
          <a className="btn" href={DOWNLOAD}>
            <Icon name="ArrowDown" size={15} /> Download the latest release
          </a>
          <a className="btn ghost" href={REPO}>
            Read the docs
          </a>
        </div>
        <p className="weak">MIT · Rust · Native egui · No Electron</p>
      </div>
    </>
  );
}

/* ============================ Code editor ============================ */
export function EditorView({ path, content }: { path: string; content?: string }) {
  const [text, setText] = useState(content ?? "");
  const lines = text.split("\n").length;
  return (
    <div className="nvim" style={{ height: "100%" }}>
      <div className="nvim-body">
        <div className="nvim-lines mono">
          {text.split("\n").map((_, i) => (
            <div key={i}>{i + 1}</div>
          ))}
          {Array.from({ length: Math.max(0, 24 - text.split("\n").length) }).map((_, i) => (
            <div key={`f${i}`}>~</div>
          ))}
        </div>
        <textarea
          className="nvim-code"
          value={text}
          spellCheck={false}
          onChange={(e) => setText(e.target.value)}
          aria-label={`Neovim buffer for ${path}`}
        />
      </div>
      <div className="nvim-status mono">
        <span>{path}</span>
        <span>{`${Math.min(lines, 1)},1`}</span>
        <span>All</span>
      </div>
    </div>
  );
}

/* ============================ Markdown ============================ */
function renderMd(src: string) {
  const out: React.ReactNode[] = [];
  const lines = src.split("\n");
  let i = 0;
  let key = 0;
  let table: string[][] | null = null;
  const flush = () => {
    if (table) {
      const [head, ...rows] = table;
      out.push(
        <table key={`t${key++}`}>
          <thead>
            <tr>
              {head.map((c, j) => (
                <th key={j}>{c}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>
                {r.map((c, ci) => (
                  <td key={ci}>{c}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>,
      );
      table = null;
    }
  };
  const inline = (s: string) =>
    s
      .split(/(\*\*[^*]+\*\*|`[^`]+`)/g)
      .filter(Boolean)
      .map((p, j) =>
        p.startsWith("**") ? (
          <strong key={j}>{p.slice(2, -2)}</strong>
        ) : p.startsWith("`") ? (
          <code key={j}>{p.slice(1, -1)}</code>
        ) : (
          p
        ),
      );
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim().startsWith("| ")) {
      table = table ?? [];
      table.push(line.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim()));
      i++;
      continue;
    }
    flush();
    const check = line.match(/^-\s*\[( |x)\]\s*(.*)$/);
    if (check) {
      out.push(
        <label className="md-check" key={key++} style={{ display: "flex", gap: 8, margin: "2px 0" }}>
          <input type="checkbox" defaultChecked={check[1] === "x"} />
          {check[2]}
        </label>,
      );
      i++;
      continue;
    }
    if (line.startsWith("### ")) out.push(<h3 key={key++}>{inline(line.slice(4))}</h3>);
    else if (line.startsWith("## ")) out.push(<h2 key={key++}>{inline(line.slice(3))}</h2>);
    else if (line.startsWith("# ")) out.push(<h1 key={key++}>{inline(line.slice(2))}</h1>);
    else if (line.startsWith("- ")) out.push(<li key={key++}>{inline(line.slice(2))}</li>);
    else if (line === "---") out.push(<hr key={key++} />);
    else if (line.trim() === "") out.push(<div key={key++} style={{ height: 8 }} />);
    else out.push(<p key={key++}>{inline(line)}</p>);
    i++;
  }
  flush();
  return out;
}

export function MarkdownView({ path, content, onClose }: { path: string; content: string; onClose: () => void }) {
  const [mode, setMode] = useState<"edit" | "preview" | "split">("split");
  const [src, setSrc] = useState(content);
  return (
    <>
      <div className="md-toolbar">
        <span className="md-tab">{path.split("/").pop()}</span>
        {(["edit", "preview", "split"] as const).map((m) => (
          <button key={m} className={`md-tab${mode === m ? " on" : ""}`} onClick={() => setMode(m)}>
            {m[0].toUpperCase() + m.slice(1)}
          </button>
        ))}
        <span style={{ flex: 1 }} />
        <button className="md-tab" title="Refresh">
          <Icon name="RefreshCw" size={13} />
        </button>
        <button className="md-tab" onClick={onClose} title="Close pane">
          <Icon name="X" size={14} />
        </button>
      </div>
      <div className="md-split">
        {(mode === "edit" || mode === "split") && (
          <div className="md-editor" style={{ display: "flex", minHeight: 0 }}>
            <div className="nvim-lines mono">
              {src.split("\n").map((_, i) => (
                <div key={i}>{i + 1}</div>
              ))}
              {Array.from({ length: Math.max(0, 22 - src.split("\n").length) }).map((_, i) => (
                <div key={`f${i}`}>~</div>
              ))}
            </div>
            <textarea style={{ flex: 1, minWidth: 0 }} value={src} spellCheck={false} onChange={(e) => setSrc(e.target.value)} aria-label="Markdown source" />
          </div>
        )}
        {(mode === "preview" || mode === "split") && <div className="md-preview">{renderMd(src)}</div>}
      </div>
    </>
  );
}

/* ============================ Diff ============================ */
export function DiffView({ path, staged, onClose }: { path: string; staged: boolean; onClose: () => void }) {
  const [side, setSide] = useState(true);
  const rows = diffFor(path, staged);
  return (
    <>
      <div className="diff-bar">
        <span className="mono">
          {path} <span className="weak">M</span>
        </span>
        <span style={{ flex: 1 }} />
        <button className="small-btn" onClick={() => setSide((s) => !s)}>
          {side ? "Side by side" : "Inline"}
        </button>
        <span className="weak" style={{ fontFamily: '"JetBrains Mono", monospace', fontSize: 10 }}>
          ···
        </span>
        <button className="md-tab" onClick={onClose} title="Close pane">
          <Icon name="X" size={14} />
        </button>
      </div>
      <div className="diff-body">
        {side ? (
          <div className="diff-side">
            <div>
              {rows
                .filter(([k]) => k !== "add")
                .map(([k, t], i) => (
                  <div key={i} className={`diff-row ${k === "del" ? "del" : k === "hunk" ? "hunk" : ""}`}>
                    <span className="ln">{k === "hunk" ? "" : i}</span>
                    <span className="tx">{t}</span>
                  </div>
                ))}
            </div>
            <div>
              {rows
                .filter(([k]) => k !== "del")
                .map(([k, t], i) => (
                  <div key={i} className={`diff-row ${k === "add" ? "add" : k === "hunk" ? "hunk" : ""}`}>
                    <span className="ln">{k === "hunk" ? "" : i}</span>
                    <span className="tx">{t}</span>
                  </div>
                ))}
            </div>
          </div>
        ) : (
          rows.map(([k, t], i) => (
            <div key={i} className={`diff-row ${k === "add" ? "add" : k === "del" ? "del" : k === "hunk" ? "hunk" : ""}`}>
              <span className="ln">{k === "hunk" ? "" : i}</span>
              <span className="tx">{k === "add" ? "+" : k === "del" ? "-" : " "}{t}</span>
            </div>
          ))
        )}
      </div>
    </>
  );
}

/* ============================ Image ============================ */
export function ImageView({ path }: { path: string }) {
  return (
    <div className="img-prev">
      <div className="checker">
        <Icon name="FileImage" size={28} />
        <span style={{ marginLeft: 10 }}>{path}</span>
      </div>
    </div>
  );
}

/* ============================ Settings ============================ */
const SECTIONS = [
  { id: "appearance", name: "Appearance", icon: "Settings2" },
  { id: "editor", name: "Terminal & Editor", icon: "Terminal" },
  { id: "notifications", name: "Notifications", icon: "Bell" },
  { id: "history", name: "History", icon: "History" },
  { id: "shortcuts", name: "Shortcuts", icon: "SquareDashed" },
  { id: "hooks", name: "Agent Hooks", icon: "GitBranch" },
  { id: "updates", name: "Updates", icon: "RefreshCw" },
] as const;

const ACCENTS = ["#3871e1", "#79baa3", "#e2c08d", "#c74e39", "#5b9dff"];

export function SettingsView({
  accent,
  setAccent,
  compact,
  setCompact,
}: {
  accent: string;
  setAccent: (a: string) => void;
  compact: boolean;
  setCompact: (c: boolean) => void;
}) {
  const [section, setSection] = useState<string>("appearance");
  const [query, setQuery] = useState("");
  const hits = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? SECTIONS.filter((s) => s.name.toLowerCase().includes(q)) : SECTIONS;
  }, [query]);
  return (
    <div className="settings">
      <div className="settings-body">
        <div className="settings-nav">
          <div className="field" style={{ margin: "0 4px 8px" }}>
            <Icon name="Search" size={13} />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search settings" aria-label="Search settings" />
          </div>
          {hits.map((s) => (
            <button key={s.id} className={section === s.id ? "on" : ""} onClick={() => setSection(s.id)}>
              <Icon name={s.icon} size={15} />
              {s.name}
            </button>
          ))}
        </div>
        <div className="settings-main">
          {section === "appearance" && (
            <>
              <h2>Appearance</h2>
              <p className="desc">Preview colors and borders. Apply saves your changes.</p>
              <div className="set-row">
                <div className="lbl">Theme</div>
                <div>
                  <div className="seg">
                    <button className="on">Current saved</button>
                    <button>Terminator dark</button>
                    <button>High contrast</button>
                  </div>
                  <div className="hint">Presets fill colors. Advanced hex stays available below.</div>
                </div>
              </div>
              <div className="set-row">
                <div className="lbl">Accent</div>
                <div>
                  <div className="seg">
                    {ACCENTS.map((a) => (
                      <button key={a} onClick={() => setAccent(a)} title={a}>
                        <span style={{ display: "inline-block", width: 34, height: 18, background: a, borderRadius: 3, verticalAlign: "middle" }} />
                      </button>
                    ))}
                  </div>
                  <div className="hint">Selection color is derived from the accent.</div>
                </div>
              </div>
              <div className="set-row">
                <div className="lbl">Density</div>
                <div>
                  <div className="seg">
                    <button className={!compact ? "on" : ""} onClick={() => setCompact(false)}>
                      Comfortable
                    </button>
                    <button className={compact ? "on" : ""} onClick={() => setCompact(true)}>
                      Compact
                    </button>
                  </div>
                  <div className="hint">Compact shortens sidebar rows and control height.</div>
                </div>
              </div>
              <div className="set-row">
                <div className="lbl" />
                <div>
                  <button className="small-btn" onClick={() => { setAccent("#3871e1"); setCompact(false); }}>
                    Reset appearance
                  </button>
                </div>
              </div>
              {["Interface", "Terminal", "Git status", "Agent status"].map((g) => (
                <div key={g} style={{ padding: "8px 0", color: "var(--secondary)" }}>
                  <span style={{ marginRight: 8 }}>▸</span>
                  {g}
                </div>
              ))}
            </>
          )}
          {section === "editor" && <SimpleSection title="Terminal & Editor" blurb="Shell, editor, and external program. Paths are picked, not typed." rows={[["Shell", "Automatic (zsh)"], ["Editor", "Embedded Neovim"], ["External editor", "System default"], ["Diff viewer", "Native"]]} />}
          {section === "notifications" && <SimpleSection title="Notifications" blurb="Per event: in app, OS when unfocused, both, or off. Add a sound, the menu-bar badge, or ntfy." rows={[["Needs input", "both"], ["Needs permission", "both"], ["Completed", "in app"], ["Failed", "OS"], ["Notification sound", "on"], ["ntfy channel", "—"]]} />}
          {section === "history" && <SimpleSection title="History" blurb="Retention and search for saved scrollback." rows={[["Retention", "90 days"], ["Max sessions", "500"], ["Search scrollback", "on"]]} />}
          {section === "shortcuts" && <ShortcutsSection />}
          {section === "hooks" && <HooksSection />}
          {section === "updates" && <SimpleSection title="Updates" blurb="Check for a new release once a minute. A newly built GUI never replaces a running daemon." rows={[["Automatic update checks", "on"], ["Current version", "2.4.0"], ["Channel", "stable"]]} />}
        </div>
      </div>
      <div className="settings-foot">
        <button className="small-btn">Apply</button>
        <button className="small-btn">Cancel</button>
      </div>
    </div>
  );
}

function SimpleSection({ title, blurb, rows }: { title: string; blurb: string; rows: [string, string][] }) {
  return (
    <>
      <h2>{title}</h2>
      <p className="desc">{blurb}</p>
      {rows.map(([k, v]) => (
        <div className="set-row" key={k}>
          <div className="lbl">{k}</div>
          <div className="ctl weak">{v}</div>
        </div>
      ))}
    </>
  );
}

function ShortcutsSection() {
  const rows: [string, string][] = [
    ["New terminal", "⌘T"],
    ["Open file", "⌘O"],
    ["Split right", "⌘D"],
    ["Split down", "⇧⌘D"],
    ["Next pane", "⌘]"],
    ["Command palette", "⌘K"],
    ["Settings", "⌘,"],
    ["Toggle IDE mode", "⌘E"],
    ["Toggle sidebar", "⌘B"],
  ];
  return (
    <>
      <h2>Shortcuts</h2>
      <p className="desc">Click a row and press a chord. Conflicts block Apply.</p>
      {rows.map(([k, v]) => (
        <div className="set-row" key={k}>
          <div className="lbl">{k}</div>
          <div className="ctl">
            <span className="kbd">{v}</span>
          </div>
        </div>
      ))}
    </>
  );
}

function HooksSection() {
  const agents = [
    ["claude", "AgentClaude"],
    ["codex", "AgentCodex"],
    ["opencode", "AgentOpencode"],
    ["muse", "AgentMuse"],
    ["grok", "AgentGrok"],
  ];
  return (
    <>
      <h2>Agent Hooks</h2>
      <p className="desc">Agents are launched manually. Install hooks so Terminator can show waiting and done states.</p>
      {agents.map(([a, brand]) => (
        <div className="hook-row" key={a}>
          <span className="name">{a}</span>
          <Icon name={brand} size={15} />
          <span className="weak">
            <span className="dot running" style={{ display: "inline-block", marginRight: 6 }} />
            Detected · ~/bin/{a}
          </span>
          <span className="grow" />
          <span className="weak">Configured</span>
          <button className="small-btn">Repair</button>
          <button className="small-btn">Remove</button>
        </div>
      ))}
      <p className="weak" style={{ marginTop: 10 }}>
        See docs/INTEGRATIONS.md for event limitations.
      </p>
    </>
  );
}

/* ============================ Player ============================ */
const TRACKS = ["pulse.wav", "hum.wav", "chime.wav"];
const BANDS = ["pre", "60", "170", "310", "600", "1k", "3k", "6k", "12k", "14k", "16k"];

export function PlayerView() {
  const [playing, setPlaying] = useState(false);
  const [track, setTrack] = useState(0);
  const [pos, setPos] = useState(0);
  const [vol, setVol] = useState(68);
  const [eq, setEq] = useState(true);
  const [shuffle, setShuffle] = useState(false);
  const [repeat, setRepeat] = useState(false);
  useEffect(() => {
    if (!playing) return;
    const t = window.setInterval(() => setPos((p) => (p >= 100 ? 0 : p + 1.5)), 220);
    return () => window.clearInterval(t);
  }, [playing]);
  const step = (d: number) => {
    setTrack((t) => (shuffle ? Math.floor(Math.random() * TRACKS.length) : (t + d + TRACKS.length) % TRACKS.length));
    setPos(0);
  };
  return (
    <div className="player">
      <div className="player-head">
        <h2>Player</h2>
      </div>
      <div className="player-time">
        <span className="elapsed">{playing ? "0:12" : "0:00"}</span>
        <span className="grow" />
        <span>Live</span>
      </div>
      <div className="scrub">
        <i style={{ width: `${pos}%` }} />
        <span className="knob" style={{ left: `${pos}%` }} />
      </div>
      <div className="spec play">
        {Array.from({ length: 48 }).map((_, i) => (
          <i key={i} style={{ animationDelay: `${(i % 12) * 0.06}s` }} />
        ))}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "0 14px" }}>
        <span style={{ fontSize: 12, color: "var(--secondary)" }}>Vol</span>
        <div className="vol" style={{ flex: 1, margin: 0 }}>
          <i style={{ width: `${vol}%` }} />
          <span className="knob" style={{ left: `${vol}%` }} />
        </div>
      </div>
      <div className="transport">
        <button onClick={() => step(-1)} title="Previous">
          <Icon name="SkipBack" size={16} />
        </button>
        <button onClick={() => setPlaying((p) => !p)} title={playing ? "Pause" : "Play"}>
          <Icon name={playing ? "Pause" : "Play"} size={16} />
        </button>
        <button onClick={() => setPlaying(false)} title="Stop">
          <Icon name="Square" size={14} />
        </button>
        <button onClick={() => step(1)} title="Next">
          <Icon name="SkipForward" size={16} />
        </button>
        <button title="Playlist" className="on">
          <Icon name="ListMusic" size={15} />
        </button>
        <button
          className={shuffle ? "on" : ""}
          onClick={() => setShuffle((s) => !s)}
          title="Shuffle"
        >
          <Icon name="Shuffle" size={15} />
        </button>
        <button className={repeat ? "on" : ""} onClick={() => setRepeat((r) => !r)} title="Repeat">
          <Icon name="Repeat" size={15} />
        </button>
        <button title="Radio">
          <Icon name="Radio" size={15} />
        </button>
      </div>
      <div className="eq-row">
        <span className="cap">Equalizer</span>
        <div className="seg">
          <button className={eq ? "on" : ""} onClick={() => setEq(true)}>
            On
          </button>
          <button>Flat</button>
          <button>Bass</button>
          <button>Treble</button>
        </div>
      </div>
      <div className="eq-bands">
        {BANDS.map((b, i) => (
          <div className="eq-band" key={b}>
            <div className="track">
              <span className="thumb" style={{ top: `${50 - (i % 3) * 8}%` }} />
            </div>
            <span>{b}</span>
          </div>
        ))}
      </div>
      <div className="playlist">
        {TRACKS.map((t, i) => (
          <div key={t} className={`pl-track${i === track ? " on" : ""}`} onClick={() => { setTrack(i); setPos(0); setPlaying(true); }}>
            <span className="no">{i + 1}.</span>
            <span className="grow">{t}</span>
            <span className="dur">--:--</span>
          </div>
        ))}
      </div>
      <div className="playlist-foot">
        <button className="small-btn">Add</button>
        <button className="small-btn">Rem</button>
        <button className="small-btn">Sel</button>
        <button className="small-btn">Misc</button>
        <button className="small-btn">List</button>
        <span className="grow" />
        <span className="weak mono">{playing ? "0:12/0:37" : "0:00/--:--"}</span>
      </div>
    </div>
  );
}
