import { useLayoutEffect, useRef, useState } from "react";

export type MenuItem =
  | { kind: "header"; label: string }
  | { kind: "sep" }
  | { kind: "item"; label: string; shortcut?: string; destructive?: boolean; disabled?: boolean; run: () => void }
  | { kind: "sub"; label: string; disabled?: boolean; items: MenuItem[] };

export type MenuState = { x: number; y: number; items: MenuItem[] };

const GLYPHS: Record<string, string> = {
  Copy: "⧉",
  "Select all": "▦",
  Paste: "⎘",
  "New tab": "+",
  "Split up": "▲",
  "Split down": "▼",
  "Split left": "◀",
  "Split right": "▶",
  "Add tab to the left": "+",
  "Add tab to the right": "+",
  "Open file path…": "❏",
  "Search scrollback": "⌕",
  "Copy working directory": "❏",
  "Copy path": "⧉",
  "Rename…": "✎",
  "Close session…": "×",
  "Close tab…": "×",
  "Close all tabs…": "×",
  "Close all tabs to the left…": "×",
  "Close all tabs to the right…": "×",
  "Clear saved scrollback": "⌫",
  "Detach to new tab": "❐",
  "Float window": "❖",
  "Dock back": "⇲",
  "Close float": "×",
  "Move to pane": "⇄",
  "Tabs in this pane": "▤",
  Open: "↗",
  "Open in focused pane": "↗",
  "Open in new split": "◫",
};

function Row({ it, close, flip }: { it: MenuItem; close: () => void; flip: boolean }) {
  const [open, setOpen] = useState(false);
  if (it.kind === "sep") return <div className="ctx-sep" />;
  if (it.kind === "header")
    return (
      <div className="ctx-head" title={it.label}>
        {it.label}
      </div>
    );
  if (it.kind === "sub")
    return (
      <div
        className="ctx-subwrap"
        onMouseEnter={() => {
          if (!it.disabled) setOpen(true);
        }}
        onMouseLeave={() => setOpen(false)}
      >
        <button
          role="menuitem"
          aria-haspopup="menu"
          disabled={it.disabled}
          className="ctx-item"
          onClick={() => {
            if (!it.disabled) setOpen((o) => !o);
          }}
        >
          <span className="ctx-ic">{GLYPHS[it.label] ?? "▸"}</span>
          <span className="ctx-label">{it.label}</span>
          <span className="ctx-sc">▸</span>
        </button>
        {open && (
          <div className={flip ? "ctx ctx-sub flip" : "ctx ctx-sub"} role="menu">
            {it.items.map((s, j) => (
              <Row key={j} it={s} close={close} flip={flip} />
            ))}
          </div>
        )}
      </div>
    );
  return (
    <button
      role="menuitem"
      disabled={it.disabled}
      className={`ctx-item${it.destructive ? " danger" : ""}`}
      onClick={() => {
        close();
        it.run();
      }}
    >
      <span className="ctx-ic">{GLYPHS[it.label] ?? "·"}</span>
      <span className="ctx-label">{it.label}</span>
      {it.shortcut && <span className="ctx-sc">{it.shortcut}</span>}
    </button>
  );
}

export function CtxMenu({ menu, close }: { menu: MenuState; close: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: menu.x, y: menu.y });
  const flip = menu.x > window.innerWidth - 480;
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(menu.x, window.innerWidth - r.width - 4)),
      y: Math.max(4, Math.min(menu.y, window.innerHeight - r.height - 4)),
    });
  }, [menu]);
  return (
    <div ref={ref} className="ctx" style={{ left: pos.x, top: pos.y }} role="menu" onContextMenu={(e) => e.preventDefault()}>
      {menu.items.map((it, i) => (
        <Row key={i} it={it} close={close} flip={flip} />
      ))}
    </div>
  );
}
