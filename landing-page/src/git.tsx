import { Fragment, useMemo, useState, useSyncExternalStore } from "react";
import { Icon } from "./icons";
import { basename, fileIcon } from "./data";

/* ============================ shared Git demo store ============================ */
type Line = ["add" | "del", string];
type Hunk = { id: string; old: number; neu: number; lines: Line[]; state: "unstaged" | "staged" | "gone" };
type Seg = { ctx: [number, number, string] } | { hunk: Hunk };
type GitFile = { path: string; status: "M" | "U"; segs: Seg[]; checked: boolean };
type Commit = { hash: string; msg: string; author: string; when: string };

const h = (id: string, old: number, neu: number, lines: Line[]): Seg => ({ hunk: { id, old, neu, lines, state: "unstaged" } });
const c = (o: number, n: number, t: string): Seg => ({ ctx: [o, n, t] });

const seed = (): GitFile[] => [
  {
    path: "src/app.ts",
    status: "M",
    checked: false,
    segs: [
      h("a1", 1, 1, [["del", 'export const App = () => "hello";'], ["add", 'export const App = () => "hello atlas";'], ["add", "export const version = 2;"]]),
      c(2, 3, ""),
      c(3, 4, "export function route(path: string) {"),
      h("a2", 4, 5, [["del", "  return path;"], ["add", "  return path.trim();"]]),
      c(5, 6, "}"),
    ],
  },
  {
    path: "README.md",
    status: "M",
    checked: false,
    segs: [
      c(1, 1, "# Atlas web"),
      c(2, 2, ""),
      h("r1", 3, 3, [["del", "- [ ] Billing"], ["add", "- [x] Billing"]]),
      c(4, 4, "| /home | ok |"),
      h("r2", 5, 5, [["add", "| /pay | ok |"]]),
    ],
  },
  { path: "src/billing.ts", status: "U", checked: false, segs: [h("b1", 0, 1, [["add", "export const bill = () => 42;"]])] },
];

let state = {
  files: seed(),
  log: [
    { hash: "4f2c9a1", msg: "Add billing route", author: "Dana", when: "2 h ago" },
    { hash: "b81e07d", msg: "Keep sessions running after GUI exit", author: "Lee", when: "yesterday" },
    { hash: "0d6b48f", msg: "init", author: "Demo", when: "3 days ago" },
  ] as Commit[],
  note: "",
};
const subs = new Set<() => void>();
const subscribe = (f: () => void) => {
  subs.add(f);
  return () => {
    subs.delete(f);
  };
};
const snapshot = () => state;
const set = (next: Partial<typeof state>) => {
  state = { ...state, ...next };
  subs.forEach((f) => f());
};
const useGit = () => useSyncExternalStore(subscribe, snapshot);

const hunks = (f: GitFile) => f.segs.flatMap((s) => ("hunk" in s && s.hunk.state !== "gone" ? [s.hunk] : []));
const counts = (hs: Hunk[]) => {
  let add = 0;
  let del = 0;
  for (const x of hs) for (const [k] of x.lines) k === "add" ? add++ : del++;
  return { add, del };
};
/** One store update for any number of hunks. */
const setHunks = (ids: Set<string>, st: Hunk["state"]) =>
  set({
    files: state.files.map((f) => ({
      ...f,
      segs: f.segs.map((s) => ("hunk" in s && ids.has(s.hunk.id) ? { hunk: { ...s.hunk, state: st } } : s)),
    })),
  });

type Row = { f: GitFile; hs: Hunk[]; staged: boolean; unstaged: boolean };
/** One pass per store change: every file with live hunks and its staged/unstaged flags. */
const liveRows = (files: GitFile[]): Row[] =>
  files.flatMap((f) => {
    const hs = hunks(f);
    return hs.length ? [{ f, hs, staged: hs.some((x) => x.state === "staged"), unstaged: hs.some((x) => x.state === "unstaged") }] : [];
  });

/* ============================ shared diff row ============================ */
export function DiffRow({ kind, oldLn, newLn, text, staged }: { kind: "add" | "del" | "ctx"; oldLn?: number | ""; newLn?: number | ""; text: string; staged?: boolean }) {
  return (
    <div className={`diff-row${kind === "ctx" ? "" : ` ${kind}`}${staged ? " staged" : ""}`}>
      {oldLn !== undefined && <span className="ln">{oldLn}</span>}
      {newLn !== undefined && <span className="ln">{newLn}</span>}
      <span className="sg">{kind === "add" ? "+" : kind === "del" ? "-" : ""}</span>
      <span className="tx">{text}</span>
    </div>
  );
}

/* ============================ Git panel ============================ */
export function GitPanel({ onOpenDiff }: { onOpenDiff: (path: string) => void }) {
  const g = useGit();
  const [tab, setTab] = useState<"commit" | "stash" | "history">("commit");
  const [msg, setMsg] = useState("");
  const [amend, setAmend] = useState(false);
  const [q, setQ] = useState("");
  const rows = useMemo(() => liveRows(g.files), [g.files]);
  const staged = rows.filter((r) => r.staged);
  const changes = rows.filter((r) => r.f.status === "M" && r.unstaged);
  const untracked = rows.filter((r) => r.f.status === "U" && r.unstaged);
  const picked = rows.filter((r) => r.f.checked || r.staged);
  const idsOf = (rs: Row[], st?: Hunk["state"]) => new Set(rs.flatMap((r) => r.hs.filter((x) => !st || x.state === st).map((x) => x.id)));

  const commit = (push: boolean) => {
    if (!msg.trim() || picked.length === 0) {
      set({ note: !msg.trim() ? "Write a commit message first." : "Check a file or stage a hunk first." });
      return;
    }
    const pickedPaths = new Set(picked.map((r) => r.f.path));
    const hash = Math.random().toString(16).slice(2, 9);
    const head = amend ? g.log[0].hash : hash;
    set({
      files: g.files.map((f) =>
        pickedPaths.has(f.path)
          ? { ...f, checked: false, segs: f.segs.map((s) => ("hunk" in s && (f.checked || s.hunk.state === "staged") ? { hunk: { ...s.hunk, state: "gone" as const } } : s)) }
          : f,
      ),
      log: amend ? [{ ...g.log[0], msg: msg.trim(), when: "now" }, ...g.log.slice(1)] : [{ hash, msg: msg.trim(), author: "you", when: "now" }, ...g.log],
      note: `${amend ? "Amended" : "Committed"} ${picked.length} file${picked.length > 1 ? "s" : ""} as ${head}${push ? " and pushed to origin/master" : ""}.`,
    });
    setMsg("");
    setAmend(false);
  };

  const group = (title: string, rs: Row[], kind: "staged" | "unstaged") =>
    rs.length > 0 && (
      <div key={title}>
        <div className="git-group">
          <Icon name="ChevronDown" size={12} />
          <span>{title}</span>
          <span className="weak">{rs.length} file{rs.length > 1 ? "s" : ""}</span>
          <span className="grow" />
          <button className="ic-btn sm" title={kind === "staged" ? "Unstage all" : "Stage all"} onClick={() => setHunks(idsOf(rs, kind), kind === "staged" ? "unstaged" : "staged")}>
            <Icon name={kind === "staged" ? "X" : "Plus"} size={13} />
          </button>
        </div>
        {rs.map(({ f, hs }) => {
          const n = counts(hs.filter((x) => x.state === kind));
          return (
            <div key={f.path} className="tree-row indent" onClick={() => onOpenDiff(f.path)} title={`Open diff for ${f.path}`}>
              <input
                type="checkbox"
                className="chk"
                checked={kind === "staged" || f.checked}
                onClick={(e) => e.stopPropagation()}
                onChange={() => set({ files: g.files.map((x) => (x.path === f.path ? { ...x, checked: !x.checked } : x)) })}
                aria-label={`Include ${f.path}`}
              />
              <span className="row-ic"><Icon name={fileIcon(f.path)} size={15} /></span>
              <span className="row-label" style={{ color: f.status === "U" ? "var(--git-untracked)" : "var(--git-modified)" }}>{basename(f.path)}</span>
              <span className="row-trail mono">
                {n.add ? `+${n.add} ` : ""}
                {n.del ? `-${n.del} ` : ""}
                {f.status}
              </span>
            </div>
          );
        })}
      </div>
    );

  return (
    <>
      <div className="git-head">
        <span className="weak">Last known directory</span>
      </div>
      <div className="git-tabs">
        {(["commit", "stash", "history"] as const).map((t) => (
          <button key={t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>
            {t[0].toUpperCase() + t.slice(1)}
          </button>
        ))}
        <span className="grow" />
        <span>master</span>
        <Icon name="Menu" size={13} />
      </div>
      {tab === "commit" && (
        <>
          <div className="git-tools">
            <button className="ic-btn sm" title="Refresh" onClick={() => set({ note: "Up to date." })}><Icon name="RefreshCw" size={14} /></button>
            <button className="ic-btn sm" title="Stage all" onClick={() => setHunks(idsOf(rows), "staged")}><Icon name="Plus" size={14} /></button>
            <button className="ic-btn sm" title="Stash changes" onClick={() => setTab("stash")}><Icon name="Archive" size={14} /></button>
            <button className="ic-btn sm" title="Show diff" onClick={() => rows[0] && onOpenDiff(rows[0].f.path)}><Icon name="Eye" size={14} /></button>
            <span className="vsep" />
            <button className="ic-btn sm" title="Group by folder"><Icon name="List" size={14} /></button>
            <button className="ic-btn sm" title="Expand all"><Icon name="ChevronsUpDown" size={14} /></button>
            <button className="ic-btn sm" title="Collapse all"><Icon name="ChevronsDownUp" size={14} /></button>
          </div>
          <div className="side-scroll">
            {group("Staged", staged, "staged")}
            {group("Changes", changes, "unstaged")}
            {group("Untracked Files", untracked, "unstaged")}
            {rows.length === 0 && <div className="agent-empty">Working tree clean.</div>}
            {g.note && <div className="git-note">{g.note}</div>}
          </div>
          <div className="git-commit">
            <label><input type="checkbox" className="chk" checked={amend} onChange={() => setAmend((a) => !a)} /> Amend</label>
            <label><input type="checkbox" className="chk" /> Sign-off</label>
            <textarea className="git-msg" value={msg} onChange={(e) => setMsg(e.target.value)} placeholder="Commit Message" aria-label="Commit message" />
            <div className="git-actions">
              <button className="small-btn go" onClick={() => commit(false)}>Commit</button>
              <button className="small-btn" onClick={() => commit(true)}>Commit and Push...</button>
            </div>
          </div>
        </>
      )}
      {tab === "stash" && <div className="agent-empty">No stashes. Stash saves the working tree and index, then restores them on Pop.</div>}
      {tab === "history" && (
        <>
          <div className="side-head">
            <div className="field">
              <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Filter: --grep, --author, or hash" aria-label="Filter commits" />
            </div>
          </div>
          <div className="side-scroll">
            {g.log
              .filter((x) => !q || `${x.hash} ${x.msg} ${x.author}`.toLowerCase().includes(q.toLowerCase()))
              .map((x) => (
                <div key={x.hash + x.msg} className="log-row" title="Open the revision diff">
                  <span className="log-dot" />
                  <span className="row-label">{x.msg}</span>
                  <span className="weak mono">{x.hash}</span>
                  <span className="weak">{x.author} · {x.when}</span>
                </div>
              ))}
          </div>
        </>
      )}
    </>
  );
}

/* ============================ Diff view ============================ */
export function DiffView({ path, onClose, onFile }: { path: string; onClose: () => void; onFile: (path: string) => void }) {
  const g = useGit();
  const [side, setSide] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);
  const rows = useMemo(() => liveRows(g.files), [g.files]);
  const idx = rows.findIndex((r) => r.f.path === path);
  const row = rows[idx];
  const live = row ? row.hs.filter((x) => x.state === "unstaged").length : 0;
  const step = (d: number) => rows.length && onFile(rows[(Math.max(idx, 0) + d + rows.length) % rows.length].f.path);

  const hunkRows = (x: Hunk) => {
    let o = x.old;
    let n = x.neu;
    return x.lines.map(([k, t], i) => <DiffRow key={i} kind={k} oldLn={k === "del" ? o++ : ""} newLn={k === "add" ? n++ : ""} text={t} staged={x.state === "staged"} />);
  };

  return (
    <>
      <div className="diff-bar">
        <button className="ic-btn sm" title="Show files"><Icon name="ListTree" size={14} /></button>
        <button className="ic-btn sm" title="Edit source"><Icon name="Pencil" size={14} /></button>
        <button className="ic-btn sm" title="Previous change"><Icon name="ArrowUp" size={14} /></button>
        <button className="ic-btn sm" title="Next change"><Icon name="ArrowDown" size={14} /></button>
        <span className="vsep" />
        <button className="ic-btn sm" title="Previous file" onClick={() => step(-1)}><Icon name="ArrowLeft" size={14} /></button>
        <span className="weak">{idx + 1}/{rows.length} files</span>
        <button className="ic-btn sm" title="Next file" onClick={() => step(1)}><Icon name="ArrowRight" size={14} /></button>
        <span className="grow" />
        <span className="weak">{live ? `Change 1 of ${live}` : "No changes"}</span>
        <button className={`ic-btn sm${side ? "" : " on"}`} title="Unified" onClick={() => setSide(false)}><Icon name="Rows2" size={14} /></button>
        <button className={`ic-btn sm${side ? " on" : ""}`} title="Side by side" onClick={() => setSide(true)}><Icon name="Columns2" size={14} /></button>
        <button className="ic-btn sm" title="Close" onClick={onClose}><Icon name="X" size={14} /></button>
      </div>
      <div className="diff-title">
        Index <span className="weak">→</span> Working tree <span className="weak">Editable</span> <span className="weak mono">{path}</span>
      </div>
      <div className={`diff-body gutter${side ? " side" : ""}`}>
        {!row ? (
          <div className="agent-empty">No changes left in this file.</div>
        ) : (
          row.f.segs.map((s, i) => {
            if ("ctx" in s) return <DiffRow key={i} kind="ctx" oldLn={s.ctx[0]} newLn={s.ctx[1]} text={s.ctx[2]} />;
            const x = s.hunk;
            if (x.state === "gone") return null;
            return (
              <Fragment key={x.id}>
                <div className="hunk">
                  <div className="hunk-gutter">
                    {x.state === "staged" ? (
                      <button className="ic-btn" title="Unstage hunk" onClick={() => setHunks(new Set([x.id]), "unstaged")}><Icon name="X" size={13} /></button>
                    ) : (
                      <>
                        <button className="ic-btn" title="Stage hunk" onClick={() => setHunks(new Set([x.id]), "staged")}><Icon name="Plus" size={13} /></button>
                        <button className="ic-btn" title="Discard hunk…" onClick={() => setConfirm(x.id)}><Icon name="Eraser" size={13} /></button>
                      </>
                    )}
                  </div>
                  <div className="hunk-lines">{hunkRows(x)}</div>
                </div>
                {confirm === x.id && (
                  <div className="diff-confirm">
                    Discard this change? It cannot be undone.
                    <button className="small-btn danger" onClick={() => { setHunks(new Set([x.id]), "gone"); setConfirm(null); }}>Discard</button>
                    <button className="small-btn" onClick={() => setConfirm(null)}>Cancel</button>
                  </div>
                )}
              </Fragment>
            );
          })
        )}
      </div>
    </>
  );
}
