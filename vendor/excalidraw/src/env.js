// Must run before Excalidraw loads: fonts come from the app, not a CDN.
window.EXCALIDRAW_ASSET_PATH = new URL("./", window.location.href).href;
