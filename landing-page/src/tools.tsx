import { useState } from "react";
import { Icon } from "./icons";
import { basename, fileIcon } from "./data";

/* ============================ Tool stripe ============================ */
export function ToolStripe<Id extends string>({
  items,
  active,
  onPick,
  onSearch,
}: {
  items: readonly { id: Id; label: string; icon: string }[];
  active: Id | null;
  onPick: (id: Id) => void;
  onSearch: () => void;
}) {
  return (
    <nav className="stripe" aria-label="Tool windows">
      {items.map((t) => (
        <button key={t.id} className={`stripe-btn${active === t.id ? " on" : ""}`} title={t.label} onClick={() => onPick(t.id)}>
          <Icon name={t.icon} size={16} />
          <span>{t.label}</span>
        </button>
      ))}
      <span className="grow" />
      <button className="stripe-btn" title="Search everywhere (double Shift)" onClick={onSearch}>
        <Icon name="Search" size={16} />
        <span>Search</span>
      </button>
    </nav>
  );
}

function PanelHead({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className="panel-head">
      <span>{title}</span>
      <span className="grow" />
      {children}
    </div>
  );
}

/* ============================ Problems ============================ */
const PROBLEMS = [
  { file: "src/billing.ts", line: 1, col: 14, sev: "error", msg: "Type 'string' is not assignable to type 'number'.", src: "ts" },
  { file: "src/app.ts", line: 5, col: 10, sev: "warning", msg: "'route' is declared but never used.", src: "ts" },
  { file: "src/app.ts", line: 2, col: 14, sev: "info", msg: "Prefer 'as const' for literal exports.", src: "eslint" },
];

export function ProblemsPanel({ onOpen }: { onOpen: (path: string) => void }) {
  const [scope, setScope] = useState<"file" | "project">("project");
  const shown = scope === "file" ? PROBLEMS.filter((p) => p.file === "src/app.ts") : PROBLEMS;
  const byFile = new Map<string, typeof shown>();
  for (const p of shown) byFile.set(p.file, [...(byFile.get(p.file) ?? []), p]);
  const files = [...byFile];
  return (
    <>
      <PanelHead title="Problems">
        <button className="ic-btn" title="Refresh"><Icon name="RefreshCw" size={14} /></button>
      </PanelHead>
      <div className="agent-tabs">
        <button className={scope === "file" ? "on" : ""} onClick={() => setScope("file")}>Current file</button>
        <button className={scope === "project" ? "on" : ""} onClick={() => setScope("project")}>Project</button>
      </div>
      <div className="side-scroll">
        {files.map(([f, ps]) => (
          <div key={f}>
            <div className="tree-row">
              <Icon name="ChevronDown" size={12} />
              <span className="row-ic"><Icon name={fileIcon(f)} size={15} /></span>
              <span className="row-label">{basename(f)}</span>
              <span className="row-trail">{ps.length}</span>
            </div>
            {ps.map((p) => (
              <div key={`${p.line}:${p.col}`} className="tree-row indent" title={`${p.src}: ${p.msg}`} onClick={() => onOpen(p.file)}>
                <span className="row-ic" style={{ color: `var(--${p.sev === "error" ? "failed" : p.sev === "warning" ? "waiting" : "secondary"})` }}>
                  <Icon name="CircleAlert" size={14} />
                </span>
                <span className="row-label">{p.msg}</span>
                <span className="row-trail mono">:{p.line}</span>
              </div>
            ))}
          </div>
        ))}
      </div>
    </>
  );
}

/* ============================ Structure ============================ */
const OUTLINE: { name: string; kind: string; depth: number }[] = [
  { name: "App", kind: "const", depth: 0 },
  { name: "version", kind: "const", depth: 0 },
  { name: "route(path)", kind: "function", depth: 0 },
  { name: "Billing", kind: "class", depth: 0 },
  { name: "total()", kind: "method", depth: 1 },
  { name: "refund(id)", kind: "method", depth: 1 },
];
const KIND_COLOR: Record<string, string> = { const: "var(--git-ignored)", function: "var(--git-modified)", class: "var(--git-added)", method: "var(--git-untracked)" };

export function StructurePanel() {
  const [sel, setSel] = useState("route(path)");
  return (
    <>
      <PanelHead title="Structure">
        <button className="ic-btn" title="Refresh"><Icon name="RefreshCw" size={14} /></button>
      </PanelHead>
      <div className="side-scroll">
        {OUTLINE.map((o) => (
          <div key={o.name} className={`tree-row${sel === o.name ? " on" : ""}${o.depth ? " indent" : ""}`} onClick={() => setSel(o.name)}>
            <span className="kind-chip" style={{ color: KIND_COLOR[o.kind] }}>{o.kind[0].toUpperCase()}</span>
            <span className="row-label mono">{o.name}</span>
            <span className="row-trail">{o.kind}</span>
          </div>
        ))}
      </div>
    </>
  );
}

/* ============================ VNC ============================ */
const VNC_HOSTS = [
  { name: "studio-mac", addr: "studio.local:5900", live: true, last: "connected" },
  { name: "lab-linux", addr: "10.0.4.12:5901", live: false, last: "2 h ago" },
  { name: "render-01", addr: "render-01 via SSH", live: false, last: "yesterday" },
];

export function VncPanel() {
  return (
    <>
      <PanelHead title="VNC">
        <button className="ic-btn" title="Address book"><Icon name="LayoutDashboard" size={14} /></button>
        <button className="ic-btn" title="Connect…"><Icon name="Plus" size={14} /></button>
      </PanelHead>
      <div className="section-cap">Sessions</div>
      {VNC_HOSTS.filter((h) => h.live).map((h) => (
        <div key={h.name} className="tree-row">
          <span className="dot running" />
          <span className="row-ic"><Icon name="Monitor" size={15} /></span>
          <span className="row-label">{h.name}</span>
          <span className="row-trail">detached</span>
        </div>
      ))}
      <div className="section-cap">Hosts</div>
      <div className="side-scroll">
        {VNC_HOSTS.map((h) => (
          <div key={h.name} className="tree-row" title={h.addr}>
            <span className="row-ic"><Icon name="Monitor" size={15} /></span>
            <span className="row-label">{h.name}</span>
            <span className="row-trail">{h.last}</span>
          </div>
        ))}
        <div className="agent-empty">Closing a VNC tab detaches it. The daemon keeps the session, like a terminal.</div>
      </div>
    </>
  );
}

/* ============================ Hosts (SSH) ============================ */
const SSH_HOSTS = [
  { name: "build-box", addr: "dev@build-box", mosh: false, sessions: 2, fwd: "5173 → 5173" },
  { name: "gpu-01", addr: "ml@gpu-01.lab", mosh: true, sessions: 1, fwd: "" },
  { name: "staging", addr: "deploy@staging", mosh: false, sessions: 0, fwd: "" },
];

export function HostsPanel() {
  const [q, setQ] = useState("");
  const shown = SSH_HOSTS.filter((h) => !q || h.name.includes(q.toLowerCase()));
  return (
    <>
      <PanelHead title="Hosts">
        <button className="ic-btn" title="Import from ~/.ssh/config"><Icon name="History" size={14} /></button>
        <button className="ic-btn" title="Add host"><Icon name="Plus" size={14} /></button>
      </PanelHead>
      <div className="side-head">
        <div className="field">
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Filter hosts..." aria-label="Filter hosts" />
        </div>
      </div>
      <div className="side-scroll">
        {shown.map((h) => (
          <div key={h.name}>
            <div className="tree-row" title={h.addr}>
              <Icon name="ChevronDown" size={12} />
              <span className="row-ic"><Icon name="Globe" size={15} /></span>
              <span className="row-label">{h.name}</span>
              {h.mosh && <span className="tag-chip">mosh</span>}
              <span className="row-trail">{h.sessions || ""}</span>
            </div>
            {h.sessions > 0 && (
              <div className="tree-row indent">
                <span className="row-ic"><Icon name="Terminal" size={15} /></span>
                <span className="row-label">{h.mosh ? "tmux (mosh)" : "Terminal 1"}</span>
                <span className="dot running" />
              </div>
            )}
            {h.fwd && (
              <div className="tree-row indent">
                <span className="row-ic"><Icon name="ArrowRight" size={14} /></span>
                <span className="row-label mono">{h.fwd}</span>
              </div>
            )}
          </div>
        ))}
        <div className="agent-empty">Remote Explorer, Git, editors and transfers run over OpenSSH. Nothing is installed on the host.</div>
      </div>
    </>
  );
}
