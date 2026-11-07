import { useState } from "react";
import { Icon } from "./icons";
import { DiffRow } from "./git";

/* ============================ Browser tab (native: WKWebView / WebKitGTK) ============================ */
export function BrowserView({ url, onClose }: { url: string; onClose: () => void }) {
  const [addr, setAddr] = useState(url.startsWith("http") ? url : `file:///~/code/atlas-web/${url}`);
  return (
    <>
      <div className="browser-bar">
        <button className="ic-btn sm" title="Back"><Icon name="ArrowLeft" size={14} /></button>
        <button className="ic-btn sm" title="Forward"><Icon name="ArrowRight" size={14} /></button>
        <button className="ic-btn sm" title="Reload"><Icon name="RefreshCw" size={14} /></button>
        <input className="browser-addr" value={addr} onChange={(e) => setAddr(e.target.value)} aria-label="Address" spellCheck={false} />
        <button className="ic-btn sm" title="Open in default browser"><Icon name="ExternalLink" size={14} /></button>
        <button className="ic-btn sm" title="Close" onClick={onClose}><Icon name="X" size={14} /></button>
      </div>
      <div className="browser-page">
        <h1>Atlas docs</h1>
        <p>Local HTML and http(s) URLs open in a native browser tab beside your terminals.</p>
        <ul>
          <li>Isolated profile: no cookies from your everyday browser.</li>
          <li>The tab closes with the GUI. Its URL is saved in the layout.</li>
          <li><strong>Open as text</strong> opens the HTML source in the editor.</li>
        </ul>
      </div>
    </>
  );
}

/* ============================ Local History ============================ */
type Snap = { id: string; when: string; label: string; lines: ["add" | "del" | "ctx", string][] };
const SNAPS: Snap[] = [
  {
    id: "s3",
    when: "2 min ago",
    label: "External change (agent)",
    lines: [["ctx", 'export const App = () => "hello atlas";'], ["del", "export const version = 2;"], ["add", "export const version = 3;"], ["add", "export const debug = true;"]],
  },
  {
    id: "s2",
    when: "14 min ago",
    label: "Saved in editor",
    lines: [["del", 'export const App = () => "hello";'], ["add", 'export const App = () => "hello atlas";'], ["add", "export const version = 2;"]],
  },
  { id: "s1", when: "1 h ago", label: "Opened", lines: [["ctx", 'export const App = () => "hello";']] },
];

export function LocalHistoryView({ path, onClose, onNote }: { path: string; onClose: () => void; onNote: (s: string) => void }) {
  const [sel, setSel] = useState(SNAPS[0].id);
  const snap = SNAPS.find((s) => s.id === sel) ?? SNAPS[0];
  return (
    <>
      <div className="diff-bar">
        <Icon name="History" size={14} />
        <span>Local History</span>
        <span className="weak mono">{path}</span>
        <span className="grow" />
        <button className="small-btn" onClick={() => onNote(`Reverted ${path} to the snapshot from ${snap.when}. Revert is also a snapshot.`)}>Revert</button>
        <button className="ic-btn sm" title="Close" onClick={onClose}><Icon name="X" size={14} /></button>
      </div>
      <div className="lh">
        <div className="lh-list">
          {SNAPS.map((s) => (
            <div key={s.id} className={`tree-row${s.id === sel ? " on" : ""}`} onClick={() => setSel(s.id)}>
              <span className="log-dot" />
              <span className="row-label">{s.label}</span>
              <span className="row-trail">{s.when}</span>
            </div>
          ))}
          <div className="agent-empty">A snapshot on every save and external change. A safety net while agents edit.</div>
        </div>
        <div className="diff-body">
          {snap.lines.map(([k, t], i) => (
            <DiffRow key={i} kind={k} newLn={i + 1} text={t} />
          ))}
        </div>
      </div>
    </>
  );
}
