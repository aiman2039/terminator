import type * as React from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "./icons";

export type Command = {
  id: string;
  label: string;
  kind: string;
  icon: string;
  sub?: string;
  run: () => void;
};

export function Palette({ commands, close }: { commands: Command[]; close: () => void }) {
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => inputRef.current?.focus(), []);

  const hits = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return commands;
    return commands.filter((c) => `${c.label} ${c.kind} ${c.sub ?? ""}`.toLowerCase().includes(needle));
  }, [q, commands]);

  useEffect(() => setSel(0), [q]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSel((s) => Math.min(s + 1, hits.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSel((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      hits[sel]?.run();
      close();
    }
  };

  return (
    <div className="scrim" onMouseDown={close}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Command palette">
        <input
          ref={inputRef}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={onKey}
          placeholder="Run a command, go to a session or file…"
          aria-label="Command palette query"
        />
        <div className="pl-sep" />
        <div className="pl-list">
          {hits.map((c, i) => (
            <button
              key={c.id}
              className={`pl-item${i === sel ? " sel" : ""}`}
              onMouseEnter={() => setSel(i)}
              onClick={() => {
                c.run();
                close();
              }}
            >
              <span className="ic">
                <Icon name={c.icon} size={15} />
              </span>
              <span>
                {c.label}
                {c.sub && <span className="sub"> · {c.sub}</span>}
              </span>
              <span className="kind">{c.kind}</span>
            </button>
          ))}
          {hits.length === 0 && <div className="pl-item" style={{ color: "#a1a1a1" }}>No matching commands.</div>}
        </div>
      </div>
    </div>
  );
}
