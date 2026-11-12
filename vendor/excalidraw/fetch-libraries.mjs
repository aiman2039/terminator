// Downloads the shape libraries named in libraries.json into libraries/.
// Source: https://github.com/excalidraw/excalidraw-libraries (MIT).
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";

const base = "https://raw.githubusercontent.com/excalidraw/excalidraw-libraries/main/libraries/";
const out = new URL("./libraries/", import.meta.url);
const sources = JSON.parse(await readFile(new URL("./libraries.json", import.meta.url), "utf8"));

await rm(out, { recursive: true, force: true });
await mkdir(out);
let items = 0;
for (const source of sources) {
  const response = await fetch(`${base}${source}.excalidrawlib`);
  if (!response.ok) {
    throw new Error(`${source}: HTTP ${response.status}`);
  }
  const text = await response.text();
  const data = JSON.parse(text);
  if (data.type !== "excalidrawlib") {
    throw new Error(`${source}: not a library`);
  }
  items += (data.libraryItems ?? data.library ?? []).length;
  const name = source.replace("/", "--").toLowerCase();
  await writeFile(new URL(`${name}.excalidrawlib`, out), text);
}
console.log(`${sources.length} libraries, ${items} items`);
