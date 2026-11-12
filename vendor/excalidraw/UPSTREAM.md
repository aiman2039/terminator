# Bundled Excalidraw

Source: https://github.com/excalidraw/excalidraw (MIT, see LICENSE)
Pinned npm package: `@excalidraw/excalidraw` 0.18.1, with `react` and
`react-dom` 19.2.0. Exact versions are in `package-lock.json`.

`dist/` is the built page and is committed. The app embeds it with
`include_dir` (`crates/app/src/browser/draw.rs`) and serves it to drawing tabs
from the `terminator-draw` scheme. No Node.js is necessary to build Terminator,
and the page loads nothing from the network at start.

Terminator code is `src/` (the page and its save/load calls) and `build.mjs`.
The Excalidraw package is used unchanged. The CJK font (Xiaolai, 12 MB) is left
out; CJK text uses a system font.

`libraries/` holds shape libraries from
https://github.com/excalidraw/excalidraw-libraries (MIT), unchanged, as of
2026-11-12. `libraries.json` names them; `node fetch-libraries.mjs` downloads
them again. The page adds a new bundled set to the user's library one time,
so items that the user removes stay removed until the set changes.

The user's library is `excalidraw-library.excalidrawlib` in the data folder.

"Browse libraries" opens https://libraries.excalidraw.com in a Browser tab.
Its "Add to Excalidraw" link points back at the drawing page; the app refuses
the navigation and sets the `#addLibrary=` hash on the drawing view. The page
then downloads that library from the site. This is the only network use of
the page, and only after that click.

To update the pin or change `src/`, with Node.js 20+:

```sh
npm install
npm run build
```

Then commit `dist/` and `package-lock.json`, and open a `.excalidraw` file in
the app: draw, check that the file changes, and check that an edit of the file
by another program shows on the canvas.
