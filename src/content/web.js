// The browser's side of the web's bundled content (src/content/web.rs): the index at start, the packs' parts, Cache
// Storage and the LRU book. A part's body stays here, a Uint8Array outside the wasm's memory.

const CACHE = "honeydew-packs-v1";
const BOOK = "honeydew.packs";
// Each part downloading, by URL: its AbortController.
const downloads = new Map();
let store;
let persisted = false;

// Cache Storage, or null where there is none (a non-secure origin, some private modes): then the parts stay in memory
// for the session.
function open() {
  store ??= (async () => caches.open(CACHE))().catch((err) => {
    console.warn(`No Cache Storage (${err}): the sounds stay in memory.`);
    return null;
  });
  return store;
}

// The content index: from the network, for 5 s at most, and kept in the cache for a start offline; else the cache's
// copy. null without either.
export async function index(url) {
  const cache = await open();
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 5000);
  try {
    const res = await fetch(url, { signal: ctl.signal });
    if (res.ok) {
      const bytes = new Uint8Array(await res.clone().arrayBuffer());
      cache?.put(url, res).catch(() => {});
      return bytes;
    }
  } catch {
  } finally {
    clearTimeout(timer);
  }
  const hit = await cache?.match(url);
  return hit ? new Uint8Array(await hit.arrayBuffer()) : null;
}

// The origin's storage quota in bytes, 0 when the browser doesn't say.
export async function quota() {
  try {
    return (await navigator.storage.estimate()).quota ?? 0;
  } catch {
    return 0;
  }
}

// The device has 4 GB of memory or less, or doesn't say (navigator.deviceMemory is Chrome's).
export function small() {
  return !(navigator.deviceMemory > 4);
}

export function book() {
  try {
    return localStorage.getItem(BOOK) ?? "";
  } catch {
    return "";
  }
}

export function save(json) {
  try {
    localStorage.setItem(BOOK, json);
  } catch {}
}

export function evict(url) {
  open().then((cache) => cache?.delete(url));
}

export function abort(url) {
  downloads.get(url)?.abort();
  downloads.delete(url);
}

// The part at `url` as `[Uint8Array, cached]`: the cache's copy, else downloaded, checked against `sha` (the base64
// SHA-256 of the tar) and cached; `cached` is false when Cache Storage refused it (over the quota) or there is none. A
// retry (`reload`) goes past the HTTP cache. Rejects with "offline" when the browser is offline and the part is not
// cached, with "aborted" once `abort` stopped it.
export async function part(url, sha, reload) {
  const ctl = new AbortController();
  downloads.set(url, ctl);
  try {
    const cache = await open();
    const hit = await cache?.match(url);
    if (hit) return [new Uint8Array(await hit.arrayBuffer()), true];
    if (navigator.onLine === false) throw "offline";
    const tar = await download(url, sha, { signal: ctl.signal, cache: reload ? "reload" : "default" });
    // Over the quota: it stays in memory for the session.
    const put = cache?.put(url, new Response(tar)).then(
      () => true,
      (err) => {
        console.warn(`${url} not cached: ${err}`);
        return false;
      },
    );
    if (!persisted) {
      persisted = true;
      navigator.storage?.persist?.();
    }
    return [tar, (await put) ?? false];
  } catch (err) {
    throw ctl.signal.aborted ? "aborted" : err;
  } finally {
    if (downloads.get(url) === ctl) downloads.delete(url);
  }
}

// A text file beside the index (CREDITS.txt).
export async function notices(url) {
  const res = await fetch(url);
  if (!res.ok) throw `HTTP ${res.status}`;
  return res.text();
}

// index.html's rule for the wasm: the `.gz` beside the file, gunzipped here unless the host sent it with
// Content-Encoding (the browser has then), its tar's SHA-256 checked by hand. No `.gz` (or no WebCrypto, which needs a
// secure origin): the plain file, which the browser checks against `integrity`.
async function download(url, sha, init) {
  let res = crypto.subtle && (await fetch(`${url}.gz`, init));
  if (!res?.ok || /html/.test(res.headers.get("content-type"))) {
    res = await fetch(url, { ...init, integrity: `sha256-${sha}` });
    if (!res.ok) throw `HTTP ${res.status}`;
    return new Uint8Array(await res.arrayBuffer());
  }
  const gunzipped = /gzip/.test(res.headers.get("content-encoding"));
  const body = gunzipped ? res.body : res.body.pipeThrough(new DecompressionStream("gzip"));
  const tar = new Uint8Array(await new Response(body).arrayBuffer());
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", tar));
  if (btoa(String.fromCharCode(...digest)) !== sha) throw "SHA-256 mismatch";
  return tar;
}
