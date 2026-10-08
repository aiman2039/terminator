import type * as React from "react";
import { useEffect, useRef, useState } from "react";

type Line = { t: string; cls?: "cmd" | "dim" | "link" | "ok" | "warn" };
type TermState = { lines: Line[]; history: string[] };

const starters: Record<string, Line[]> = {
  "s-dev": [
    { t: "npm run dev", cls: "cmd" },
    { t: "" },
    { t: "  VITE v5.4.2  ready in 312 ms" },
    { t: "" },
    { t: "  ➜  Local:   http://localhost:5173/", cls: "link" },
    { t: "  ➜  press h + enter to show help", cls: "dim" },
  ],
  "s-tests": [
    { t: "cargo test", cls: "cmd" },
    { t: "" },
    { t: "running 42 tests" },
    { t: "test result: ok. 42 passed; 0 failed; 0 ignored", cls: "ok" },
    { t: "" },
  ],
  "s-term": [
    { t: "claude", cls: "cmd" },
    { t: "" },
    { t: "  Claude Code v2.1 — hooks report to Terminator", cls: "dim" },
    { t: "  waiting for you", cls: "warn" },
  ],
};

const cache = new Map<string, TermState>();

const FILES: Record<string, string> = {
  "README.md": "# atlas-web\n\nA small sample project.",
  "src/app.ts": 'export const APP = () => "hello atlas";\nexport const version = 2;',
  "src/billing.ts": "export const price = (cents: number) => cents / 100;",
  "src/styles.css": ":root {\n  --accent: #3871e1;\n}",
};

const BANNER: Line[] = [
  { t: "Terminator demo shell — commands are simulated locally.", cls: "dim" },
  { t: "Type `help` for the list. Real sessions run in the daemon.", cls: "dim" },
  { t: "" },
];

function run(cmd: string): Line[] {
  const parts = cmd.trim().split(/\s+/);
  const name = parts[0];
  const args = parts.slice(1);
  const arg = args.join(" ");
  switch (name) {
    case "":
      return [];
    case "help":
      return [{ t: "help  ls  pwd  cd  cat  echo  clear  whoami  date  env  git  npm  cargo  open  about" }];
    case "ls":
      return [{ t: "README.md   docs.html   public/   src/   tone.wav" }];
    case "pwd":
      return [{ t: "~/code/atlas-web" }];
    case "cd":
    case "export":
      return [];
    case "whoami":
      return [{ t: "you" }];
    case "date":
      return [{ t: new Date().toString() }];
    case "env":
      return [
        {
          t: "SHELL=/bin/zsh\nTERM=xterm-256color\nTERMINATOR_DATA_DIR=~/Library/Application Support/terminator",
        },
      ];
    case "cat": {
      const key = Object.keys(FILES).find((k) => k === arg || k.endsWith("/" + arg) || k.endsWith(arg));
      return key
        ? FILES[key].split("\n").map((t) => ({ t }))
        : [{ t: `cat: ${arg}: No such file`, cls: "dim" }];
    }
    case "echo":
      return [{ t: arg }];
    case "clear":
      return [{ t: "__CLEAR__" }];
    case "git":
      if (args[0] === "status")
        return [
          { t: "On branch master" },
          { t: "Changes not staged for commit:", cls: "dim" },
          { t: "  modified:   src/app.ts" },
          { t: "  modified:   README.md" },
          { t: "Untracked files:", cls: "dim" },
          { t: "  src/billing.ts" },
        ];
      if (args[0] === "log")
        return [
          { t: "8f21c04  attach running sessions, never restart", cls: "dim" },
          { t: "a93b7d1  native diff viewer" },
        ];
      if (args[0] === "diff")
        return [
          { t: "diff --git a/src/app.ts b/src/app.ts", cls: "dim" },
          { t: '-const APP = () => "hello";', cls: "warn" },
          { t: '+const APP = () => "hello atlas";', cls: "ok" },
        ];
      return [{ t: `git: '${arg || "?"}' is not a demo command`, cls: "dim" }];
    case "npm":
      if (arg.includes("dev"))
        return [{ t: "  VITE ready in 312 ms\n  ➜  Local:   http://localhost:5173/", cls: "link" }];
      if (arg.includes("test"))
        return [{ t: "Test Files  3 passed (3)\n     Tests  12 passed (12)", cls: "ok" }];
      return [{ t: "npm: demo — try `npm run dev`", cls: "dim" }];
    case "cargo":
      if (arg.includes("test")) return [{ t: "test result: ok. 42 passed; 0 failed", cls: "ok" }];
      return [{ t: "cargo: demo — try `cargo test`", cls: "dim" }];
    case "open":
      return [{ t: `Opening ${arg || "file"} in the default viewer…`, cls: "dim" }];
    case "claude":
    case "codex":
    case "opencode":
    case "muse":
    case "grok":
      return [
        { t: `  ${name} is not launched from the demo.`, cls: "dim" },
        { t: "  Start it in a real Terminator terminal; hooks report status.", cls: "dim" },
      ];
    case "about":
      return [
        { t: "Terminator — terminals, AI agents, and project files in one place." },
        { t: "Native Rust + egui. The daemon owns every PTY.", cls: "dim" },
      ];
    case "exit":
      return [
        { t: "exit", cls: "dim" },
        { t: "[process exited — the session stays in History]", cls: "dim" },
      ];
    default:
      return [{ t: `zsh: command not found: ${name}`, cls: "dim" }];
  }
}

export function Terminal({ sid }: { sid: string }) {
  const [state, setState] = useState<TermState>(
    () => cache.get(sid) ?? { lines: starters[sid] ?? BANNER, history: [] },
  );
  const [val, setVal] = useState("");
  const [hIdx, setHIdx] = useState(-1);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    cache.set(sid, state);
  }, [sid, state]);

  useEffect(() => {
    const el = rootRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [state]);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const out = run(val);
    const clear = out.some((l) => l.t === "__CLEAR__");
    setState((s) => ({
      lines: clear
        ? []
        : [...s.lines, { t: val, cls: "cmd" as const }, ...out.filter((l) => l.t !== "__CLEAR__")],
      history: val.trim() ? [...s.history, val.trim()] : s.history,
    }));
    setVal("");
    setHIdx(-1);
  };

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (!state.history.length) return;
      const i = hIdx < 0 ? state.history.length - 1 : Math.max(0, hIdx - 1);
      setHIdx(i);
      setVal(state.history[i]);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      if (hIdx < 0) return;
      const i = hIdx + 1;
      if (i >= state.history.length) {
        setHIdx(-1);
        setVal("");
      } else {
        setHIdx(i);
        setVal(state.history[i]);
      }
    } else if (e.key === "l" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      setState((s) => ({ ...s, lines: [] }));
    }
  };

  return (
    <div
      ref={rootRef}
      className="tty"
      onClick={() => {
        if (!window.getSelection()?.toString()) inputRef.current?.focus();
      }}
    >
      {state.lines.map((l, i) => (
        <p key={i} className={l.cls === "cmd" ? undefined : l.cls}>
          {l.cls === "cmd" ? (
            <>
              <span className="ps">sh-3.2$ </span>
              {l.t}
            </>
          ) : (
            l.t || "\u00a0"
          )}
        </p>
      ))}
      <form className="tty-form" onSubmit={submit}>
        <span className="ps">sh-3.2$</span>
        <input
          ref={inputRef}
          value={val}
          onChange={(e) => setVal(e.target.value)}
          onKeyDown={onKey}
          spellCheck={false}
          autoComplete="off"
          aria-label="Demo terminal (commands are simulated)"
        />
      </form>
    </div>
  );
}
