const REPO = "https://github.com/aiman2039/terminator";
const DL = `${REPO}/releases/latest`;
const img = (n: string) => `/assets/${n}.png`;

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

const extras = [
  ["Browser tabs", "Open HTML files and http(s) URLs in a GUI-only tab."],
  ["Image previews", "PNG, JPEG, SVG with fit and actual size."],
  ["Terminal search", "Find text in output. Left-drag always selects."],
  ["Command palette", "Run any action from the keyboard."],
  ["Auto updates", "Checks every minute. Manual check in the menu."],
  ["Crash logs", "Panics write a log. Sessions keep running."],
];

export default function App() {
  return (
    <>
      <header className="nav">
        <a className="brand" href="#top">
          <img src="/assets/icon.png" alt="" width="28" height="28" />
          TERMINATOR
        </a>
        <nav>
          {chapters.map((c) => (
            <a key={c.id} href={`#${c.id}`}>{c.name}</a>
          ))}
          <a href={REPO}>GitHub</a>
        </nav>
        <a className="btn small" href={DL}>Download</a>
      </header>

      <main id="top">
        <section className="hero">
          <p className="eyebrow">terminals · agents · files</p>
          <h1>
            Not just a terminal. <em>Your whole workspace.</em>
          </h1>
          <p className="lead">
            Projects, splits, AI agent alerts, Neovim, Git reviews, previews, and a music player.
            One native app for macOS and Linux. Sessions survive closing the window.
          </p>
          <div className="cta">
            <a className="btn" href={DL}>Download</a>
            <a className="btn ghost" href={REPO}>Star on GitHub</a>
          </div>
          <p className="muted">Open source, MIT license · Rust · No Electron</p>
          <img className="shot hero-shot" src={img("ide")} alt="Terminator in IDE mode" />
        </section>

        <section className="toc">
          {chapters.map((c) => (
            <div key={c.id}>
              <a href={`#${c.id}`}><b>{c.name}</b></a>
              <ul>
                {c.blocks.map((b) => (
                  <li key={b.id}><a href={`#${b.id}`}>{b.title.replace(/\.$/, "")}</a></li>
                ))}
              </ul>
            </div>
          ))}
        </section>

        {chapters.map((c) => (
          <div key={c.id} id={c.id} className="chapter">
            <h2 className="chapter-name">{c.name}</h2>
            {c.blocks.map((b, i) => (
              <section key={b.id} id={b.id} className={`section show ${i % 2 ? "rev" : ""}`}>
                <div>
                  <p className="eyebrow">{b.tag}</p>
                  <h3>{b.title}</h3>
                  <p className="lead">{b.text}</p>
                  {b.points && (
                    <ul className="points">
                      {b.points.map((p) => (<li key={p}>{p}</li>))}
                    </ul>
                  )}
                </div>
                <img className="shot" src={img(b.shot)} alt={b.alt} loading="lazy" />
              </section>
            ))}
          </div>
        ))}

        <section className="section">
          <h2 className="chapter-name">And more</h2>
          <div className="grid">
            {extras.map(([t, d]) => (
              <article key={t} className="card"><h3>{t}</h3><p>{d}</p></article>
            ))}
          </div>
        </section>

        <section id="install" className="section center">
          <p className="eyebrow">// get started</p>
          <h2>From the first command to the final review.</h2>
          <pre className="code">
            <code>{`git clone ${REPO}\ncd terminator\ncargo build --workspace --locked\ncargo run --bin terminator --locked`}</code>
          </pre>
          <p className="muted">Install Neovim for editing. Git reviews with CodeDiff need Neovim 0.10+.</p>
          <div className="cta">
            <a className="btn" href={DL}>Download</a>
            <a className="btn ghost" href={`${REPO}/blob/master/docs/REFERENCE.md`}>Read the docs</a>
          </div>
        </section>
      </main>

      <footer className="footer">
        <span>© 2026 Terminator · MIT</span>
        <span>
          <a href={REPO}>GitHub</a> · <a href={`${REPO}/blob/master/CHANGELOG.md`}>Changelog</a> ·{" "}
          <a href={`${REPO}/blob/master/docs/INTEGRATIONS.md`}>Agent setup</a>
        </span>
      </footer>
    </>
  );
}
