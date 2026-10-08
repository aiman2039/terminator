import type * as React from "react";
import grokUrl from "./icons/agent-grok.png";

/**
 * Lucide-style icons mirrored from the native app (`crates/app/assets/icons`).
 * Each SVG is inlined with its root presentation attributes preserved, so
 * `stroke="currentColor"` tracks the app's exact icon tints.
 */
const raw = import.meta.glob("./icons/*.svg", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

type Def = { html: string; brand: boolean };

const parse = (svg: string, base: string): Def => {
  const uiIcon = svg.includes('stroke="#ffffff"');
  let html = svg
    .replace(/<\?xml[^>]*\?>/g, "")
    .replace(/\swidth="[^"]*"/, "")
    .replace(/\sheight="[^"]*"/, "");
  if (uiIcon) html = html.replaceAll('stroke="#ffffff"', 'stroke="currentColor"');
  // Fill the wrapper; sizing is handled by the parent element.
  html = html.replace(/<svg\b/, '<svg style="display:block;width:100%;height:100%"');
  return { html, brand: !uiIcon && !base.endsWith("file") };
};

const kebab = (name: string) =>
  name
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1-$2")
    .toLowerCase();

const OVERRIDES: Record<string, string> = {
  Terminal: "terminal-octicons",
  FileBraces: "file-json",
};

const DEFS: Record<string, Def> = {};
for (const [path, svg] of Object.entries(raw)) {
  const base = path.replace("./icons/", "").replace(/\.svg$/, "");
  DEFS[base] = parse(svg, base);
}

function lookup(name: string): Def {
  if (name === "AgentGrok") return DEFS.file;
  const base = OVERRIDES[name] ?? kebab(name);
  return DEFS[base] ?? DEFS.file;
}

export function Icon({
  name,
  size = 16,
  className,
  style,
  title,
}: {
  name: string;
  size?: number;
  className?: string;
  style?: React.CSSProperties;
  title?: string;
}) {
  const wrap: React.CSSProperties = {
    display: "inline-flex",
    flex: "0 0 auto",
    width: size,
    height: size,
    color: "inherit",
    ...style,
  };
  if (name === "AgentGrok") {
    return (
      <span className={className} style={wrap} title={title}>
        <img src={grokUrl} alt="" aria-hidden="true" style={{ width: "100%", height: "100%", objectFit: "contain" }} />
      </span>
    );
  }
  const def = lookup(name);
  return (
    <span
      className={className}
      style={wrap}
      title={title}
      aria-hidden="true"
      dangerouslySetInnerHTML={{ __html: def.html }}
    />
  );
}
