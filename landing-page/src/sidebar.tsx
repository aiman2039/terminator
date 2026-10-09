import type * as React from "react";
import { useMemo, useState } from "react";
import { Icon } from "./icons";
import {
  agentCards,
  fileTree,
  fileIcon,
  gitGroups,
  historyGroups,
  sessions,
  systemInfo,
  type AgentCard,
  type FileNode,
  type Project,
  type Session,
} from "./data";

/* ============================ Left projects tree ============================ */
export function ProjectsTree({
  projects,
  selected,
  onSelect,
  onOpenSession,
  onNewTerminal,
}: {
  projects: Project[];
  selected: string;
  onSelect: (id: string) => void;
  onOpenSession: (id: string) => void;
  onNewTerminal: () => void;
}) {
  const [filter, setFilter] = useState("");
  const [expanded, setExpanded] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(projects.map((p) => [p.id, true])),
  );
  const list = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return q ? projects.filter((p) => p.name.toLowerCase().includes(q)) : projects;
  }, [projects, filter]);

  return (
    <>
      <div className="side-head">
        <div className="field">
          <Icon name="Search" size={13} />
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter projects" aria-label="Filter projects" />
        </div>
        <button className="ic-btn" title="New project" onClick={onNewTerminal}>
          <Icon name="Plus" size={15} />
        </button>
        <button className="ic-btn" title="Worktrees">
          <Icon name="GitBranch" size={15} />
        </button>
        <button className="ic-btn" title="Sort projects">
          <Icon name="ArrowDownWideNarrow" size={15} />
        </button>
      </div>
      <div className="side-scroll">
        {list.map((p) => {
          const mine = sessions.filter((s) => s.projectId === p.id);
          const live = mine.filter((s) => s.visible).length;
          const isExp = expanded[p.id];
          return (
            <div className="proj-group" key={p.id}>
              <div className={`tree-row${selected === p.id ? " on" : ""}`} onClick={() => onSelect(p.id)} title={`${p.name}\n${p.path}`}>
                <button
                  className="row-chev"
                  onClick={(e) => {
                    e.stopPropagation();
                    setExpanded((x) => ({ ...x, [p.id]: !isExp }));
                  }}
                  title="Expand or collapse project"
                >
                  <Icon name={isExp ? "ChevronDown" : "ChevronRight"} size={12} />
                </button>
                <span className="row-ic">
                  <Icon name={isExp ? "FolderOpen" : "Folder"} size={15} />
                </span>
                <span className="row-label">{p.name}</span>
                <span className="row-trail">{live}</span>
              </div>
              {isExp &&
                mine.map((s) => (
                  <div
                    key={s.id}
                    className="tree-row indent"
                    onClick={() => onOpenSession(s.id)}
                    title={`${s.label}\n${s.cwd}\n${s.status}`}
                  >
                    <span className="row-ic">
                      {s.agent ? (
                        <Icon name={s.agent} size={15} />
                      ) : (
                        <Icon name={s.kind === "editor" ? "FileCode" : "Terminal"} size={15} />
                      )}
                    </span>
                    <span className="row-label">{s.label}</span>
                    <span className="row-trail">
                      {s.agent && <span className={`dot ${s.status}`} style={{ display: "inline-block", marginRight: 6 }} />}
                      {!s.visible ? "background" : ""}
                    </span>
                  </div>
                ))}
            </div>
          );
        })}
        <button className="side-new" onClick={onNewTerminal}>
          <Icon name="Plus" size={14} /> new terminal
        </button>
      </div>
    </>
  );
}

/* ============================ Right: Explorer ============================ */
export function ExplorerPanel({
  onOpenFile,
  root,
}: {
  onOpenFile: (f: FileNode) => void;
  root: string;
}) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<"names" | "contents">("names");
  const [showIgnored, setShowIgnored] = useState(false);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({ src: true });

  const matches = (node: FileNode): boolean => {
    if (!query.trim()) return true;
    const q = query.trim().toLowerCase();
    if (node.dir) return (node.children ?? []).some(matches);
    return node.name.toLowerCase().includes(q);
  };

  const rows: React.ReactNode[] = [];
  const walk = (nodes: FileNode[], depth: number) => {
    for (const n of nodes) {
      if (!matches(n)) continue;
      if (n.dir) {
        const open = expanded[n.path];
        rows.push(
          <div key={n.path} className="tree-row" style={{ paddingLeft: 6 + depth * 14 }} onClick={() => setExpanded((e) => ({ ...e, [n.path]: !open }))}>
            <span className="row-ic" style={{ color: n.status === "M" ? "var(--git-modified)" : "var(--icon)" }}>
              <Icon name={open ? "FolderOpen" : "Folder"} size={15} />
            </span>
            <span className="row-label">{n.name}</span>
            {n.status && <span className="marker" style={{ color: "var(--git-modified)" }}>{n.status}</span>}
          </div>,
        );
        if (open) walk(n.children ?? [], depth + 1);
      } else {
        const color =
          n.status === "M" ? "var(--git-modified)" : n.status === "U" ? "var(--git-untracked)" : "var(--text)";
        rows.push(
          <div
            key={n.path}
            className="tree-row"
            style={{ paddingLeft: 6 + depth * 14 }}
            onClick={() => onOpenFile(n)}
            title={`${n.path}\nOpen in the default viewer`}
          >
            <span className="row-ic">
              <Icon name={fileIcon(n.name)} size={15} />
            </span>
            <span className="row-label">{n.name}</span>
            {n.status && <span className="marker" style={{ color }}>{n.status}</span>}
          </div>,
        );
      }
    }
  };
  walk(fileTree, 0);

  return (
    <>
      <div className="side-head">
        <button className="ic-btn" title="New file">
          <Icon name="File" size={15} />
        </button>
        <button className="ic-btn" title="New folder">
          <Icon name="Folder" size={15} />
        </button>
        <button className="ic-btn" title="Collapse all">
          <Icon name="ChevronsDownUp" size={15} />
        </button>
        <button className="ic-btn" title="Refresh">
          <Icon name="RefreshCw" size={15} />
        </button>
        <button className={`ic-btn${showIgnored ? " on" : ""}`} title="Show ignored files" onClick={() => setShowIgnored((v) => !v)}>
          <Icon name={showIgnored ? "Eye" : "EyeOff"} size={15} />
        </button>
        <span style={{ flex: 1 }} />
        <button className="ic-btn" title="Folder actions">
          <span style={{ fontSize: 15, lineHeight: 1 }}>···</span>
        </button>
      </div>
      <div className="side-head" style={{ paddingTop: 0 }}>
        <div className="field" title={root}>
          <Icon name="Search" size={13} />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={mode === "contents" ? "Search" : "Find in folder"} aria-label="Find in folder" />
        </div>
      </div>
      <div className="seg" style={{ padding: "0 6px 6px" }}>
        <button className={mode === "names" ? "on" : ""} style={{ flex: 1 }} onClick={() => setMode("names")}>
          Names
        </button>
        <button className={mode === "contents" ? "on" : ""} style={{ flex: 1 }} onClick={() => setMode("contents")}>
          Contents
        </button>
      </div>
      <div className="side-scroll" style={{ paddingTop: 2 }}>
        {rows.length ? rows : <div className="weak" style={{ padding: 10 }}>No results</div>}
      </div>
    </>
  );
}

/* ============================ Right: Agents ============================ */
export function AgentsPanel() {
  const [cards, setCards] = useState<AgentCard[]>(agentCards);
  const [tab, setTab] = useState<"attention" | "all">("attention");
  const shown = tab === "attention" ? cards.filter((c) => c.state !== "completed") : cards;
  const dismiss = (id: string) => setCards((cs) => cs.filter((c) => c.id !== id));
  return (
    <>
      <div className="agent-tabs">
        <button className={tab === "attention" ? "on" : ""} onClick={() => setTab("attention")}>
          Needs attention
        </button>
        <button className={tab === "all" ? "on" : ""} onClick={() => setTab("all")}>
          All
        </button>
      </div>
      <div className="side-scroll">
        {shown.length === 0 && <div className="agent-empty">Inbox zero. Hooks report here when an agent needs you.</div>}
        {shown.map((c) => (
          <div className="agent-card" key={c.id}>
            <div className="head">
              <Icon name={c.brand} size={15} />
              <span className="state">{c.state}</span>
              <span className="grow" style={{ flex: 1 }} />
              <span className="weak mono" style={{ fontSize: 11 }}>{c.title}</span>
            </div>
            <div className="weak" style={{ fontSize: 12, marginTop: 6 }}>{c.body}</div>
            <div className="btns">
              <button className="small-btn go" title="Focus the agent's terminal" onClick={() => dismiss(c.id)}>
                go
              </button>
              <button className="small-btn" onClick={() => dismiss(c.id)}>
                snooze
              </button>
              <button className="small-btn" onClick={() => dismiss(c.id)}>
                dismiss
              </button>
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

/* ============================ Right: Git ============================ */
export function GitPanel({
  onOpenDiff,
}: {
  onOpenDiff: (f: FileNode, staged: boolean) => void;
}) {
  const [msg, setMsg] = useState("");
  return (
    <>
      <div className="git-head">
        <button className="ic-btn" title="Refresh">
          <Icon name="RefreshCw" size={15} />
        </button>
        <button className="ic-btn" title="Fetch">
          <Icon name="GitCompareArrows" size={15} />
        </button>
        <span className="field" style={{ justifyContent: "flex-end" }}>
          <span className="weak">Last known directory</span>
        </span>
      </div>
      <div className="git-branch-row">
        <button className="ic-btn" title="Checkout branch" style={{ width: 22, height: 22 }}>
          <Icon name="GitBranch" size={14} />
        </button>
        <span className="mono">master</span>
      </div>
      <textarea className="git-msg" value={msg} onChange={(e) => setMsg(e.target.value)} placeholder="Message" aria-label="Commit message" />
      <div className="git-actions">
        <button className="small-btn" onClick={() => setMsg("")}>Stage All</button>
        <button className="small-btn" onClick={() => setMsg("")}>Commit</button>
      </div>
      <div className="side-scroll" style={{ paddingTop: 4 }}>
        {gitGroups.map((g) => (
          <div key={g.title}>
            <div className="git-group">
              <Icon name="ChevronDown" size={12} />
              <span>{g.title}</span>
              <span>{g.count}</span>
              <span className="grow" />
              <Icon name="Plus" size={13} />
            </div>
            {g.files.map((f) => (
              <div key={f.path} className="tree-row indent" onClick={() => onOpenDiff(f, g.title === "CHANGES")} title={`Open diff for ${f.path}`}>
                <span className="row-ic">
                  <Icon name={fileIcon(f.name)} size={15} />
                </span>
                <span className="row-label">{f.name}</span>
                <span className="row-trail mono" style={{ color: f.status === "U" ? "var(--git-untracked)" : "var(--git-modified)" }}>
                  {f.status === "U" ? "+1 U" : "+2 -1 M"}
                </span>
              </div>
            ))}
          </div>
        ))}
      </div>
    </>
  );
}

/* ============================ Right: History ============================ */
export function HistoryPanel() {
  const [filter, setFilter] = useState("");
  const [expanded, setExpanded] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(historyGroups.map((g) => [g.project, true])),
  );
  const groups = historyGroups.filter((g) => {
    const q = filter.trim().toLowerCase();
    return !q || g.project.includes(q) || g.sessions.some((s) => s.label.toLowerCase().includes(q));
  });
  const allExp = groups.every((g) => expanded[g.project]);
  return (
    <>
      <div className="side-head">
        <div className="field">
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter by name" aria-label="Filter history" style={{ fontSize: 12 }} />
        </div>
        <button className="small-btn" style={{ height: "var(--toolbar)" }} onClick={() => setExpanded(Object.fromEntries(historyGroups.map((g) => [g.project, !allExp])))}>
          {allExp ? "Collapse all" : "Expand all"}
        </button>
        <button className="small-btn" style={{ height: "var(--toolbar)" }}>Sort</button>
      </div>
      <div className="side-scroll">
        {groups.length === 0 && <div className="agent-empty">No matching sessions.</div>}
        {groups.map((g) => (
          <div key={g.project}>
            <div className="tree-row" onClick={() => setExpanded((e) => ({ ...e, [g.project]: !e[g.project] }))}>
              <Icon name={expanded[g.project] ? "ChevronDown" : "ChevronRight"} size={12} />
              <span className="row-label">{g.project}</span>
              <span className="row-trail">{g.sessions.length}</span>
            </div>
            {expanded[g.project] &&
              g.sessions.map((s) => (
                <div key={s.id} className="tree-row indent">
                  <span className="row-ic">
                    <Icon name="Terminal" size={15} />
                  </span>
                  <span className="row-label mono">{s.label}</span>
                  <span className="row-trail">{s.when}</span>
                </div>
              ))}
          </div>
        ))}
      </div>
    </>
  );
}

/* ============================ Right: Info ============================ */
export function InfoPanel({ session }: { session: Session }) {
  const bars: [string, number, string][] = [
    ["cpu", systemInfo.cpu, `${systemInfo.cpu}%`],
    ["memory", Math.round((systemInfo.memUsed / systemInfo.memTotal) * 100), `${systemInfo.memUsed} GB / ${systemInfo.memTotal} GB`],
    ["pressure", 12, systemInfo.pressure],
    ["load", 44, systemInfo.load],
  ];
  return (
    <>
      <div className="info-h">
        <span>Process</span>
        <span className="grow" />
        <Icon name="ChevronDown" size={13} />
      </div>
      <div className="info-row">
        <span className="k">session</span>
        <span className="v">{session.label}</span>
      </div>
      <div className="info-row">
        <span className="k">cwd</span>
        <span className="v mono">{session.cwd}</span>
      </div>
      <div className="info-row">
        <span className="k">branch</span>
        <span className="v">{session.branch}</span>
      </div>
      <div className="info-row">
        <span className="k">started</span>
        <span className="v">{session.started}</span>
      </div>
      <div className="info-h" style={{ marginTop: 6 }}>
        <span>Resources</span>
        <span className="grow" />
        <span className="weak" style={{ fontSize: 12 }}>✓ SYSTEM</span>
        <Icon name="ChevronDown" size={13} />
      </div>
      <div className="section-cap" style={{ paddingTop: 2 }}>This session</div>
      <div className="info-row">
        <span className="k">cpu</span>
        <span className="v">{session.cpu}%</span>
      </div>
      <div className="info-row">
        <span className="k">memory</span>
        <span className="v">{session.mem} MB</span>
      </div>
      <div className="section-cap">System</div>
      {bars.map(([k, v, label]) => (
        <div className="bar-row" key={k}>
          <span className="k">{k}</span>
          <span className="bar">
            <i style={{ width: `${v}%` }} />
          </span>
          <span className="v">{label}</span>
        </div>
      ))}
    </>
  );
}
