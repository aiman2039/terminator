import { useEffect, useMemo, useState } from "react";
import { AgentsPane, DemoFor, DummyTerm } from "./demos";
import { CtxMenu, type MenuItem, type MenuState } from "./menu";

const REPO = "https://github.com/aiman2039/terminator";
const DL = `${REPO}/releases/latest`;

type Block = {
  id: string;
  tag: string;
  title: string;
  text: string;
  shot: string;
  alt: string;
  points?: string[];
};

const chapters: { id: string; name: string; blocks: Block[] }[] = [
  {
    id: "workspace",
    name: "Workspace",
    blocks: [
      {
        id: "projects",
        tag: "// projects, tabs, splits",
        title: "One workspace per project.",
        text: "Each project owns its tabs and split layouts. Right-click a terminal to split in any direction. Sessions live in a background daemon, so closing the app never kills them.",
        shot: "workspace",
        alt: "Two split terminals with project tree and file sidebar",
        points: ["Reconnect to running sessions after restart", "Sort and filter projects", "Rename, reorder, and close tabs in bulk"],
      },
      {
        id: "agents",
        tag: "// agent inbox",
        title: "Know the moment an agent needs you.",
        text: "Hooks for Claude Code, Codex, OpenCode, Muse, and Grok report real lifecycle events. Needs input, needs permission, completed, failed. One card per agent run, with go, snooze, and dismiss.",
        shot: "agents",
        alt: "Agents inbox with a waiting agent card",
        points: ["Never guessed from terminal text", "Waiting count badge on the bell", "Nothing launches without you"],
      },
      {
        id: "history",
        tag: "// history",
        title: "Every session, searchable.",
        text: "Revisit past terminals and their scrollback. Search terminal output, then jump back to the exact session.",
        shot: "history",
        alt: "History sidebar",
      },
      {
        id: "info",
        tag: "// info panel",
        title: "Session details at a glance.",
        text: "See the process tree, working directory, and live system load for the active session.",
        shot: "info",
        alt: "Info sidebar with process and resource details",
      },
    ],
  },
  {
    id: "editing",
    name: "Editing",
    blocks: [
      {
        id: "nvim",
        tag: "// neovim",
        title: "Your Neovim, your config.",
        text: "Open any file in Neovim inside the workspace. It loads your own configuration. Prefer another editor? Pick an external one in Settings.",
        shot: "nvim",
        alt: "Neovim editing beside a terminal",
        points: ["Unsaved-change guard on close", "Editors run in the daemon and survive restarts"],
      },
      {
        id: "markdown",
        tag: "// markdown",
        title: "Edit with a live preview.",
        text: "Switch between Edit, Preview, and Split. The preview renders headings, checklists, tables, and code.",
        shot: "markdown",
        alt: "Markdown split view",
      },
      {
        id: "explorer",
        tag: "// file explorer",
        title: "Files open the right way.",
        text: "Names and Contents search, ignore toggles, and default viewers: images preview, HTML opens in a browser tab, Markdown previews, audio plays. Open as text bypasses any viewer.",
        shot: "workspace",
        alt: "File explorer with Git status markers",
      },
    ],
  },
  {
    id: "git",
    name: "Git",
    blocks: [
      {
        id: "git-sidebar",
        tag: "// git sidebar",
        title: "Changes grouped and clickable.",
        text: "Staged, working, and untracked files in one list. Click a changed file to open its diff.",
        shot: "git",
        alt: "Git sidebar with changed files",
      },
      {
        id: "git-diff",
        tag: "// diff review",
        title: "Native diffs. Neovim CodeDiff if you want it.",
        text: "A native side-by-side viewer by default. Switch to the bundled Neovim CodeDiff in Settings. Reviews are read-only snapshots: HEAD to index, index to disk. They never touch your repo.",
        shot: "git-diff",
        alt: "Native diff view of a modified file",
      },
    ],
  },
  {
    id: "ide",
    name: "IDE mode",
    blocks: [
      {
        id: "ide-mode",
        tag: "// ide mode",
        title: "Flip to IDE layout with one key.",
        text: "Pin both sidebars, keep the editor in the center, and dock a full terminal strip at the bottom. Toggle with Cmd+E or the command palette. Player and agent bell move into the status bar.",
        shot: "ide",
        alt: "IDE mode with editor above and terminal strip below",
        points: ["Strip has tabs, splits, and drag reorder", "Full-height or full-width sidebars", "Saved in named layout presets"],
      },
    ],
  },
  {
    id: "media",
    name: "Audio",
    blocks: [
      {
        id: "player",
        tag: "// player + radio",
        title: "A built-in player. And internet radio.",
        text: "Open mp3, flac, ogg, wav, m4a, opus, or aac. A Winamp-style deck with spectrum, EQ, shuffle, repeat, and named playlists. Switch to the bundled Icecast and Shoutcast catalog, or add your own stations. Music keeps playing while you work.",
        shot: "player",
        alt: "Player with equalizer and playlist",
      },
    ],
  },
  {
    id: "notify",
    name: "Notifications",
    blocks: [
      {
        id: "notifications",
        tag: "// alerts everywhere",
        title: "In app, on the desktop, and on your phone.",
        text: "Choose per event: in app, OS when unfocused, or both. Add a sound, a macOS menu-bar badge with a pending list, or push to your phone through ntfy. Bursts are time-gated so your phone buzzes once.",
        shot: "settings-notifications",
        alt: "Notification settings including ntfy channel",
        points: ["ntfy sends only agent and status, never prompt text", "Click a notification to focus its terminal"],
      },
      {
        id: "hooks",
        tag: "// agent hooks",
        title: "Explicit, reversible setup.",
        text: "Install hooks from Settings. Existing config is preserved and backed up. Repair or remove at any time.",
        shot: "settings-hooks",
        alt: "Agent hooks settings",
      },
      {
        id: "settings",
        tag: "// customize",
        title: "Make it yours.",
        text: "Colors, fonts, shortcuts, shell, history retention, and update checks. Settings open in the main pane, not a popup.",
        shot: "settings",
        alt: "Appearance settings",
      },
    ],
  },
];

type TabKind = "term" | "doc" | "agents";
type Tab = { id: string; title: string; name?: string; kind: TabKind; block?: Block };
const tabLabel = (t: Tab) => t.name ?? t.title;
type Pane = { id: string; tabIds: string[]; active: string };
type FloatWin = { id: string; tabId: string; x: number; y: number };

let n = 0;
const nid = (p: string) => `${p}-${++n}`;

const blockTab = (b: Block): Tab => ({ id: `doc-${b.id}`, title: b.id, kind: "doc", block: b });

function DocView({ block }: { block: Block }) {
  return (
    <div className="docview">
      <p className="dtag">{block.tag} — live demo, not a screenshot</p>
      <h2>{block.title}</h2>
      <p className="dtext">{block.text}</p>
      {block.points && (
        <ul className="dpoints">
          {block.points.map((p) => (
            <li key={p}>{p}</li>
          ))}
        </ul>
      )}
      <DemoFor id={block.id} />
      <p className="dsub">
        <a className="btn" href={DL}>Download</a> <a className="glink" href={REPO}>Star on GitHub</a>
      </p>
    </div>
  );
}

const tabById = (tabs: Tab[], id: string) => tabs.find((t) => t.id === id) ?? tabs[0];

function FloatWinView({
  x,
  y,
  title,
  onMove,
  onDock,
  onClose,
  onHeaderMenu,
  children,
}: {
  x: number;
  y: number;
  title: string;
  onMove: (x: number, y: number) => void;
  onDock: () => void;
  onClose: () => void;
  onHeaderMenu: (e: React.MouseEvent) => void;
  children: React.ReactNode;
}) {
  const [drag, setDrag] = useState<{ dx: number; dy: number } | null>(null);
  useEffect(() => {
    if (!drag) return;
    const move = (e: PointerEvent) => {
      onMove(
        Math.max(0, Math.min(e.clientX - drag.dx, window.innerWidth - 240)),
        Math.max(0, Math.min(e.clientY - drag.dy, window.innerHeight - 120)),
      );
    };
    const up = () => setDrag(null);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [drag, onMove]);
  return (
    <div className="float" style={{ left: x, top: y }}>
      <div
        className="float-h"
        onPointerDown={(e) => {
          if ((e.target as HTMLElement).closest("button")) return;
          setDrag({ dx: e.clientX - x, dy: e.clientY - y });
        }}
        onContextMenu={onHeaderMenu}
      >
        <span className="ptitle">{title}</span>
        <span className="pbtns">
          <button title="Dock back" onClick={onDock}>⇲</button>
          <button title="Close" onClick={onClose}>×</button>
        </span>
      </div>
      <div className="float-b">{children}</div>
    </div>
  );
}

export default function App() {
  const [tabs, setTabs] = useState<Tab[]>(() => {
    const agents = chapters[0].blocks[1];
    return [
      { id: "term-quick", title: "quickstart", kind: "term" },
      { id: `doc-${agents.id}`, title: agents.id, kind: "doc", block: agents },
      { id: "agents-live", title: "inbox", kind: "agents" },
    ];
  });
  const [panes, setPanes] = useState<Pane[]>([
    { id: "pane-a", tabIds: ["term-quick"], active: "term-quick" },
    { id: "pane-b", tabIds: ["doc-agents"], active: "doc-agents" },
  ]);
  const [floats, setFloats] = useState<FloatWin[]>([]);
  const [focus, setFocus] = useState("pane-a");
  const [termCount, setTermCount] = useState(1);
  const [filter, setFilter] = useState("");
  const [find, setFind] = useState("");
  const [left, setLeft] = useState(true);
  const [right, setRight] = useState(true);
  const [expanded, setExpanded] = useState<string[]>(chapters.map((c) => c.id));

  const focusedPane = panes.find((p) => p.id === focus) ?? panes[0];
  const activeTabId = focusedPane?.active;

  const showInPane = (paneId: string, tabId: string) => {
    setPanes((ps) =>
      ps.map((p) =>
        p.id === paneId
          ? {
              ...p,
              tabIds: p.tabIds.includes(tabId) ? p.tabIds : [...p.tabIds, tabId],
              active: tabId,
            }
          : p,
      ),
    );
  };

  const focusTabInPane = (paneId: string, tabId: string) => {
    setFocus(paneId);
    showInPane(paneId, tabId);
  };

  const openBlock = (b: Block) => {
    const t = blockTab(b);
    setTabs((ts) => (ts.some((x) => x.id === t.id) ? ts : [...ts, t]));
    showInPane(focus, t.id);
  };

  const newTab = (paneId: string = focus) => {
    const id = `term-${termCount + 1}`;
    setTermCount((c) => c + 1);
    const t: Tab = { id, title: `Terminal ${termCount + 1}`, kind: "term" };
    setTabs((ts) => [...ts, t]);
    showInPane(paneId, id);
  };

  const closeTab = (id: string) => closeTabs([id]);

  const splitPane = (paneId: string) => {
    const pane = panes.find((p) => p.id === paneId);
    if (!pane) return;
    const id = nid("pane");
    setPanes((ps) => {
      const i = ps.findIndex((p) => p.id === paneId);
      return [...ps.slice(0, i + 1), { id, tabIds: [pane.active], active: pane.active }, ...ps.slice(i + 1)];
    });
    setFocus(id);
  };

  const closePane = (paneId: string) => {
    if (panes.length <= 1) return;
    setPanes((ps) => ps.filter((p) => p.id !== paneId));
    if (focus === paneId) setFocus(panes.find((p) => p.id !== paneId)?.id ?? panes[0].id);
  };

  const visibleChapters = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return chapters;
    return chapters
      .map((c) => ({
        ...c,
        blocks: c.blocks.filter((b) => `${b.id} ${b.title}`.toLowerCase().includes(q)),
      }))
      .filter((c) => c.name.toLowerCase().includes(q) || c.blocks.length > 0);
  }, [filter]);

  const rightFiles = useMemo(() => {
    const all = chapters.flatMap((c) => c.blocks);
    const q = find.trim().toLowerCase();
    return q ? all.filter((b) => `${b.id} ${b.title}`.toLowerCase().includes(q)) : all;
  }, [find]);

  const toggleExpand = (id: string) =>
    setExpanded((e) => (e.includes(id) ? e.filter((x) => x !== id) : [...e, id]));

  const [menu, setMenu] = useState<MenuState | null>(null);
  const [editingTab, setEditingTab] = useState<string | null>(null);
  const [clears, setClears] = useState<Record<string, number>>({});
  const mod = typeof navigator !== "undefined" && /Mac/.test(navigator.platform ?? "") ? "⌘" : "Ctrl+Shift+";

  useEffect(() => {
    if (!menu) return;
    const dismiss = (e: PointerEvent) => {
      if (!(e.target as HTMLElement).closest(".ctx")) setMenu(null);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMenu(null);
        setEditingTab(null);
      }
    };
    const onResize = () => setMenu(null);
    window.addEventListener("pointerdown", dismiss, true);
    window.addEventListener("keydown", esc);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("pointerdown", dismiss, true);
      window.removeEventListener("keydown", esc);
      window.removeEventListener("resize", onResize);
    };
  }, [menu]);

  const openMenu = (e: React.MouseEvent, items: MenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ x: e.clientX, y: e.clientY, items });
  };

  const commitRename = (id: string, name: string) => {
    const clean = name.trim();
    setTabs((ts) => ts.map((t) => (t.id === id ? { ...t, name: clean || undefined } : t)));
    setEditingTab(null);
  };

  const closeTabs = (ids: string[]) => {
    const gone = new Set(ids);
    setTabs((ts) => {
      const next = ts.filter((t) => !gone.has(t.id));
      const list: Tab[] =
        next.length > 0 ? next : [{ id: "term-1", title: "Terminal 1", kind: "term" }];
      const live = new Set(list.map((t) => t.id));
      setFloats((fs) => fs.filter((f) => live.has(f.tabId)));
      setPanes((ps) => {
        const kept = ps
          .map((p) => {
            const ids2 = p.tabIds.filter((id) => live.has(id));
            return {
              ...p,
              tabIds: ids2,
              active: live.has(p.active) ? p.active : ids2[0],
            };
          })
          .filter((p) => p.tabIds.length > 0);
        if (kept.length > 0) {
          if (!kept.some((p) => p.id === focus)) setFocus(kept[0].id);
          return kept;
        }
        return [{ id: "pane-a", tabIds: [list[0].id], active: list[0].id }];
      });
      return list;
    });
  };

  const addTabAt = (index: number) => {
    const id = `term-${termCount + 1}`;
    setTermCount((c) => c + 1);
    const t: Tab = { id, title: `Terminal ${termCount + 1}`, kind: "term" };
    setTabs((ts) => [...ts.slice(0, index), t, ...ts.slice(index)]);
    showInPane(focus, id);
  };

  const allBlocks = chapters.flatMap((c) => c.blocks);
  const explorerBlock = allBlocks.find((b) => b.id === "explorer");
  const historyBlock = allBlocks.find((b) => b.id === "history");

  const openBlockInPane = (paneId: string, b: Block) => {
    const t = blockTab(b);
    setTabs((ts) => (ts.some((x) => x.id === t.id) ? ts : [...ts, t]));
    showInPane(paneId, t.id);
  };

  const openBlockSplit = (b: Block) => {
    const t = blockTab(b);
    const id = nid("pane");
    setTabs((ts) => (ts.some((x) => x.id === t.id) ? ts : [...ts, t]));
    setPanes((ps) => {
      const i = ps.findIndex((p) => p.id === focus);
      return [...ps.slice(0, i + 1), { id, tabIds: [t.id], active: t.id }, ...ps.slice(i + 1)];
    });
    setFocus(id);
  };

  const detachPane = (paneId: string) => {
    const pane = panes.find((p) => p.id === paneId);
    const tab = pane ? tabs.find((t) => t.id === pane.active) : undefined;
    if (!tab) return;
    const id = nid("tab");
    const copy: Tab = { ...tab, id, title: `${tabLabel(tab)}+` };
    setTabs((ts) => [...ts, copy]);
    setPanes((ps) =>
      ps.map((p) =>
        p.id === paneId
          ? {
              ...p,
              tabIds: p.tabIds.map((tid) => (tid === tab.id ? copy.id : tid)),
              active: copy.id,
            }
          : p,
      ),
    );
  };

  const moveActiveToPane = (srcId: string, dstId: string) => {
    const src = panes.find((p) => p.id === srcId);
    if (!src) return;
    const tid = src.active;
    setPanes((ps) => {
      const withMove = ps.map((p) =>
        p.id === dstId
          ? {
              ...p,
              tabIds: p.tabIds.includes(tid) ? p.tabIds : [...p.tabIds, tid],
              active: tid,
            }
          : p,
      );
      const left = withMove.find((p) => p.id === srcId)?.tabIds.filter((t) => t !== tid) ?? [];
      if (left.length === 0) {
        const next = withMove.filter((p) => p.id !== srcId);
        if (focus === srcId) setFocus(dstId);
        return next;
      }
      return withMove.map((p) =>
        p.id === srcId
          ? { ...p, tabIds: left, active: left.includes(p.active) ? p.active : left[0] }
          : p,
      );
    });
  };

  const floatActive = (paneId: string) => {
    const pane = panes.find((p) => p.id === paneId);
    if (!pane) return;
    const tid = pane.active;
    const id = nid("float");
    setFloats((fs) => [...fs, { id, tabId: tid, x: 120 + fs.length * 28, y: 90 + fs.length * 28 }]);
    setPanes((ps) => {
      const left = pane.tabIds.filter((t) => t !== tid);
      if (left.length === 0) {
        if (ps.length <= 1) {
          const fresh: Tab = { id: nid("term"), title: "Terminal", kind: "term" };
          setTabs((ts) => [...ts, fresh]);
          return ps.map((p) =>
            p.id === paneId ? { ...p, tabIds: [fresh.id], active: fresh.id } : p,
          );
        }
        const next = ps.filter((p) => p.id !== paneId);
        if (focus === paneId) setFocus(next[0].id);
        return next;
      }
      return ps.map((p) =>
        p.id === paneId
          ? { ...p, tabIds: left, active: left.includes(p.active) ? p.active : left[0] }
          : p,
      );
    });
  };

  const dockFloat = (id: string) => {
    const f = floats.find((w) => w.id === id);
    if (!f) return;
    showInPane(focus, f.tabId);
    setFloats((fs) => fs.filter((w) => w.id !== id));
  };

  const closeFloat = (id: string) => {
    setFloats((fs) => fs.filter((w) => w.id !== id));
  };

  const floatMenu = (e: React.MouseEvent, id: string) => {
    openMenu(e, [
      { kind: "item", label: "Dock back", run: () => dockFloat(id) },
      { kind: "item", label: "Close float", destructive: true, run: () => closeFloat(id) },
    ]);
  };

  const splitItems = (paneId: string): MenuItem[] => [
    { kind: "item", label: "New tab", run: () => newTab(paneId) },
    { kind: "item", label: "Split up", run: () => splitPane(paneId) },
    { kind: "item", label: "Split down", run: () => splitPane(paneId) },
    { kind: "item", label: "Split left", run: () => splitPane(paneId) },
    { kind: "item", label: "Split right", run: () => splitPane(paneId) },
  ];

  const stackSub = (paneId: string): MenuItem[] => {
    const stack = panes.find((p) => p.id === paneId);
    if (!stack || stack.tabIds.length <= 1) return [];
    return [
      {
        kind: "sub",
        label: "Tabs in this pane",
        items: stack.tabIds.map((tid) => {
          const st = tabs.find((t) => t.id === tid);
          return {
            kind: "item",
            label: st ? tabLabel(st) : tid,
            run: () => focusTabInPane(paneId, tid),
          } as MenuItem;
        }),
      } as MenuItem,
    ];
  };

  const moveSub = (paneId: string): MenuItem[] => {
    const others = panes.filter((p) => p.id !== paneId);
    if (others.length === 0) return [];
    return [
      {
        kind: "sub",
        label: "Move to pane",
        items: others.map((p) => {
          const st = tabs.find((t) => t.id === p.active);
          return {
            kind: "item",
            label: st ? tabLabel(st) : p.id,
            run: () => moveActiveToPane(paneId, p.id),
          } as MenuItem;
        }),
      } as MenuItem,
    ];
  };

  const termMenu = (
    e: React.MouseEvent,
    paneId: string,
    tab: Tab,
    scopeOverride?: HTMLElement | null,
  ) => {
    const scope = scopeOverride ?? (e.currentTarget as HTMLElement).closest("[data-pane]");
    const selection = window.getSelection();
    const text = selection?.toString() ?? "";
    const inScope =
      !!scope && !!text && !!selection?.anchorNode && scope.contains(selection.anchorNode);
    const items: MenuItem[] = [
      { kind: "header", label: "~/terminator" },
      { kind: "sep" },
      {
        kind: "item",
        label: "Copy",
        shortcut: `${mod}C`,
        disabled: !inScope,
        run: () => {
          if (text) void navigator.clipboard?.writeText(text).catch(() => {});
        },
      },
      {
        kind: "item",
        label: "Select all",
        run: () => {
          const tty = scope?.querySelector("[data-term]");
          const s = window.getSelection();
          if (tty && s) {
            const r = document.createRange();
            r.selectNodeContents(tty);
            s.removeAllRanges();
            s.addRange(r);
          }
        },
      },
      {
        kind: "item",
        label: "Paste",
        shortcut: `${mod}V`,
        run: () => {
          const input = scope?.querySelector(".tin input") as HTMLInputElement | null;
          input?.focus();
          void navigator.clipboard
            ?.readText()
            .then((t) => {
              if (input && t) {
                if (!document.execCommand("insertText", false, t)) input.value += t;
              }
            })
            .catch(() => {});
        },
      },
      { kind: "sep" },
      ...splitItems(paneId),
      ...stackSub(paneId),
      { kind: "sep" },
      {
        kind: "item",
        label: "Open file path…",
        run: () => {
          if (explorerBlock) openBlockInPane(paneId, explorerBlock);
        },
      },
      {
        kind: "item",
        label: "Search scrollback",
        run: () => {
          if (historyBlock) openBlockInPane(paneId, historyBlock);
        },
      },
      {
        kind: "item",
        label: "Copy working directory",
        run: () => {
          void navigator.clipboard?.writeText("~/terminator").catch(() => {});
        },
      },
      { kind: "sep" },
      ...(tab.kind === "term"
        ? [
            {
              kind: "item",
              label: "Rename…",
              run: () => setEditingTab(tab.id),
            } as MenuItem,
          ]
        : []),
      {
        kind: "item",
        label: "Close session…",
        destructive: true,
        run: () => closeTab(tab.id),
      },
    ];
    openMenu(e, items);
  };

  const paneMenu = (e: React.MouseEvent, paneId: string, tab: Tab) => {
    const items: MenuItem[] = [
      ...(tab.kind === "term"
        ? [
            {
              kind: "item",
              label: "Rename…",
              run: () => setEditingTab(tab.id),
            } as MenuItem,
            { kind: "sep" } as MenuItem,
          ]
        : []),
      ...splitItems(paneId),
      ...stackSub(paneId),
      { kind: "sep" },
      { kind: "item", label: "Detach to new tab", run: () => detachPane(paneId) },
      { kind: "item", label: "Float window", run: () => floatActive(paneId) },
      ...moveSub(paneId),
      ...(tab.kind === "term"
        ? ([
            { kind: "sep" },
            {
              kind: "item",
              label: "Search scrollback",
              run: () => {
                if (historyBlock) openBlockInPane(paneId, historyBlock);
              },
            },
            {
              kind: "item",
              label: "Clear saved scrollback",
              destructive: true,
              run: () =>
                setClears((m) => ({ ...m, [tab.id]: (m[tab.id] ?? 0) + 1 })),
            },
          ] as MenuItem[])
        : []),
    ];
    openMenu(e, items);
  };

  const tabMenu = (e: React.MouseEvent, tab: Tab, index: number) => {
    const count = tabs.length;
    const items: MenuItem[] = [
      ...(tab.kind === "term"
        ? [
            {
              kind: "item",
              label: "Rename…",
              run: () => setEditingTab(tab.id),
            } as MenuItem,
          ]
        : []),
      { kind: "item", label: "Close tab…", destructive: true, run: () => closeTab(tab.id) },
      { kind: "sep" },
      {
        kind: "item",
        label: "Close all tabs…",
        destructive: true,
        disabled: count <= 1,
        run: () => closeTabs(tabs.map((t) => t.id)),
      },
      {
        kind: "item",
        label: "Close all tabs to the left…",
        destructive: true,
        disabled: index <= 0,
        run: () => closeTabs(tabs.slice(0, index).map((t) => t.id)),
      },
      {
        kind: "item",
        label: "Close all tabs to the right…",
        destructive: true,
        disabled: index + 1 >= count,
        run: () => closeTabs(tabs.slice(index + 1).map((t) => t.id)),
      },
      { kind: "sep" },
      { kind: "item", label: "Add tab to the left", run: () => addTabAt(index) },
      { kind: "item", label: "Add tab to the right", run: () => addTabAt(index + 1) },
    ];
    openMenu(e, items);
  };

  const sideMenu = (e: React.MouseEvent, b: Block) => {
    const id = `doc-${b.id}`;
    const isOpen = tabs.some((t) => t.id === id);
    openMenu(e, [
      { kind: "item", label: "Open in focused pane", run: () => openBlock(b) },
      { kind: "item", label: "Open in new split", run: () => openBlockSplit(b) },
      {
        kind: "item",
        label: "Close tab…",
        destructive: true,
        disabled: !isOpen,
        run: () => closeTab(id),
      },
    ]);
  };

  const fileMenu = (e: React.MouseEvent, b: Block) => {
    openMenu(e, [
      { kind: "item", label: "Open", run: () => openBlock(b) },
      {
        kind: "item",
        label: "Copy path",
        run: () => {
          void navigator.clipboard?.writeText(`~/terminator/${b.id}.md`).catch(() => {});
        },
      },
    ]);
  };

  const dockMenu = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest(".pane")) return;
    openMenu(e, splitItems(focus));
  };

  const tabBody = (tab: Tab, termCtx: (e: React.MouseEvent) => void) => (
    <>
      {tab.kind === "term" && (
        <div onContextMenu={termCtx}>
          <DummyTerm
            title={tabLabel(tab)}
            termKey={tab.id}
            clearSignal={clears[tab.id] ?? 0}
            starter={["demo shell — type anything, nothing runs", "try: help"]}
          />
        </div>
      )}
      {tab.kind === "agents" && <div id="agents"><AgentsPane /></div>}
      {tab.kind === "doc" && tab.block && <DocView block={tab.block} />}
    </>
  );

  return (
    <div
      className="window"
      onContextMenu={(e) => {
        if ((e.target as HTMLElement).closest("input,textarea,[contenteditable]")) return;
        e.preventDefault();
      }}
    >
      <header className="top">
        <span className="proj" title="Selected project">terminator</span>
        <button className="ic" title="Player">player</button>
        <a className="ic bell" href="#agents" title="Agents inbox">bell<span className="badge">2</span></a>
        <button className="ic" title="Toggle left sidebar" onClick={() => setLeft((v) => !v)}>left</button>
        <button className="ic" title="Toggle right sidebar" onClick={() => setRight((v) => !v)}>right</button>
        <span className="drag" />
        <a className="gh" href={REPO}>GitHub</a>
        <a className="dl" href={DL}>Download</a>
      </header>

      <div className="tabrow">
        {left && (
          <span className="fbox">
            <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter projects" aria-label="Filter projects" />
          </span>
        )}
        <div className="tabs" role="tablist" aria-label="Workspace tabs">
          {tabs.map((t, index) => (
            <span
              key={t.id}
              role="tab"
              aria-selected={t.id === activeTabId}
              className={t.id === activeTabId ? "tab on" : "tab"}
              onContextMenu={(e) => tabMenu(e, t, index)}
            >
              {editingTab === t.id ? (
                <input
                  className="tedit"
                  autoFocus
                  defaultValue={tabLabel(t)}
                  onBlur={(e) => commitRename(t.id, e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") commitRename(t.id, (e.target as HTMLInputElement).value);
                    if (e.key === "Escape") setEditingTab(null);
                  }}
                  onClick={(e) => e.stopPropagation()}
                  aria-label="Rename tab"
                />
              ) : (
                <button
                  className="tlabel"
                  onClick={() => setPanes((ps) => ps.map((p) => (p.id === focus ? { ...p, tabId: t.id } : p)))}
                  title={t.kind === "doc" ? t.block?.title : "Demo terminal — commands do nothing"}
                >
                  {t.kind === "term" ? "term " : t.kind === "agents" ? "agents " : ""}
                  {tabLabel(t)}
                </button>
              )}
              <button className="tx" onClick={() => closeTab(t.id)} aria-label={`Close ${t.title}`}>×</button>
            </span>
          ))}
          <button className="tadd" onClick={() => newTab()} aria-label="New tab">+</button>
        </div>
        {right && (
          <span className="fbox r">
            <input value={find} onChange={(e) => setFind(e.target.value)} placeholder="Find in folder" aria-label="Find in folder" />
          </span>
        )}
      </div>

      <div className={`mid ${left ? "" : "noleft"} ${right ? "" : "noright"}`}>
        {left && (
          <aside className="side l" aria-label="Projects sidebar">
            {visibleChapters.map((c) => (
              <div key={c.id} className="proj-group">
                <button className="pgroup" onClick={() => toggleExpand(c.id)}>
                  <span className="car">{expanded.includes(c.id) ? "▾" : "▸"}</span>
                  {c.name}
                  <span className="cnt">{c.blocks.length}</span>
                </button>
                {expanded.includes(c.id) && (
                  <ul>
                    {c.blocks.map((b) => (
                      <li key={b.id}>
                        <button
                          className={activeTabId === `doc-${b.id}` ? "sess on" : "sess"}
                          onClick={() => openBlock(b)}
                          onContextMenu={(e) => sideMenu(e, b)}
                          title={b.title}
                        >
                          {b.id}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            ))}
            <button className="sess new" onClick={() => newTab()}>+ new terminal</button>
          </aside>
        )}

        <main className="dock" aria-label="Split panes" onContextMenu={dockMenu}>
          {panes.map((pane) => {
            const tab = tabById(tabs, pane.active);
            return (
              <section
                key={pane.id}
                data-pane={pane.id}
                className={pane.id === focus ? "pane foc" : "pane"}
                onClick={() => setFocus(pane.id)}
              >
                <div className="pane-h" onContextMenu={(e) => paneMenu(e, pane.id, tab)}>
                  <span className="ptitle">{tabLabel(tab)}</span>
                  <span className="pbr">master</span>
                  <span className="pbtns">
                    <button title="Split right" onClick={(e) => { e.stopPropagation(); setFocus(pane.id); splitPane(pane.id); }}>❚❚</button>
                    <button title="Split down (stacks below)" onClick={(e) => { e.stopPropagation(); setFocus(pane.id); splitPane(pane.id); }}>═</button>
                    <button title="Close pane" onClick={(e) => { e.stopPropagation(); closePane(pane.id); }}>×</button>
                  </span>
                </div>
                {pane.tabIds.length > 1 && (
                  <div className="leaf-tabs">
                    {pane.tabIds.map((tid) => {
                      const st = tabs.find((t) => t.id === tid);
                      if (!st) return null;
                      return (
                        <span key={tid} className={tid === pane.active ? "leaf on" : "leaf"}>
                          <button onClick={() => focusTabInPane(pane.id, tid)}>{tabLabel(st)}</button>
                          <button aria-label={`Close ${tabLabel(st)}`} onClick={() => closeTab(tid)}>×</button>
                        </span>
                      );
                    })}
                    <button className="leaf-add" onClick={() => newTab(pane.id)} aria-label="New tab in pane">+</button>
                  </div>
                )}
                <div className="pane-b">{tabBody(tab, (e) => termMenu(e, pane.id, tab))}</div>
              </section>
            );
          })}
        </main>

        {right && (
          <aside className="side r" aria-label="Context sidebar">
            <p className="sh">In this folder</p>
            <ul className="files">
              {rightFiles.map((b) => (
                <li key={b.id}>
                  <button
                    className={activeTabId === `doc-${b.id}` ? "sess on" : "sess"}
                    onClick={() => openBlock(b)}
                    onContextMenu={(e) => fileMenu(e, b)}
                  >
                    {b.id}
                  </button>
                </li>
              ))}
            </ul>
            <p className="sh">Install</p>
            <pre className="mini"><code>{`git clone ${REPO}\ncd terminator\ncargo run --bin terminator`}</code></pre>
            <p className="sh">Docs</p>
            <p className="rlinks">
              <a href={`${REPO}/blob/master/docs/REFERENCE.md`}>Reference</a>
              {" · "}
              <a href={`${REPO}/blob/master/docs/INTEGRATIONS.md`}>Agent setup</a>
            </p>
          </aside>
        )}
      </div>

      <footer className="status">
        <span><i className="live" />Connected</span>
        <span>{tabs.length} tabs · {panes.length} panes · master</span>
        <span>MIT · Rust · No Electron</span>
      </footer>
      {floats.map((f) => {
        const tab = tabById(tabs, f.tabId);
        return (
          <FloatWinView
            key={f.id}
            x={f.x}
            y={f.y}
            title={tabLabel(tab)}
            onMove={(x, y) => setFloats((fs) => fs.map((w) => (w.id === f.id ? { ...w, x, y } : w)))}
            onDock={() => dockFloat(f.id)}
            onClose={() => closeFloat(f.id)}
            onHeaderMenu={(e) => floatMenu(e, f.id)}
          >
            {tabBody(tab, (e) =>
              termMenu(
                e,
                focus,
                tab,
                (e.currentTarget as HTMLElement).closest(".float") as HTMLElement | null,
              ),
            )}
          </FloatWinView>
        );
      })}
      {menu && <CtxMenu menu={menu} close={() => setMenu(null)} />}
    </div>
  );
}
