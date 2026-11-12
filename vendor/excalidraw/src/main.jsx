// Terminator drawing tab. The app serves this page and one scene file over
// the `terminator-draw` scheme; the page has no other way into the app.
import "./env.js";
import React from "react";
import { createRoot } from "react-dom/client";
import {
  CaptureUpdateAction,
  Excalidraw,
  MainMenu,
  exportToBlob,
  exportToSvg,
  getSceneVersion,
  mergeLibraryItems,
  restore,
  restoreLibraryItems,
  serializeAsJSON,
  serializeLibraryAsJSON,
  useHandleLibrary,
} from "@excalidraw/excalidraw";
import "@excalidraw/excalidraw/index.css";

const EXPORT_KEY = navigator.platform.includes("Mac") ? "Cmd+Shift+E" : "Ctrl+Shift+E";

const libraryItems = (data, status) =>
  restoreLibraryItems(data.libraryItems ?? data.library ?? [], status);

/// The library file text. `bundled` records which bundled set it has.
const libraryText = (items) =>
  JSON.stringify({ ...JSON.parse(serializeLibraryAsJSON(items)), bundled: BUNDLED_LIBRARIES });

/// The user's library file plus the bundled shapes. The bundled set is
/// added one time for each new set (`fresh`), so items the user removes
/// stay removed. A file that cannot be read is never replaced.
async function loadLibrary() {
  const bundled = async () =>
    (await import("bundled-libraries")).default.flatMap((data) => libraryItems(data, "published"));
  try {
    const response = await fetch("library", { cache: "no-store" });
    const text = await response.text();
    if (!response.ok) {
      throw new Error(text);
    }
    const data = text.trim() ? JSON.parse(text) : {};
    const items = libraryItems(data, "unpublished");
    return data.bundled === BUNDLED_LIBRARIES
      ? { items, writable: true, fresh: false }
      : { items: mergeLibraryItems(items, await bundled()), writable: true, fresh: true };
  } catch {
    return { items: await bundled(), writable: false, fresh: false };
  }
}

const query = new URLSearchParams(window.location.search);
const id = encodeURIComponent(query.get("id") ?? "");
const theme = query.get("theme") === "dark" ? "dark" : "light";
// Where the libraries site sends "Add to Excalidraw": this drawing.
const RETURN_URL = new URL(`index.html?id=${id}`, window.location.href).href;
const SAVE_DELAY = 500;
const POLL = 2000;

const sceneUrl = (stamp) =>
  `scene?id=${id}${stamp ? `&stamp=${encodeURIComponent(stamp)}` : ""}`;

/// `{stamp, text?}`: no `text` means the file did not change since `stamp`.
async function fetchScene(stamp) {
  const response = await fetch(sceneUrl(stamp), { cache: "no-store" });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  return response.json();
}

async function post(path, body) {
  const response = await fetch(`${path}?id=${id}`, { method: "POST", body });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(text);
  }
  return text;
}

function parse(text) {
  const data = text.trim() ? JSON.parse(text) : {};
  return restore(data, null, null);
}

/// What a save must keep: elements, image count and canvas color.
const fingerprint = (elements, appState, files) =>
  `${getSceneVersion(elements)}:${Object.keys(files).length}:${appState.viewBackgroundColor}`;

function App({ initial }) {
  const api = React.useRef(null);
  const state = React.useRef({
    stamp: initial.stamp,
    saved: fingerprint(initial.scene.elements, initial.scene.appState, initial.scene.files),
    timer: null,
    saving: false,
  });

  const save = React.useCallback(async () => {
    const current = state.current;
    current.timer = null;
    if (current.saving || !api.current) {
      return;
    }
    const elements = api.current.getSceneElements();
    const appState = api.current.getAppState();
    const files = api.current.getFiles();
    const next = fingerprint(elements, appState, files);
    current.saving = true;
    try {
      current.stamp = await post("scene", serializeAsJSON(elements, appState, files, "local"));
      current.saved = next;
    } catch (error) {
      api.current.setToast({ message: `Could not save: ${error.message}`, closable: true });
    } finally {
      current.saving = false;
    }
  }, []);

  const onChange = React.useCallback(
    (elements, appState, files) => {
      const current = state.current;
      if (fingerprint(elements, appState, files) === current.saved) {
        return;
      }
      clearTimeout(current.timer);
      current.timer = setTimeout(save, SAVE_DELAY);
    },
    [save],
  );

  // Another program (an agent) changed the file: show it, unless there
  // are local changes that are not on disk yet.
  React.useEffect(() => {
    const timer = setInterval(async () => {
      const current = state.current;
      if (current.timer || current.saving || !api.current) {
        return;
      }
      try {
        const { stamp, text } = await fetchScene(current.stamp);
        if (text === undefined || current.timer || current.saving) {
          return;
        }
        const scene = parse(text);
        current.stamp = stamp;
        current.saved = fingerprint(scene.elements, scene.appState, scene.files);
        api.current.addFiles(Object.values(scene.files));
        api.current.updateScene({
          elements: scene.elements,
          appState: { viewBackgroundColor: scene.appState.viewBackgroundColor },
          captureUpdate: CaptureUpdateAction.NEVER,
        });
      } catch {
        // A file that is mid-write or not valid JSON: keep the canvas.
      }
    }, POLL);
    return () => clearInterval(timer);
  }, []);

  /// Writes `<file>.png` or `<file>.svg` beside the scene file.
  const saveImage = React.useCallback(async (kind) => {
    try {
      const scene = {
        elements: api.current.getSceneElements(),
        appState: { ...api.current.getAppState(), exportBackground: true },
        files: api.current.getFiles(),
        exportPadding: 16,
      };
      const body =
        kind === "svg"
          ? (await exportToSvg(scene)).outerHTML
          : await exportToBlob({ ...scene, mimeType: "image/png" });
      const name = await post(kind, body);
      api.current.setToast({ message: `Saved ${name}` });
    } catch (error) {
      api.current.setToast({ message: `Could not save image: ${error.message}`, closable: true });
    }
  }, []);

  // Before Excalidraw sees the key: its own export dialog only downloads.
  React.useEffect(() => {
    const onKey = (event) => {
      if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.code === "KeyE") {
        event.preventDefault();
        event.stopPropagation();
        saveImage("png");
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [saveImage]);

  // A new bundled set is not in the file yet: the first call saves it.
  const library = React.useRef(initial.library.fresh ? "" : libraryText(initial.library.items));
  const onLibraryChange = React.useCallback(
    async (items) => {
      const text = libraryText(items);
      if (!initial.library.writable || text === library.current) {
        return;
      }
      library.current = text;
      try {
        await post("library", text);
      } catch (error) {
        api.current?.setToast({ message: `Could not save library: ${error.message}`, closable: true });
      }
    },
    [initial],
  );
  React.useEffect(() => {
    onLibraryChange(initial.library.items);
  }, [initial, onLibraryChange]);

  // "Add to Excalidraw" on the libraries site: the app sets this page's
  // `#addLibrary=` hash, and Excalidraw gets the library from that site.
  const [handle, setHandle] = React.useState(null);
  useHandleLibrary({ excalidrawAPI: handle });

  return (
    <Excalidraw
      excalidrawAPI={(value) => {
        api.current = value;
        setHandle(value);
      }}
      libraryReturnUrl={RETURN_URL}
      initialData={{
        ...initial.scene,
        libraryItems: initial.library.items,
        scrollToContent: true,
      }}
      onChange={onChange}
      onLibraryChange={onLibraryChange}
      theme={theme}
      UIOptions={{
        canvasActions: {
          loadScene: false,
          export: false,
          saveAsImage: false,
          saveToActiveFile: false,
          toggleTheme: false,
        },
      }}
    >
      <MainMenu>
        <MainMenu.Item onSelect={() => saveImage("png")} shortcut={EXPORT_KEY}>
          Save PNG beside file
        </MainMenu.Item>
        <MainMenu.Item onSelect={() => saveImage("svg")}>Save SVG beside file</MainMenu.Item>
        <MainMenu.DefaultItems.SearchMenu />
        <MainMenu.DefaultItems.ClearCanvas />
        <MainMenu.DefaultItems.Help />
        <MainMenu.Separator />
        <MainMenu.DefaultItems.ChangeCanvasBackground />
      </MainMenu>
    </Excalidraw>
  );
}

const root = document.getElementById("root");
document.body.classList.toggle("dark", theme === "dark");
try {
  const [{ stamp, text }, library] = await Promise.all([fetchScene(""), loadLibrary()]);
  createRoot(root).render(<App initial={{ stamp, scene: parse(text ?? ""), library }} />);
} catch (error) {
  // Never open an editor over a file it cannot read: a save would
  // replace the file.
  root.className = "error";
  root.textContent = `Could not open this drawing: ${error.message}. Use Open as text.`;
}
