import type * as React from "react";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "./icons";
import { CtxMenu, type MenuItem, type MenuState } from "./menu";
import { Palette, type Command } from "./palette";
import { Terminal } from "./terminal";
import { AgentsPanel, ExplorerPanel, HistoryPanel, InfoPanel, ProjectsTree } from "./sidebar";
import { DiffView, GitPanel } from "./git";
import { BrowserView, LocalHistoryView } from "./panes";
import { HostsPanel, ProblemsPanel, StructurePanel, ToolStripe, VncPanel } from "./tools";
import { EditorView, ImageView, MarkdownView, PlayerView, SettingsView, TourView, type TourAction } from "./views";
import {
  APP_TS,
  BILLING_TS,
  CSS,
  DOWNLOAD,
  README_MD,
  REPO,
  basename,
  firstLeaf,
  fileIcon,
  flattenFiles,
  initialGroups,
  leaf,
  nid,
  projects,
  leaves,
  removePane,
  updatePane,
  sessions as seedSessions,
  treesByProject,
  type FileNode,
  type Group,
  type Layout,
  type PaneContent,
  type Session,
} from "./data";
/* Right panel width measured from native captures at 1440x900; keep in sync with --right-w in styles.css. */
const RIGHT_W = 326;

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

/* JetBrains-style tool stripes on both window edges, as in the native app. */
const STRIPE = [
  { id: "projects", label: "Projects", icon: "Folder" },
  { id: "explorer", label: "Explorer", icon: "Files" },
  { id: "agents", label: "Agents", icon: "Bell" },
  { id: "git", label: "Git", icon: "GitBranch" },
  { id: "history", label: "History", icon: "History" },
  { id: "info", label: "Info", icon: "Info" },
  { id: "problems", label: "Problems", icon: "CircleAlert" },
  { id: "structure", label: "Structure", icon: "ListTree" },
  { id: "vnc", label: "VNC", icon: "Monitor" },
  { id: "hosts", label: "Hosts", icon: "Globe" },
] as const;
type SideTool = (typeof STRIPE)[number]["id"];
type PaletteMode = "all" | "files" | "actions" | "recent";
const PALETTE_TITLE: Record<PaletteMode, string> = {
  all: "Search Everywhere (double Shift)",
  files: "Go to File (⌘⇧O)",
  actions: "Find Action (⌘⇧A)",
  recent: "Recent Files (⌘E)",
};
/** Command kinds each palette mode lists; null lists everything. */
const PALETTE_KINDS: Record<PaletteMode, string[] | null> = {
  all: null,
  files: ["file"],
  actions: ["action", "view"],
  recent: ["file", "session"],
};
/** ⌘digit tool-window keys (Alt+digit on Linux in the app). */
const TOOL_KEYS: Record<string, SideTool | undefined> = { "1": "explorer", "6": "problems", "7": "structure", "9": "git" };

/* Header tool row: how many fit beside the right panel; the rest go in the overflow menu. */
const rowWidth = (n: number, menu: boolean) => n * 36 + (n > 1 ? (n - 1) * 4 : 0) + (menu ? (n > 0 ? 4 : 0) + 32 : 0);
const HEADER_BUDGET = RIGHT_W - 16 - 36;
let headerFit: number = TOOLS.length;
if (rowWidth(TOOLS.length, false) > HEADER_BUDGET) {
  headerFit = 0;
  while (headerFit < TOOLS.length && rowWidth(headerFit + 1, true) <= HEADER_BUDGET) headerFit++;
}
const HEADER_VISIBLE = TOOLS.slice(0, headerFit);
const HEADER_HIDDEN = TOOLS.slice(headerFit);
type Central = "dock" | "settings" | "player";


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
    case "browser":
      return { label: basename(c.url) || c.url, icon: "Globe" };
    case "localhistory":
      return { label: `Local History: ${basename(c.path)}`, icon: "History" };
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
  /* Each project owns its tabs and split layout, preserved across switches. */
  const [workspaces, setWorkspaces] = useState<Record<string, { groups: Group[]; active: string }>>(() =>
    Object.fromEntries(
      projects.map((p) => {
        const gs = initialGroups(p.id);
        return [p.id, { groups: gs, active: gs[0]?.id ?? "" }];
      }),
    ),
  );
  const ws = workspaces[selectedProject] ?? { groups: initialGroups(selectedProject), active: "" };
  const groups = ws.groups;
  const activeGroup = ws.active;
  const setActiveGroup = (id: string) =>
    setWorkspaces((w) => ({ ...w, [selectedProject]: { ...w[selectedProject], active: id } }));
  const setGroups = (u: Group[] | ((gs: Group[]) => Group[]), active?: string) =>
    setWorkspaces((w) => {
      const cur = w[selectedProject];
      if (!cur) return w;
      const next = typeof u === "function" ? (u as (gs: Group[]) => Group[])(cur.groups) : u;
      return {
        ...w,
        [selectedProject]: {
          groups: next,
          active: active ?? (next.some((g) => g.id === cur.active) ? cur.active : (next[0]?.id ?? cur.active)),
        },
      };
    });
  const [central, setCentral] = useState<Central>("dock");
  const [tool, setTool] = useState<SideTool>("explorer");
  const [leftTool, setLeftTool] = useState<SideTool>("projects");
  const [maxLeaf, setMaxLeaf] = useState<string | null>(null);
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

  /* Like the app: one diff tab per project, reused for every file. */
  const openDiff = (path: string) => {
    const content: PaneContent = { kind: "diff", path };
    const owner = groups.find((g) => leaves(g.layout).some((x) => x.content.kind === "diff"));
    if (!owner) {
      addGroup(content);
      return;
    }
    const lf = leaves(owner.layout).find((x) => x.content.kind === "diff")!;
    setGroups((gs) => gs.map((g) => (g.id === owner.id ? { ...g, layout: updatePane(g.layout, lf.id, content), activeLeaf: lf.id } : g)), owner.id);
    setCentral("dock");
  };

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
    if (name.endsWith(".html")) {
      openInNewTab({ kind: "browser", url: f.path });
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
    const pid = s?.projectId ?? selectedProject;
    if (s) setSelectedProject(pid);
    setWorkspaces((w) => {
      const cur = w[pid];
      if (!cur) return w;
      const owner = cur.groups.find((g) =>
        leaves(g.layout).some((l) => l.content.kind === "term" && l.content.sid === sid),
      );
      if (!owner) {
        const lf = leaf({ kind: "term", sid });
        const id = nid("grp");
        return { ...w, [pid]: { groups: [...cur.groups, { id, layout: lf, activeLeaf: lf.id }], active: id } };
      }
      const l = leaves(owner.layout).find((x) => x.content.kind === "term" && x.content.sid === sid);
      return {
        ...w,
        [pid]: {
          groups: cur.groups.map((g) => (g.id === owner.id && l ? { ...g, activeLeaf: l.id } : g)),
          active: owner.id,
        },
      };
    });
    setCentral("dock");
  };

  const splitFocused = (dir: "row" | "col") => {
    if (!group) return;
    const s = newSession(selectedProject);
    const next = splitLeaf(group.layout, group.activeLeaf, dir, { kind: "term", sid: s.id });
    const nl = leaves(next).find((x) => x.content.kind === "term" && x.content.sid === s.id);
    setLayout(group.id, next, nl?.id ?? group.activeLeaf);
  };

  const selectTool = (id: ToolId) => {
    if (id === "settings") {
      setCentral("settings");
      return;
    }
    if (id === "palette") {
      openPalette("all");
      return;
    }
    if (id === "ide") {
      setIdeMode((v) => !v);
      return;
    }
    toggleTool(id);
  };

  /* One place for "show a tool window" and the JetBrains toggle (hidden → show, shown → hide). */
  const showTool = (t: SideTool) => {
    setTool(t);
    setRightVisible(true);
    setCentral("dock");
  };
  const toggleTool = (t: SideTool) => {
    if (tool === t && rightVisible) {
      setRightVisible(false);
      setCentral("dock");
    } else showTool(t);
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
    for (const t of STRIPE) if (t.id !== "projects") cmds.push({ id: `tool-${t.id}`, label: `Show ${t.label}`, kind: "view", icon: t.icon, run: () => showTool(t.id) });
    cmds.push(
      { id: "local-history", label: "Local History: src/app.ts", kind: "action", icon: "History", run: () => openInNewTab({ kind: "localhistory", path: "src/app.ts" }) },
      { id: "new-task", label: "New task (worktree + terminal)…", kind: "action", icon: "GitBranch", run: () => newTask() },
    );
    for (const f of flattenFiles(treesByProject[selectedProject] ?? [])) cmds.push({ id: `file-${f.path}`, label: f.name, kind: "file", icon: fileIcon(f.name), sub: f.path, run: () => openFile(f) });
    return cmds;
  }, [sessions, selectedProject, groups]);

  const [paletteMode, setPaletteMode] = useState<PaletteMode>("all");
  const openPalette = (m: PaletteMode) => {
    setPaletteMode(m);
    setPaletteOpen(true);
  };
  const paletteCommands = useMemo(() => {
    const kinds = PALETTE_KINDS[paletteMode];
    return kinds ? commands.filter((c) => kinds.includes(c.kind)) : commands;
  }, [commands, paletteMode]);

  /* Status-bar note, like the app's transient status notes. */
  const [note, setNote] = useState("");
  useEffect(() => {
    if (!note) return;
    const t = setTimeout(() => setNote(""), 4000);
    return () => clearTimeout(t);
  }, [note]);

  /* New task: worktree + terminal; types the agent command but never presses Enter. */
  const newTask = () => {
    const s = newSession(selectedProject);
    setSessions((xs) => xs.map((x) => (x.id === s.id ? { ...x, label: "task/billing-fix", branch: "task/billing-fix", cwd: `~/code/${selectedProject}-worktrees/billing-fix` } : x)));
    addGroup({ kind: "term", sid: s.id });
    setNote("Created worktree task/billing-fix. The agent command is typed; press Enter to start it.");
  };

  /* --------------------------- deep links --------------------------- */
  useEffect(() => {
    const h = window.location.hash.replace(/^#\/?/, "");
    if (!h) return;
    const ids = STRIPE.map((t) => t.id) as readonly string[];
    if (ids.includes(h)) showTool(h as SideTool);
    else if (["settings", "player", "ide", "palette", "diff"].includes(h)) tourAction(h as TourAction);
    else if (h === "readme") setActiveGroup("grp-web-readme");
    else if (h === "dev") setActiveGroup("grp-web-dev");
    else if (h === "agents-demo") tourAction("agents");
    else if (h === "notes") setSelectedProject("field-notes");
    else if (h === "soundlab") setSelectedProject("soundlab");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /* --------------------------- keyboard --------------------------- */
  /* JetBrains keymap, as in the app: tool keys toggle hidden → show, shown → hide. */
  /* Registered once; the ref always holds this render's handler. */
  const lastShift = useRef(0);
  const onKeyRef = useRef<(e: KeyboardEvent) => void>(() => {});
  onKeyRef.current = (e: KeyboardEvent) => {
    const meta = e.metaKey || e.ctrlKey;
    if (e.key === "Shift" && !e.repeat && !meta && !e.altKey) {
      const now = performance.now();
      if (now - lastShift.current < 350) openPalette("all");
      lastShift.current = now;
      return;
    }
    if (e.altKey && !meta && e.code === "F12") {
      e.preventDefault();
      setIdeMode((v) => !v);
      return;
    }
    if (!meta) {
      if (e.key === "Escape") {
        setMenu(null);
        setPaletteOpen(false);
      }
      return;
    }
    const k = e.key.toLowerCase();
    const digit = /^Digit(\d)$/.exec(e.code)?.[1];
    if (digit && !e.shiftKey) {
      const t = TOOL_KEYS[digit];
      if (t) {
        e.preventDefault();
        toggleTool(t);
      }
    } else if (e.shiftKey && e.code === "KeyO") {
      e.preventDefault();
      openPalette("files");
    } else if (e.shiftKey && e.code === "KeyA") {
      e.preventDefault();
      openPalette("actions");
    } else if (e.code === "KeyE" && !e.altKey && !e.shiftKey) {
      e.preventDefault();
      openPalette("recent");
    } else if (k === "k" || k === "p") {
      e.preventDefault();
      openPalette("all");
    } else if (k === ",") {
      e.preventDefault();
      setCentral("settings");
    } else if (e.code === "KeyE" && e.altKey) {
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
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => onKeyRef.current(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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
      { kind: "item", label: "Select all", icon: "TextSelect", run: () => setNote("Selected all scrollback.") },
      { kind: "item", label: "Paste", icon: "ClipboardPaste", shortcut: `${mod}V`, run: () => setNote("Paste goes to the shell. This demo shell is read-only.") },
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
      { kind: "item", label: "Clear saved scrollback", icon: "Eraser", destructive: true, run: () => setNote("Saved scrollback cleared. The session keeps running.") },
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
      { kind: "item", label: "New task worktree…", icon: "GitBranch", run: () => newTask() },
      { kind: "sep" },
      { kind: "item", label: "Remove project from sidebar", icon: "X", destructive: true, run: () => setNote("Removed from the sidebar. Its sessions keep running in the daemon.") },
    ]);
  };

  const sessionMenu = (e: React.MouseEvent, s: Session) => {
    openMenu(e, [
      { kind: "item", label: "Rename…", icon: "Pencil", run: () => setNote("Rename: type a new label inline.") },
      { kind: "sep" },
      { kind: "item", label: "Open session", icon: "Terminal", run: () => goSession(s.id) },
      { kind: "sep" },
      { kind: "item", label: "Close session…", icon: "X", destructive: true, run: () => setNote(`Close ${s.label}: keep it running in the background, or terminate it.`) },
    ]);
  };

  const fileMenu = (e: React.MouseEvent, f: FileNode) => {
    openMenu(e, [
      { kind: "item", label: "Open", icon: "ExternalLink", run: () => openFile(f) },
      { kind: "item", label: "Open to the side", icon: "Columns2", run: () => { openFile(f); } },
      { kind: "sep" },
      { kind: "item", label: "Copy path", icon: "Copy", run: () => navigator.clipboard?.writeText(f.path) },
      { kind: "item", label: "Reveal in file manager", icon: "FolderOpen", run: () => setNote(`Revealed ${f.path} in the file manager.`) },
      { kind: "item", label: "Rename…", icon: "Pencil", run: () => setNote("Rename stages every reference and writes all files or none. ⌘Z undoes it.") },
      { kind: "item", label: "Delete", icon: "X", destructive: true, run: () => setNote(`Moved ${f.name} to the Trash. ⌘Z undoes it.`) },
    ]);
  };

  /* --------------------------- guided tour --------------------------- */
  const tourAction = (action: TourAction) => {
    switch (action) {
      case "agents":
        setSelectedProject("atlas-api");
        setWorkspaces((w) => ({ ...w, "atlas-api": { ...w["atlas-api"], active: "grp-api-agents" } }));
        showTool("agents");
        break;
      case "diff":
        openDiff("src/app.ts");
        break;
      case "split":
        setCentral("dock");
        splitFocused("row");
        break;
      case "palette":
        openPalette("all");
        break;
      case "player":
        setSelectedProject("soundlab");
        setCentral("player");
        break;
      case "settings":
        setCentral("settings");
        break;
      case "ide":
        setIdeMode(true);
        setCentral("dock");
        break;
      case "localhistory":
        openInNewTab({ kind: "localhistory", path: "src/app.ts" });
        break;
      case "hosts":
      case "vnc":
      case "problems":
      case "git":
      case "history":
        showTool(action);
        break;
      case "browser":
        openInNewTab({ kind: "browser", url: "docs.html" });
        break;
      case "keys":
        openPalette("files");
        break;
      case "task":
        newTask();
        break;
      case "markdown":
        setSelectedProject("field-notes");
        setWorkspaces((w) => ({ ...w, "field-notes": { ...w["field-notes"], active: "grp-notes-md" } }));
        setCentral("dock");
        break;
      case "projects":
        setLeftVisible(true);
        break;
      default:
        break;
    }
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
            <button className="bar-btn" title="New terminal in this pane" onClick={(e) => { e.stopPropagation(); const ns = newSession(selectedProject); addGroup({ kind: "term", sid: ns.id }); }}>
              <Icon name="Plus" size={14} />
            </button>
            <button className="bar-btn" title="Split right" onClick={(e) => { e.stopPropagation(); focus(); splitFocused("row"); }}>
              <Icon name="Columns2" size={14} />
            </button>
            <button className="bar-btn" title="Split down" onClick={(e) => { e.stopPropagation(); focus(); splitFocused("col"); }}>
              <Icon name="Rows2" size={14} />
            </button>
            <button className="bar-btn" title={maxLeaf === l.id ? "Restore pane" : "Maximize pane"} onClick={(e) => { e.stopPropagation(); focus(); setMaxLeaf((m) => (m === l.id ? null : l.id)); }}>
              <Icon name="Maximize2" size={13} />
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
      <section className={`pane${foc ? " foc" : ""}${c.kind === "tour" ? " tour" : ""}`} onClick={focus}>
        {c.kind === "tour" && <TourView onClose={onClose} onAction={tourAction} />}
        {c.kind === "markdown" && <MarkdownView path={c.path} content={openFileContent(c.path)} onClose={onClose} />}
        {c.kind === "diff" && <DiffView path={c.path} onClose={onClose} onFile={openDiff} />}
        {c.kind === "browser" && <BrowserView url={c.url} onClose={onClose} />}
        {c.kind === "localhistory" && <LocalHistoryView path={c.path} onClose={onClose} onNote={setNote} />}
      </section>
    );
  };

  /* A maximized pane replaces its group's whole split tree. */
  const renderGroup = (g: Group) => {
    const maxed = maxLeaf ? leaves(g.layout).find((x) => x.id === maxLeaf) : undefined;
    return maxed ? renderLeaf(maxed, g.id) : renderLayout(g.layout, g.id);
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
    const l = leaves(group.layout).find((x) => x.id === group.activeLeaf);
    const sid = l && l.content.kind === "term" ? l.content.sid : sessions.find((s) => s.projectId === selectedProject)?.id;
    return sessions.find((s) => s.id === sid) ?? sessions[0];
  }, [group, sessions, selectedProject]);

  const headerToolMenu = (e: React.MouseEvent) => {
    e.stopPropagation();
    openMenu(e, HEADER_HIDDEN.map((t) => ({ kind: "item", label: t.label, icon: t.icon, checked: tool === t.id, run: () => selectTool(t.id) })));
  };

  const panelFor = (t: SideTool): React.ReactNode => {
    switch (t) {
      case "projects":
        return (
          <ProjectsTree
            projects={projects}
            sessions={sessions}
            selected={selectedProject}
            onSelect={(id) => { setSelectedProject(id); setCentral("dock"); }}
            onOpenSession={goSession}
            onNewTerminal={() => addGroup({ kind: "term", sid: newSession(selectedProject).id })}
          />
        );
      case "explorer":
        return <ExplorerPanel root={`~/code/${selectedProject}`} tree={treesByProject[selectedProject] ?? []} onOpenFile={openFile} />;
      case "agents":
        return <AgentsPanel />;
      case "git":
        return <GitPanel onOpenDiff={openDiff} />;
      case "history":
        return <HistoryPanel />;
      case "info":
        return <InfoPanel session={activeSession} />;
      case "problems":
        return <ProblemsPanel onOpen={(path) => openInNewTab({ kind: "editor", path })} />;
      case "structure":
        return <StructurePanel />;
      case "vnc":
        return <VncPanel />;
      case "hosts":
        return <HostsPanel />;
    }
  };

  const projectNavMenu = (e: React.MouseEvent) => {
    e.stopPropagation();
    openMenu(e, [
      { kind: "header", label: "terminator 0.159.0" },
      { kind: "sep" },
      { kind: "item", label: "Download latest release", icon: "ArrowDown", run: () => window.open(DOWNLOAD, "_blank") },
      { kind: "item", label: "Star on GitHub", icon: "ExternalLink", run: () => window.open(REPO, "_blank") },
      { kind: "sep" },
      { kind: "item", label: "Check for updates", icon: "RefreshCw", run: () => setNote("Terminator 0.159.0 is up to date. Updates are checked every minute.") },
    ]);
  };

  return (
    <div className={`app${compact ? " compact" : ""}`} onContextMenu={(e) => e.preventDefault()}>
      {/* header */}
      <header className="titlebar">
        <div className="titlebar-left">
          <div className="traffic">
            <i />
            <i />
            <i />
          </div>
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
          <button className="proj-name" title="Switch project" onClick={projectNavMenu}>
            {projects.find((p) => p.id === selectedProject)?.name ?? "terminator"} <span className="caret">▾</span>
          </button>
          <button className="proj-branch" title="Branches" onClick={() => { setTool("git"); setRightVisible(true); setCentral("dock"); }}>
            <Icon name="GitBranch" size={14} />
            {projects.find((p) => p.id === selectedProject)?.branch}
          </button>
          <button className="ic-btn" title="Search everywhere (double Shift)" onClick={() => openPalette("all")}>
            <Icon name="Search" size={15} />
          </button>
        </div>
        <div className="titlebar-drag" />
        <div className={`titlebar-tools${rightVisible ? " pinned" : ""}`}>
          {HEADER_VISIBLE.map((t) => (
            <button
              key={t.id}
              className={`ic-btn${(t.id === "ide" && ideMode) || (t.id === tool && rightVisible) ? " on" : ""}`}
              title={t.label}
              onClick={() => selectTool(t.id)}
            >
              <Icon name={t.icon} size={16} />
            </button>
          ))}
          {HEADER_HIDDEN.length > 0 && (
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
        <ToolStripe
          items={STRIPE}
          active={leftVisible ? leftTool : null}
          onPick={(id) => {
            if (id === leftTool && leftVisible) setLeftVisible(false);
            else { setLeftTool(id); setLeftVisible(true); }
          }}
          onSearch={() => openPalette("all")}
        />
        <aside className={`sidebar left${leftVisible ? "" : " hidden"}`} data-tool={leftTool}>
          {panelFor(leftTool)}
        </aside>

        <main className="center">
          {central === "settings" ? (
            <SettingsView accent={accent} setAccent={setAccent} compact={compact} setCompact={setCompact} />
          ) : central === "player" ? (
            <PlayerView />
          ) : ideMode ? (
            <div className="ide">
              <div className="ide-top">{group && renderGroup(group)}</div>
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
                  <button className="tplus" title="All tabs" onClick={(e) => openMenu(e, groups.map((g) => { const m = meta(firstLeaf(g.layout).content, sessions); return { kind: "item" as const, label: m.label, icon: m.icon, checked: g.id === activeGroup, run: () => setActiveGroup(g.id) }; }))}>
                    <Icon name="ChevronDown" size={15} />
                  </button>
                  <button className="tplus" title="Recent files (⌘E)" onClick={() => openPalette("recent")}>
                    <Icon name="History" size={15} />
                  </button>
                </div>
                <span className="tstrip-drag" />
              </div>
              <div className="display">{group && renderGroup(group)}</div>
            </>
          )}
        </main>

        <aside className={`sidebar right${rightVisible ? "" : " hidden"}`} data-tool={tool}>
          {panelFor(tool)}
        </aside>
        <ToolStripe
          items={STRIPE}
          active={rightVisible ? tool : null}
          onPick={toggleTool}
          onSearch={() => openPalette("all")}
        />
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
        <span className="vsep" />
        <span className="crumbs">
          {selectedProject} <span className="weak">›</span> {group ? meta(firstLeaf(group.layout).content, sessions).label : ""}
          <Icon name="Bell" size={13} />
        </span>
        {note && <span className="status-note">{note}</span>}
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

      {paletteOpen && <Palette commands={paletteCommands} title={PALETTE_TITLE[paletteMode]} close={() => setPaletteOpen(false)} />}
      {menu && <CtxMenu menu={menu} close={() => setMenu(null)} />}
    </div>
  );
}

