import type * as React from "react";
import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { Icon } from "./icons";
import { CtxMenu, type MenuItem, type MenuState } from "./menu";
import { Palette, type Command } from "./palette";
import { Terminal } from "./terminal";
import { AgentsPanel, ExplorerPanel, GitPanel, HistoryPanel, InfoPanel, ProjectsTree } from "./sidebar";
import { DiffView, EditorView, ImageView, MarkdownView, PlayerView, SettingsView, TourView } from "./views";
import {
  APP_TS,
  BILLING_TS,
  CSS,
  DOWNLOAD,
  README_MD,
  REPO,
  firstLeaf,
  leaf,
  nid,
  projects,
  removePane,
  sessions as seedSessions,
  type FileNode,
  type Group,
  type Layout,
  type PaneContent,
  type Session,
} from "./data";

const LEFT_W = 260;
const RIGHT_W = 300;

const TOOLS = [
  { id: "explorer", label: "Explorer", icon: "Files" },
  { id: "agents", label: "Agents", icon: "Bell" },
  { id: "git", label: "Git", icon: "GitBranch" },
  { id: "history", label: "History", icon: "History" },
  { id: "info", label: "Info", icon: "Info" },
  { id: "settings", label: "Settings", icon: "Settings" },
  { id: "palette", label: "Command palette", icon: "Search" },
  { id: "ide", label: "IDE mode", icon: "LayoutDashboard" },
] as const;

type ToolId = (typeof TOOLS)[number]["id"];
type Central = "dock" | "settings" | "player";

function basename(p: string) {
  return p.split("/").pop() ?? p;
}

function meta(c: PaneContent, sess: Session[]): { label: string; icon: string } {
  switch (c.kind) {
    case "tour":
      return { label: "Tour", icon: "FileText" };
    case "term": {
      const s = sess.find((x) => x.id === c.sid);
      return { label: s?.label ?? "Terminal", icon: "Terminal" };
    }
    case "editor":
      return { label: basename(c.path), icon: "FileCode" };
    case "markdown":
      return { label: basename(c.path), icon: "FileText" };
    case "diff":
      return { label: basename(c.path), icon: "FileDiff" };
    case "image":
      return { label: basename(c.path), icon: "FileImage" };
  }
}

function splitLeaf(layout: Layout, leafId: string, dir: "row" | "col", content: PaneContent): Layout {
  if (layout.type === "leaf") {
    if (layout.id !== leafId) return layout;
    return { type: "split", id: nid("split"), dir, children: [layout, leaf(content)] };
  }
  return { ...layout, children: layout.children.map((c) => splitLeaf(c, leafId, dir, content)) };
}

let termSeq = 10;

export default function App() {
  const [sessions, setSessions] = useState<Session[]>(seedSessions);
  const [selectedProject, setSelectedProject] = useState("atlas-web");
  const [groups, setGroups] = useState<Group[]>(() => [
    {
      id: "grp-tour",
      layout: {
        type: "split",
        id: "split-tour",
        dir: "row",
        children: [leaf({ kind: "tour" }, "lf-tour"), leaf({ kind: "term", sid: "s-api" }, "lf-tour-term")],
      },
      activeLeaf: "lf-tour",
    },
    {
      id: "grp-dev",
      layout: {
        type: "split",
        id: "split-dev",
        dir: "row",
        children: [leaf({ kind: "term", sid: "s-dev" }, "lf-dev"), leaf({ kind: "term", sid: "s-tests" }, "lf-tests")],
      },
      activeLeaf: "lf-dev",
    },
    { id: "grp-readme", layout: leaf({ kind: "markdown", path: "README.md" }, "lf-readme"), activeLeaf: "lf-readme" },
  ]);
  const [activeGroup, setActiveGroup] = useState("grp-tour");
  const [central, setCentral] = useState<Central>("dock");
  const [tool, setTool] = useState<ToolId>("explorer");
  const [leftVisible, setLeftVisible] = useState(true);
  const [rightVisible, setRightVisible] = useState(true);
  const [ideMode, setIdeMode] = useState(false);
  const [stripSid, setStripSid] = useState("s-dev");
  const [accent, setAccent] = useState("#3871e1");
  const [compact, setCompact] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [menu, setMenu] = useState<MenuState | null>(null);

  const group = groups.find((g) => g.id === activeGroup) ?? groups[0];
  const focusedLeaf = group ? group.activeLeaf : "";

  useEffect(() => {
    document.documentElement.style.setProperty("--accent", accent);
  }, [accent]);

  useEffect(() => {
    if (!menu) return;
    const dismiss = (e: PointerEvent) => {
      if (!(e.target as HTMLElement).closest(".ctx")) setMenu(null);
    };
    window.addEventListener("pointerdown", dismiss, true);
    return () => window.removeEventListener("pointerdown", dismiss, true);
  }, [menu]);

  const openMenu = (e: React.MouseEvent, items: MenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    if ((e.target as HTMLElement).closest("input,textarea")) return;
    setMenu({ x: e.clientX, y: e.clientY, items });
  };

  const mod = useMemo(
    () => (typeof navigator !== "undefined" && /Mac/.test(navigator.platform ?? "") ? "⌘" : "Ctrl+"),
    [],
  );

  /* --------------------------- mutations --------------------------- */
  const newSession = useCallback((projectId: string): Session => {
    const n = ++termSeq;
    const s: Session = {
      id: `s-new-${n}`,
      label: `Terminal ${n}`,
      projectId,
      branch: projects.find((p) => p.id === projectId)?.branch ?? "main",
      cwd: `~/code/${projectId}`,
      kind: "shell",
      status: "running",
      visible: true,
      started: "now",
      cpu: 0,
      mem: 2,
    };
    setSessions((xs) => [...xs, s]);
    return s;
  }, []);

  const setLayout = (groupId: string, layout: Layout, activeLeaf?: string) =>
    setGroups((gs) => gs.map((g) => (g.id === groupId ? { ...g, layout, activeLeaf: activeLeaf ?? g.activeLeaf } : g)));

  const addGroup = (content: PaneContent): string => {
    const lf = leaf(content);
    const id = nid("grp");
    setGroups((gs) => [...gs, { id, layout: lf, activeLeaf: lf.id }]);
    setActiveGroup(id);
    setCentral("dock");
    return id;
  };

  const closeGroup = (id: string) => {
    setGroups((gs) => {
      const next = gs.filter((g) => g.id !== id);
      if (next.length === 0) {
        const s = newSession(selectedProject);
        const lf = leaf({ kind: "term", sid: s.id });
        setActiveGroup("grp-fresh");
        return [{ id: "grp-fresh", layout: lf, activeLeaf: lf.id }];
      }
      if (id === activeGroup) setActiveGroup(next[Math.max(0, gs.findIndex((g) => g.id === id) - 1)]?.id ?? next[0].id);
      return next;
    });
  };

  const closeLeaf = (groupId: string, leafId: string) => {
    const g = groups.find((x) => x.id === groupId);
    if (!g) return;
    const next = removePane(g.layout, leafId);
    if (!next) {
      closeGroup(groupId);
      return;
    }
    const activeLeaf = next.type === "leaf" ? next.id : firstLeaf(next).id;
    setLayout(groupId, next, activeLeaf);
  };

  const openInNewTab = (content: PaneContent) => addGroup(content);

  const openFile = (f: FileNode) => {
    const name = f.name.toLowerCase();
    if (name.endsWith(".wav") || name.endsWith(".mp3")) {
      setCentral("player");
      return;
    }
    if (name.endsWith(".png") || name.endsWith(".jpg")) {
      openInNewTab({ kind: "image", path: f.path });
      return;
    }
    if (name.endsWith(".md")) {
      openInNewTab({ kind: "markdown", path: f.path });
      return;
    }
    openInNewTab({ kind: "editor", path: f.path });
  };

  const openFileContent = (path: string): string => {
    if (path.endsWith(".md")) return README_MD;
    if (path.endsWith("app.ts")) return APP_TS;
    if (path.endsWith("billing.ts")) return BILLING_TS;
    if (path.endsWith(".css")) return CSS;
    return "";
  };

  const goSession = (sid: string) => {
    const s = sessions.find((x) => x.id === sid);
    if (s) setSelectedProject(s.projectId);
    const owner = groups.find((g) =>
      collectLeaves(g.layout).some((l) => l.content.kind === "term" && l.content.sid === sid),
    );
    if (owner) {
      setActiveGroup(owner.id);
      const l = collectLeaves(owner.layout).find((x) => x.content.kind === "term" && x.content.sid === sid);
      if (l) setLayout(owner.id, owner.layout, l.id);
    } else {
      addGroup({ kind: "term", sid });
    }
    setCentral("dock");
  };

  const splitFocused = (dir: "row" | "col") => {
    if (!group) return;
    const s = newSession(selectedProject);
    const next = splitLeaf(group.layout, group.activeLeaf, dir, { kind: "term", sid: s.id });
    const nl = collectLeaves(next).find((x) => x.content.kind === "term" && x.content.sid === s.id);
    setLayout(group.id, next, nl?.id ?? group.activeLeaf);
  };

  const selectTool = (id: ToolId) => {
    if (id === "settings") {
      setCentral("settings");
      return;
    }
    if (id === "palette") {
      setPaletteOpen(true);
      return;
    }
    if (id === "ide") {
      setIdeMode((v) => !v);
      return;
    }
    setCentral("dock");
    if (tool === id && rightVisible) {
      setRightVisible(false);
    } else {
      setTool(id);
      setRightVisible(true);
    }
  };

  /* --------------------------- commands --------------------------- */
  const commands: Command[] = useMemo(() => {
    const cmds: Command[] = [
      { id: "new-tab", label: "New terminal tab", kind: "action", icon: "Plus", run: () => { const s = newSession(selectedProject); addGroup({ kind: "term", sid: s.id }); } },
      { id: "split-right", label: "Split right", kind: "action", icon: "Columns2", run: () => splitFocused("row") },
      { id: "split-down", label: "Split down", kind: "action", icon: "Rows2", run: () => splitFocused("col") },
      { id: "settings", label: "Open Settings", kind: "action", icon: "Settings", run: () => setCentral("settings") },
      { id: "ide", label: "Toggle IDE mode", kind: "action", icon: "LayoutDashboard", run: () => setIdeMode((v) => !v) },
      { id: "sidebar", label: "Toggle sidebar", kind: "action", icon: "PanelLeft", run: () => setLeftVisible((v) => !v) },
      { id: "player", label: "Open Player", kind: "action", icon: "AudioLines", run: () => setCentral("player") },
      { id: "download", label: "Download Terminator", kind: "link", icon: "ArrowDown", run: () => window.open(DOWNLOAD, "_blank") },
    ];
    for (const s of sessions) cmds.push({ id: `go-${s.id}`, label: s.label, kind: "session", icon: "Terminal", sub: s.projectId, run: () => goSession(s.id) });
    for (const p of projects) cmds.push({ id: `proj-${p.id}`, label: p.name, kind: "project", icon: "FolderOpen", sub: p.path, run: () => { setSelectedProject(p.id); setCentral("dock"); } });
    cmds.push(
      { id: "tool-explorer", label: "Show Explorer", kind: "view", icon: "Files", run: () => { setTool("explorer"); setRightVisible(true); } },
      { id: "tool-agents", label: "Show Agents", kind: "view", icon: "Bell", run: () => { setTool("agents"); setRightVisible(true); } },
      { id: "tool-git", label: "Show Git", kind: "view", icon: "GitBranch", run: () => { setTool("git"); setRightVisible(true); } },
      { id: "tool-history", label: "Show History", kind: "view", icon: "History", run: () => { setTool("history"); setRightVisible(true); } },
      { id: "tool-info", label: "Show Info", kind: "view", icon: "Info", run: () => { setTool("info"); setRightVisible(true); } },
    );
    return cmds;
  }, [sessions, selectedProject, groups]);

  /* --------------------------- deep links --------------------------- */
  useEffect(() => {
    const h = window.location.hash.replace(/^#\/?/, "");
    if (!h) return;
    if (h === "agents") {
      setTool("agents");
      setRightVisible(true);
    } else if (h === "git") {
      setTool("git");
      setRightVisible(true);
    } else if (h === "history") {
      setTool("history");
      setRightVisible(true);
    } else if (h === "info") {
      setTool("info");
      setRightVisible(true);
    } else if (h === "settings") {
      setCentral("settings");
    } else if (h === "player") {
      setCentral("player");
    } else if (h === "ide") {
      setIdeMode(true);
    } else if (h === "palette") {
      setPaletteOpen(true);
    } else if (h === "readme") {
      setActiveGroup("grp-readme");
    } else if (h === "dev") {
      setActiveGroup("grp-dev");
    } else if (h === "diff") {
      setActiveGroup("grp-diff");
      setGroups((gs) =>
        gs.some((g) => g.id === "grp-diff")
          ? gs
          : [...gs, { id: "grp-diff", layout: leaf({ kind: "diff", path: "src/app.ts", staged: true }, "lf-diff"), activeLeaf: "lf-diff" }],
      );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /* --------------------------- keyboard --------------------------- */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const meta = e.metaKey || e.ctrlKey;
      if (!meta) {
        if (e.key === "Escape") {
          setMenu(null);
          setPaletteOpen(false);
        }
        return;
      }
      const k = e.key.toLowerCase();
      if (k === "k" || k === "p") {
        e.preventDefault();
        setPaletteOpen(true);
      } else if (k === ",") {
        e.preventDefault();
        setCentral("settings");
      } else if (k === "e") {
        e.preventDefault();
        setIdeMode((v) => !v);
      } else if (k === "b") {
        e.preventDefault();
        setLeftVisible((v) => !v);
      } else if (k === "t") {
        e.preventDefault();
        const s = newSession(selectedProject);
        addGroup({ kind: "term", sid: s.id });
      } else if (k === "w") {
        e.preventDefault();
        if (group) closeGroup(group.id);
      } else if (k === "d") {
        e.preventDefault();
        splitFocused(e.shiftKey ? "col" : "row");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /* --------------------------- menus --------------------------- */
  const termMenu = (e: React.MouseEvent, sid: string, leafId: string) => {
    const s = sessions.find((x) => x.id === sid);
    openMenu(e, [
      { kind: "header", label: s?.cwd ?? "~" },
      { kind: "sep" },
      {
        kind: "item",
        label: "Copy",
        icon: "Copy",
        shortcut: `${mod}C`,
        run: () => navigator.clipboard?.writeText(window.getSelection()?.toString() ?? ""),
      },
      { kind: "item", label: "Select all", icon: "TextSelect", run: () => {} },
      { kind: "item", label: "Paste", icon: "ClipboardPaste", shortcut: `${mod}V`, run: () => {} },
      { kind: "sep" },
      { kind: "item", label: "New tab", icon: "Plus", run: () => { const ns = newSession(selectedProject); addGroup({ kind: "term", sid: ns.id }); } },
      { kind: "item", label: "Split up", icon: "ArrowUp", run: () => splitFocused("col") },
      { kind: "item", label: "Split down", icon: "ArrowDown", run: () => splitFocused("col") },
      { kind: "item", label: "Split left", icon: "ArrowLeft", run: () => splitFocused("row") },
      { kind: "item", label: "Split right", icon: "ArrowRight", run: () => splitFocused("row") },
      { kind: "sep" },
      { kind: "item", label: "Open file path…", icon: "FolderOpen", run: () => setTool("explorer") },
      { kind: "item", label: "Search scrollback", icon: "Search", run: () => setTool("history") },
      { kind: "item", label: "Copy working directory", icon: "Copy", run: () => navigator.clipboard?.writeText(s?.cwd ?? "") },
      { kind: "sep" },
      { kind: "item", label: "Close session…", icon: "X", destructive: true, run: () => { if (group) closeLeaf(group.id, leafId); } },
    ]);
  };

  const paneMenu = (e: React.MouseEvent, leafId: string, title: string) => {
    openMenu(e, [
      { kind: "header", label: title },
      { kind: "sep" },
      { kind: "item", label: "New tab", icon: "Plus", run: () => { const ns = newSession(selectedProject); addGroup({ kind: "term", sid: ns.id }); } },
      { kind: "item", label: "Split up", icon: "ArrowUp", run: () => splitFocused("col") },
      { kind: "item", label: "Split down", icon: "ArrowDown", run: () => splitFocused("col") },
      { kind: "item", label: "Split left", icon: "ArrowLeft", run: () => splitFocused("row") },
      { kind: "item", label: "Split right", icon: "ArrowRight", run: () => splitFocused("row") },
      { kind: "sep" },
      { kind: "item", label: "Search scrollback", icon: "Search", run: () => setTool("history") },
      { kind: "item", label: "Clear saved scrollback", icon: "Eraser", destructive: true, run: () => {} },
    ]);
  };

  const tabMenu = (e: React.MouseEvent, g: Group, index: number) => {
    openMenu(e, [
      { kind: "item", label: "Close tab…", icon: "X", destructive: true, run: () => closeGroup(g.id) },
      { kind: "sep" },
      { kind: "item", label: "Close all tabs…", icon: "X", destructive: true, disabled: groups.length <= 1, run: () => setGroups([groups[Math.min(index, groups.length - 1)]]) },
      { kind: "item", label: "Close all tabs to the left…", icon: "X", destructive: true, disabled: index <= 0, run: () => setGroups(groups.slice(index)) },
      { kind: "item", label: "Close all tabs to the right…", icon: "X", destructive: true, disabled: index + 1 >= groups.length, run: () => setGroups(groups.slice(0, index + 1)) },
      { kind: "sep" },
      { kind: "item", label: "Add tab to the left", icon: "Plus", run: () => { const s = newSession(selectedProject); const id = nid("grp"); const lf = leaf({ kind: "term", sid: s.id }); setGroups((gs) => [...gs.slice(0, index), { id, layout: lf, activeLeaf: lf.id }, ...gs.slice(index)]); setActiveGroup(id); } },
      { kind: "item", label: "Add tab to the right", icon: "Plus", run: () => { const s = newSession(selectedProject); const id = nid("grp"); const lf = leaf({ kind: "term", sid: s.id }); setGroups((gs) => [...gs.slice(0, index + 1), { id, layout: lf, activeLeaf: lf.id }, ...gs.slice(index + 1)]); setActiveGroup(id); } },
    ]);
  };

  const projectMenu = (e: React.MouseEvent, pid: string) => {
    openMenu(e, [
      { kind: "item", label: "New task worktree…", icon: "GitBranch", run: () => {} },
      { kind: "sep" },
      { kind: "item", label: "Remove project from sidebar", icon: "X", destructive: true, run: () => {} },
    ]);
  };

  const sessionMenu = (e: React.MouseEvent, s: Session) => {
    openMenu(e, [
      { kind: "item", label: "Rename…", icon: "Pencil", run: () => {} },
      { kind: "sep" },
      { kind: "item", label: "Open session", icon: "Terminal", run: () => goSession(s.id) },
      { kind: "sep" },
      { kind: "item", label: "Close session…", icon: "X", destructive: true, run: () => {} },
    ]);
  };

  const fileMenu = (e: React.MouseEvent, f: FileNode) => {
    openMenu(e, [
      { kind: "item", label: "Open", icon: "ExternalLink", run: () => openFile(f) },
      { kind: "item", label: "Open to the side", icon: "Columns2", run: () => { openFile(f); } },
      { kind: "sep" },
      { kind: "item", label: "Copy path", icon: "Copy", run: () => navigator.clipboard?.writeText(f.path) },
      { kind: "item", label: "Reveal in file manager", icon: "FolderOpen", run: () => {} },
      { kind: "item", label: "Rename…", icon: "Pencil", run: () => {} },
      { kind: "item", label: "Delete", icon: "X", destructive: true, run: () => {} },
    ]);
  };

  /* --------------------------- render --------------------------- */
  const renderLeaf = (l: Extract<Layout, { type: "leaf" }>, groupId: string) => {
    const c = l.content;
    const foc = groupId === activeGroup && l.id === focusedLeaf;
    const m = meta(c, sessions);
    const onClose = () => closeLeaf(groupId, l.id);
    const focus = () => setLayout(groupId, groups.find((g) => g.id === groupId)!.layout, l.id);

    if (c.kind === "term" || c.kind === "editor" || c.kind === "image") {
      const s = c.kind === "term" ? sessions.find((x) => x.id === c.sid) : undefined;
      return (
        <section
          className={`pane${foc ? " foc" : ""}`}
          onClick={focus}
          onContextMenu={(e) => (s ? termMenu(e, s.id, l.id) : undefined)}
        >
          <div className="pane-bar" onContextMenu={(e) => paneMenu(e, l.id, m.label)}>
            <div className="lead">
              {s?.agent && <Icon name={s.agent} size={13} />}
              <span className={s?.agent ? "" : ""}>
                <Icon name={m.icon} size={13} />
              </span>
            </div>
            <span className="pane-title">{m.label}</span>
            <span className="pane-branch">
              {s ? (
                <>
                  <span
                    className={`dot ${s.status}`}
                    style={{ display: "inline-block", verticalAlign: "middle", marginRight: 5 }}
                  />
                  <Icon name="GitBranch" size={13} style={{ display: "inline-block", verticalAlign: "-2px", marginRight: 3 }} />
                  {s.branch}
                </>
              ) : (
                ""
              )}
            </span>
            <button className="bar-btn" title="Split right" onClick={(e) => { e.stopPropagation(); focus(); splitFocused("row"); }}>
              <Icon name="Columns2" size={14} />
            </button>
            <button className="bar-btn" title="Split down" onClick={(e) => { e.stopPropagation(); focus(); splitFocused("col"); }}>
              <Icon name="Rows2" size={14} />
            </button>
            <button className="bar-btn danger" title="Close pane" onClick={(e) => { e.stopPropagation(); onClose(); }}>
              <Icon name="X" size={14} />
            </button>
          </div>
          <div className="pane-body">
            {c.kind === "term" && <Terminal sid={c.sid} />}
            {c.kind === "editor" && <EditorView path={c.path} content={openFileContent(c.path)} />}
            {c.kind === "image" && <ImageView path={c.path} />}
          </div>
        </section>
      );
    }

    return (
      <section className={`pane${foc ? " foc" : ""}`} onClick={focus}>
        {c.kind === "tour" && <TourView onClose={onClose} />}
        {c.kind === "markdown" && <MarkdownView path={c.path} content={openFileContent(c.path)} onClose={onClose} />}
        {c.kind === "diff" && <DiffView path={c.path} staged={c.staged} onClose={onClose} />}
      </section>
    );
  };

  const renderLayout = (l: Layout, groupId: string): React.ReactNode => {
    if (l.type === "leaf") return renderLeaf(l, groupId);
    return (
      <div className={`split ${l.dir}`}>
        {l.children.map((c, i) => (
          <Fragment key={c.id}>
            {i > 0 && <div className="divider" />}
            {renderLayout(c, groupId)}
          </Fragment>
        ))}
      </div>
    );
  };

  /* --------------------------- right sidebar --------------------------- */
  const activeSession = useMemo(() => {
    if (!group) return sessions[0];
    const l = collectLeaves(group.layout).find((x) => x.id === group.activeLeaf);
    const sid = l && l.content.kind === "term" ? l.content.sid : sessions.find((s) => s.projectId === selectedProject)?.id;
    return sessions.find((s) => s.id === sid) ?? sessions[0];
  }, [group, sessions, selectedProject]);

  const rightW = rightVisible ? RIGHT_W : 0;
  const budget = rightW - 16 - 36;
  const rowWidth = (n: number, menu: boolean) => n * 36 + (n > 1 ? (n - 1) * 4 : 0) + (menu ? (n > 0 ? 4 : 0) + 32 : 0);
  let visible: number = TOOLS.length;
  if (rowWidth(TOOLS.length, false) > budget) {
    visible = 0;
    while (visible < TOOLS.length && rowWidth(visible + 1, true) <= budget) visible++;
  }
  const hidden = TOOLS.slice(visible);

  const headerToolMenu = (e: React.MouseEvent) => {
    e.stopPropagation();
    openMenu(e, hidden.map((t) => ({ kind: "item", label: t.label, icon: t.icon, checked: tool === t.id, run: () => selectTool(t.id) })));
  };

  const projectNavMenu = (e: React.MouseEvent) => {
    e.stopPropagation();
    openMenu(e, [
      { kind: "header", label: "terminator 2.4.0" },
      { kind: "sep" },
      { kind: "item", label: "Download latest release", icon: "ArrowDown", run: () => window.open(DOWNLOAD, "_blank") },
      { kind: "item", label: "Star on GitHub", icon: "ExternalLink", run: () => window.open(REPO, "_blank") },
      { kind: "sep" },
      { kind: "item", label: "Check for updates", icon: "RefreshCw", run: () => {} },
    ]);
  };

  return (
    <div className={`app${compact ? " compact" : ""}`} onContextMenu={(e) => e.preventDefault()}>
      {/* header */}
      <header className="titlebar">
        <div className="titlebar-left" style={{ width: leftVisible ? LEFT_W : undefined }}>
          <div className="traffic">
            <i />
            <i />
            <i />
          </div>
          <button className="proj-name" title="Project menu" onClick={projectNavMenu}>
            {projects.find((p) => p.id === selectedProject)?.name ?? "terminator"}
          </button>
          <button className="ic-btn" title="Player" onClick={() => setCentral("player")}>
            <Icon name="AudioLines" size={16} />
          </button>
          <button className="ic-btn" title="Agents inbox" style={{ position: "relative", overflow: "visible" }} onClick={() => { setTool("agents"); setRightVisible(true); setCentral("dock"); }}>
            <Icon name="Bell" size={16} />
            <span className="badge" style={{ position: "absolute", top: 1, left: 17, transform: "translateY(-30%)", background: "#ff3b30", color: "#fff", borderRadius: 999, fontSize: 9, lineHeight: "13px", padding: "0 4px" }}>
              2
            </span>
          </button>
          <button className="ic-btn" title="Toggle sidebar" onClick={() => setLeftVisible((v) => !v)}>
            <Icon name="PanelLeft" size={16} />
          </button>
        </div>
        <div className="titlebar-drag" />
        <div className="titlebar-tools" style={{ width: rightVisible ? RIGHT_W : undefined }}>
          {TOOLS.slice(0, visible).map((t) => (
            <button
              key={t.id}
              className={`ic-btn${(t.id === "ide" && ideMode) || (t.id === tool && rightVisible && t.id !== "settings" && t.id !== "palette") ? " on" : ""}`}
              title={t.label}
              onClick={() => selectTool(t.id)}
            >
              <Icon name={t.icon} size={16} />
            </button>
          ))}
          {hidden.length > 0 && (
            <button className="ic-btn" title="More" onClick={headerToolMenu}>
              <Icon name="Menu" size={16} />
            </button>
          )}
          <div className="titlebar-drag" />
          <button className="ic-btn" title="Toggle right sidebar" onClick={() => setRightVisible((v) => !v)}>
            <Icon name="PanelRight" size={16} />
          </button>
        </div>
      </header>

      {/* body */}
      <div className="body">
        <aside className={`sidebar left${leftVisible ? "" : " hidden"}`} style={{ width: LEFT_W }}>
          <ProjectsTree
            projects={projects}
            selected={selectedProject}
            onSelect={(id) => { setSelectedProject(id); setCentral("dock"); }}
            onOpenSession={goSession}
            onNewTerminal={() => addGroup({ kind: "term", sid: newSession(selectedProject).id })}
          />
        </aside>

        <main className="center">
          {central === "settings" ? (
            <SettingsView accent={accent} setAccent={setAccent} compact={compact} setCompact={setCompact} />
          ) : central === "player" ? (
            <PlayerView />
          ) : ideMode ? (
            <div className="ide">
              <div className="ide-top">{group && renderLayout(group.layout, group.id)}</div>
              <div className="ide-strip">
                <div className="tabstrip" style={{ height: 30 }}>
                  {sessions
                    .filter((s) => s.kind === "shell")
                    .map((s) => (
                      <button key={s.id} className={`tab${stripSid === s.id ? " on" : ""}`} onClick={() => setStripSid(s.id)}>
                        <Icon name="Terminal" size={13} />
                        <span className="tlabel">{s.label}</span>
                      </button>
                    ))}
                  <span className="tstrip-drag" />
                </div>
                <div className="pane-body" style={{ background: "var(--term-bg)" }}>
                  <Terminal sid={stripSid} />
                </div>
              </div>
            </div>
          ) : (
            <>
              <div className="tabstrip">
                <div className="tabs">
                  {groups.map((g, i) => {
                    const m = meta(firstLeaf(g.layout).content, sessions);
                    const on = g.id === activeGroup;
                    return (
                      <div
                        key={g.id}
                        className={`tab${on ? " on" : ""}`}
                        onContextMenu={(e) => tabMenu(e, g, i)}
                        onClick={() => { setActiveGroup(g.id); setCentral("dock"); }}
                        title={m.label}
                      >
                        <Icon name={m.icon} size={13} />
                        <span className="tlabel">{m.label}</span>
                        <button className="tx" aria-label={`Close ${m.label}`} onClick={(e) => { e.stopPropagation(); closeGroup(g.id); }}>
                          <Icon name="X" size={14} />
                        </button>
                      </div>
                    );
                  })}
                  <button className="tplus" title="New top-level terminal tab" onClick={() => addGroup({ kind: "term", sid: newSession(selectedProject).id })}>
                    <Icon name="Plus" size={17} />
                  </button>
                </div>
                <span className="tstrip-drag" />
              </div>
              <div className="display">{group && renderLayout(group.layout, group.id)}</div>
            </>
          )}
        </main>

        <aside className={`sidebar right${rightVisible ? "" : " hidden"}`} style={{ width: RIGHT_W }}>
          {tool === "explorer" && <ExplorerPanel root={`~/code/${selectedProject}`} onOpenFile={openFile} />}
          {tool === "agents" && <AgentsPanel />}
          {tool === "git" && <GitPanel onOpenDiff={(f, staged) => openInNewTab({ kind: "diff", path: f.path, staged })} />}
          {tool === "history" && <HistoryPanel />}
          {tool === "info" && <InfoPanel session={activeSession} />}
        </aside>
      </div>

      {/* status */}
      <footer className="status">
        <span>
          <span className="dot running" style={{ display: "inline-block" }} />
          Connected
        </span>
        <span className="vsep" />
        <span>
          {sessions.filter((s) => s.kind === "shell").length} sessions running{" "}
          {projects.find((p) => p.id === selectedProject)?.branch}
        </span>
        <span className="grow" />
        {ideMode && (
          <>
            <button className="link" onClick={() => setCentral("player")}>
              player
            </button>
            <button className="link" onClick={() => { setTool("agents"); setRightVisible(true); }}>
              agents · 2
            </button>
          </>
        )}
        <span className="mono" style={{ fontSize: 11 }}>108% · 974 MB</span>
      </footer>

      {paletteOpen && <Palette commands={commands} close={() => setPaletteOpen(false)} />}
      {menu && <CtxMenu menu={menu} close={() => setMenu(null)} />}
    </div>
  );
}

/* helpers */
function collectLeaves(l: Layout): Extract<Layout, { type: "leaf" }>[] {
  return l.type === "leaf" ? [l] : l.children.flatMap(collectLeaves);
}
