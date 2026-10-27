// R43.31: Node >= 25 ships an experimental global `localStorage` /
// `sessionStorage` (Web Storage) on `globalThis`. Without
// `--localstorage-file` its getter returns `undefined` (plus a warning).
// Vitest's jsdom environment only copies window keys that are NOT already
// on the global (unless they are in its fixed allowlist, which omits the
// storage objects), so Node's empty getter shadows jsdom's working Storage.
// Re-point both names at the jsdom window. Harmless on Node 20/22, where
// vitest already exposes jsdom's Storage under the same names.
type JsdomGlobal = typeof globalThis & {
  jsdom?: { window: Pick<Window, "localStorage" | "sessionStorage"> };
};

const dom = (globalThis as JsdomGlobal).jsdom;
if (dom) {
  for (const key of ["localStorage", "sessionStorage"] as const) {
    Object.defineProperty(globalThis, key, {
      get: () => dom.window[key],
      configurable: true,
    });
  }
}
