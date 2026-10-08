export const REPO = "https://github.com/aiman2039/terminator";
export const DOWNLOAD = `${REPO}/releases/latest`;

/* --------------------------- sample file text --------------------------- */
export const README_MD = `# atlas-web
A small sample project.

- [x] Auth
- [ ] Billing

**Ships** with \`npm run dev\`.

| Route | Status |
| --- | --- |
| /home | ok |
| /pay | ok |
`;

export const APP_TS = `export const APP = () => "hello atlas";
export const version = 2;
`;

export const BILLING_TS = `export const price = (cents: number) => cents / 100;
`;

export const CSS = `:root {
  --accent: #3871e1;
}
`;

export type PaneContent =
  | { kind: "term"; sid: string }
  | { kind: "markdown"; path: string }
  | { kind: "editor"; path: string }
  | { kind: "diff"; path: string; staged: boolean }
  | { kind: "image"; path: string }
  | { kind: "tour" };

export type Layout =
  | { type: "leaf"; id: string; content: PaneContent }
  | { type: "split"; id: string; dir: "row" | "col"; children: Layout[] };

export type Group = { id: string; layout: Layout; activeLeaf: string };

export type Session = {
  id: string;
  label: string;
  projectId: string;
  branch: string;
  cwd: string;
  kind: "shell" | "editor";
  status: "running" | "waiting" | "failed";
  agent?: string;
  visible: boolean;
  started: string;
  cpu: number;
  mem: number;
};

export type Project = {
  id: string;
  name: string;
  path: string;
  branch: string;
};

let n = 100;
export const nid = (p: string) => `${p}-${++n}`;

export const leaf = (content: PaneContent, id = nid("leaf")): Layout => ({
  type: "leaf",
  id,
  content,
});

export const firstLeaf = (l: Layout): Extract<Layout, { type: "leaf" }> =>
  l.type === "leaf" ? l : firstLeaf(l.children[0]);

export const leaves = (l: Layout): Extract<Layout, { type: "leaf" }>[] =>
  l.type === "leaf" ? [l] : l.children.flatMap(leaves);

export const findPane = (l: Layout, id: string): Layout | undefined => {
  if (l.type === "leaf") return l.id === id ? l : undefined;
  for (const c of l.children) {
    const hit = findPane(c, id);
    if (hit) return hit;
  }
  return undefined;
};

export const removePane = (l: Layout, id: string): Layout | undefined => {
  if (l.type === "leaf") return l.id === id ? undefined : l;
  const kids = l.children.map((c) => removePane(c, id)).filter(Boolean) as Layout[];
  if (kids.length === 0) return undefined;
  if (kids.length === 1) return kids[0];
  return { ...l, children: kids };
};

export const updatePane = (l: Layout, id: string, content: PaneContent): Layout =>
  l.type === "leaf"
    ? l.id === id
      ? { ...l, content }
      : l
    : { ...l, children: l.children.map((c) => updatePane(c, id, content)) };

export const projects: Project[] = [
  { id: "atlas-api", name: "atlas-api", path: "~/code/atlas-api", branch: "main" },
  { id: "atlas-web", name: "atlas-web", path: "~/code/atlas-web", branch: "master" },
  { id: "dotfiles", name: "dotfiles", path: "~/dotfiles", branch: "master" },
];

export const sessions: Session[] = [
  {
    id: "s-api",
    label: "Terminal 1",
    projectId: "atlas-api",
    branch: "main",
    cwd: "~/code/atlas-api",
    kind: "shell",
    status: "running",
    visible: true,
    started: "22s ago",
    cpu: 0,
    mem: 4,
  },
  {
    id: "s-dev",
    label: "Dev server",
    projectId: "atlas-web",
    branch: "master",
    cwd: "~/code/atlas-web",
    kind: "shell",
    status: "running",
    visible: true,
    started: "22s ago",
    cpu: 0,
    mem: 3,
  },
  {
    id: "s-tests",
    label: "Tests",
    projectId: "atlas-web",
    branch: "master",
    cwd: "~/code/atlas-web",
    kind: "shell",
    status: "running",
    visible: true,
    started: "1m ago",
    cpu: 0,
    mem: 2,
  },
];

/* --------------------------- files --------------------------- */
export type FileNode = {
  name: string;
  path: string;
  dir?: boolean;
  status?: "M" | "A" | "U" | "?" | "D";
  icon?: string;
  children?: FileNode[];
  contents?: string;
};

export const fileTree: FileNode[] = [
  {
    name: "src",
    path: "src",
    dir: true,
    status: "M",
    children: [
      { name: "app.ts", path: "src/app.ts", status: "M", contents: APP_TS },
      { name: "styles.css", path: "src/styles.css", status: "M", contents: CSS },
      { name: "billing.ts", path: "src/billing.ts", status: "U", contents: BILLING_TS },
    ],
  },
  { name: "public", path: "public", dir: true, children: [{ name: "hero.png", path: "public/hero.png" }] },
  { name: "docs.html", path: "docs.html" },
  { name: "README.md", path: "README.md", status: "M", contents: README_MD },
  { name: "tone.wav", path: "tone.wav" },
];

export function flattenFiles(nodes: FileNode[], out: FileNode[] = []): FileNode[] {
  for (const n of nodes) {
    if (!n.dir) out.push(n);
    if (n.children) flattenFiles(n.children, out);
  }
  return out;
}

export const allFiles = flattenFiles(fileTree);

export function fileIcon(name: string): string {
  if (name.endsWith(".ts") || name.endsWith(".tsx") || name.endsWith(".js")) return "FileCode";
  if (name.endsWith(".rs")) return "FileCode";
  if (name.endsWith(".css")) return "FileType";
  if (name.endsWith(".html")) return "FileCode";
  if (name.endsWith(".json")) return "FileJson";
  if (name.endsWith(".md")) return "FileText";
  if (name.endsWith(".png") || name.endsWith(".jpg")) return "FileImage";
  if (name.endsWith(".wav") || name.endsWith(".mp3")) return "FileMusic";
  if (name.endsWith(".toml")) return "FileSliders";
  if (name.includes(".")) return "File";
  return "File";
}

/* --------------------------- git --------------------------- */
export const gitGroups: { title: string; count: number; files: FileNode[] }[] = [
  {
    title: "CHANGES",
    count: 2,
    files: [
      { name: "app.ts", path: "src/app.ts", status: "M", contents: APP_TS },
      { name: "README.md", path: "README.md", status: "M", contents: README_MD },
    ],
  },
  {
    title: "UNTRACKED FILES",
    count: 1,
    files: [{ name: "billing.ts", path: "src/billing.ts", status: "U", contents: BILLING_TS }],
  },
];

export const diffFor = (path: string, staged: boolean) =>
  staged
    ? [
        ["hunk", "@@ -1,1 +1,2 @@"],
        ["del", 'const APP = () => "hello";'],
        ["add", 'const APP = () => "hello atlas";'],
        ["add", "const version = 2;"],
      ]
    : [
        ["hunk", "@@ -1,1 +1,2 @@"],
        ["del", "restart_all(sessions);"],
        ["add", "attach_running(sessions); // never restart"],
        ["add", "focus_existing_tab();"],
      ];

/* --------------------------- agents --------------------------- */
export type AgentCard = {
  id: string;
  brand: string;
  title: string;
  body: string;
  state: "needs permission" | "needs input" | "completed" | "failed";
};

export const agentCards: AgentCard[] = [
  {
    id: "a1",
    brand: "AgentClaude",
    title: "Dev server",
    body: "Wants to edit crates/app/src/workspace.rs",
    state: "needs permission",
  },
  {
    id: "a2",
    brand: "AgentCodex",
    title: "Tests",
    body: "Finished the review pass; scrollback saved.",
    state: "completed",
  },
  {
    id: "a3",
    brand: "AgentOpencode",
    title: "Terminal 1",
    body: "Waiting for your next instruction.",
    state: "needs input",
  },
];

/* --------------------------- history --------------------------- */
export const historyGroups: { project: string; sessions: { id: string; label: string; when: string }[] }[] = [
  {
    project: "atlas-web",
    sessions: [
      { id: "h1", label: "Dev server", when: "2 min ago" },
      { id: "h2", label: "Tests", when: "18 min ago" },
      { id: "h3", label: "npm run build", when: "yesterday" },
    ],
  },
  {
    project: "atlas-api",
    sessions: [{ id: "h4", label: "cargo run", when: "yesterday" }],
  },
  { project: "dotfiles", sessions: [{ id: "h5", label: "stow -R zsh", when: "3 days ago" }] },
];

/* --------------------------- resources --------------------------- */
export const systemInfo = {
  cpu: 13,
  memUsed: 51.7,
  memTotal: 128,
  pressure: "0.4% normal",
  load: "5.32 4.28 3.98",
};
