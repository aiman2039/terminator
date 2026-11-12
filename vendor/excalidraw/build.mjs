// Builds the Excalidraw page that Terminator embeds into its binary.
// Output: dist/ (index.html, app.js, app.css, chunks, fonts).
import { build } from "esbuild";
import { createHash } from "node:crypto";
import { cp, readFile, readdir, rm } from "node:fs/promises";

const out = new URL("./dist/", import.meta.url).pathname;
const fonts = new URL(
  "./node_modules/@excalidraw/excalidraw/dist/prod/fonts/",
  import.meta.url,
).pathname;

// All files of libraries/ as one module, `bundled-libraries`. The page loads
// it only when the bundled set is new to the user's library.
const libraryDir = new URL("./libraries/", import.meta.url).pathname;
const libraryFiles = (await readdir(libraryDir))
  .filter((file) => file.endsWith(".excalidrawlib"))
  .sort();
const libraries = JSON.stringify(
  await Promise.all(
    libraryFiles.map(async (file) => JSON.parse(await readFile(libraryDir + file, "utf8"))),
  ),
);
const bundledLibraries = {
  name: "bundled-libraries",
  setup(build) {
    build.onResolve({ filter: /^bundled-libraries$/ }, (args) => ({
      path: args.path,
      namespace: "bundled-libraries",
    }));
    build.onLoad({ filter: /.*/, namespace: "bundled-libraries" }, () => ({
      contents: libraries,
      loader: "json",
    }));
  },
};

await rm(out, { recursive: true, force: true });
await build({
  entryPoints: { app: "src/main.jsx" },
  outdir: out,
  bundle: true,
  format: "esm",
  splitting: true,
  minify: true,
  legalComments: "none",
  target: "safari15",
  conditions: ["production"],
  chunkNames: "chunks/[hash]",
  assetNames: "assets/[hash]",
  plugins: [bundledLibraries],
  define: {
    "process.env.NODE_ENV": '"production"',
    // Changes when the bundled set changes.
    BUNDLED_LIBRARIES: JSON.stringify(
      createHash("sha256").update(libraries).digest("hex").slice(0, 12),
    ),
  },
  loader: { ".woff2": "file", ".ttf": "file", ".png": "file", ".svg": "file" },
});
await cp("src/index.html", `${out}index.html`);
// Hand-drawn Latin, code and UI fonts. The CJK font (Xiaolai, 13 MB) stays
// out; CJK text uses a system font.
for (const family of [
  "Assistant",
  "Cascadia",
  "ComicShanns",
  "Excalifont",
  "Liberation",
  "Lilita",
  "Nunito",
  "Virgil",
]) {
  await cp(`${fonts}${family}`, `${out}fonts/${family}`, { recursive: true });
}
