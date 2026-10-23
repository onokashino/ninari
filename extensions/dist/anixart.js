// src/players/kodik.ts
var MAIN_HOST = "kodikplayer.com";
var LEGACY_HOSTS = ["kodik.info", "kodik.biz", "kodik.cc", "aniqit.com"];
var KODIK_HOSTS = [
  `*.${MAIN_HOST}`,
  ...LEGACY_HOSTS.map((h) => `*.${h}`),
  "*.solodcdn.com",
  "*.kodik-storage.com",
  "*.kodik-cdn.com"
];
var UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
var DEFAULT_POST_PATH = "/ftor";
var MANIFEST_SUFFIX = ":hls:manifest.m3u8";
var SCRIPT_TTL = 12 * 3600;
var SHIFT_KEY = "kodik:shift";
var KNOWN_SHIFTS = [18, 13];
var HOST_RE = /^(?:https?:)?\/\/([^/?#:]+)/i;
var PATH_RE = /\/(uv|video|seria|episode|season|serial)\/(\d+)\/([0-9a-z]+)/i;
var LINK_RE = /^(?:https?:)?\/\/[a-z0-9.-]+\.[a-z]{2,}(?::\d+)?\/[\x21-\x7e]+$/i;
var hostOf = (url) => HOST_RE.exec(url.trim())?.[1].toLowerCase() ?? "";
var onHost = (host, domain) => host === domain || host.endsWith(`.${domain}`);
function isKodik(url) {
  const host = hostOf(url);
  const known = [MAIN_HOST, ...LEGACY_HOSTS].some((d) => onHost(host, d)) || /(^|\.)kodik[\w-]*\.[a-z]+$/.test(host);
  return known && PATH_RE.test(url);
}
function withQuery(url, set) {
  const [head, hash = ""] = url.split("#", 2);
  const [path, query = ""] = head.split("?", 2);
  const keys = Object.keys(set);
  const pairs = query.split("&").filter((p) => p && !keys.includes(p.split("=")[0]));
  for (const [k, v] of Object.entries(set))
    pairs.push(`${k}=${encodeURIComponent(v)}`);
  return `${path}${pairs.length ? `?${pairs.join("&")}` : ""}${hash ? `#${hash}` : ""}`;
}
function candidateUrls(embedUrl, opts) {
  let url = embedUrl.trim().replace(/^(?:https?:)?\/\//i, "https://");
  const set = {};
  if (opts.episode !== undefined)
    set.episode = String(opts.episode);
  if (opts.season !== undefined)
    set.season = String(opts.season);
  if (Object.keys(set).length)
    url = withQuery(url, set);
  const host = hostOf(url);
  const onMain = url.replace(HOST_RE, `https://${MAIN_HOST}`);
  if (onHost(host, MAIN_HOST))
    return [url];
  if (LEGACY_HOSTS.some((d) => onHost(host, d)))
    return [onMain];
  return [url, onMain];
}
function caesar(s, shift) {
  return s.replace(/[a-zA-Z]/g, (c) => {
    const base = c <= "Z" ? 65 : 97;
    return String.fromCharCode((c.charCodeAt(0) - base + shift) % 26 + base);
  });
}
var padded = (s) => s + "=".repeat((4 - s.length % 4) % 4);
function decodeLink(src, b64decode, hint) {
  if (src.includes("//"))
    return { url: src, shift: hint ?? 0 };
  const shifts = [...new Set([...hint === undefined ? [] : [hint], ...KNOWN_SHIFTS, ...Array.from({ length: 26 }, (_, i) => i)])];
  const reversed = [...src].reverse().join("");
  for (const shift of shifts) {
    for (const variant of [src, reversed]) {
      let out = "";
      try {
        out = b64decode(padded(caesar(variant, shift)));
      } catch {
        continue;
      }
      if (LINK_RE.test(out))
        return { url: out, shift };
    }
  }
  return null;
}
function parseScript(js, b64decode) {
  const encoded = /type:\s*["']POST["']\s*,\s*url:\s*atob\(\s*["']([A-Za-z0-9+/=]+)["']\s*\)/.exec(js)?.[1];
  if (!encoded)
    return null;
  let path = "";
  try {
    path = b64decode(padded(encoded));
  } catch {
    return null;
  }
  if (!/^\/[\w\-/]{1,40}$/.test(path))
    return null;
  const shift = /charCodeAt\(0\)\s*\+\s*(\d{1,2})\s*\)/.exec(js)?.[1];
  return { path, shift: shift === undefined ? undefined : Number(shift) % 26 };
}
function parsePage(html, url) {
  const raw = /urlParams\s*=\s*'(\{[^']*\})'/.exec(html)?.[1] ?? /urlParams\s*=\s*(\{[^;\n]*\})\s*;/.exec(html)?.[1];
  if (!raw)
    return null;
  let params;
  try {
    params = JSON.parse(raw);
  } catch {
    return null;
  }
  const info = {};
  for (const m of html.matchAll(/vInfo\.(type|hash|id)\s*=\s*['"]([^'"]+)['"]/g))
    info[m[1]] ??= m[2];
  const path = PATH_RE.exec(url);
  const type = info.type ?? path?.[1];
  const id = info.id ?? path?.[2];
  const hash = info.hash ?? path?.[3];
  if (!type || !id || !hash)
    return null;
  const script = /src=["'](\/assets\/js\/app\.player_single[^"']*\.js)["']/.exec(html)?.[1] ?? /src=["'](\/assets\/js\/app\.[^"']*\.js)["']/.exec(html)?.[1];
  return { params, type, id, hash, script };
}
var originOf = (url) => `https://${hostOf(url)}`;
async function loadPage(url, opts, ctx) {
  const headers = { "User-Agent": UA, Accept: "text/html,*/*" };
  if (opts.referer)
    headers.Referer = opts.referer;
  const res = await ctx.fetch(url, { headers });
  const html = await res.text();
  const info = res.ok ? parsePage(html, url) : null;
  if (info)
    return info;
  if (res.status === 404 || /not found|не найден|не существует/i.test(html))
    ctx.fail.notFound(`kodik: ${url}`);
  if (/заблокирован|недоступн\S* в ваш|blocked in your/i.test(html))
    ctx.fail.geoBlocked("kodik: видео недоступно в этом регионе");
  throw new Error(`kodik: unexpected embed page (HTTP ${res.status}) ${url}`);
}
async function endpoint(origin, script, fresh, ctx) {
  if (!script)
    return { path: DEFAULT_POST_PATH };
  const key = `kodik:script:${script}`;
  const cached = fresh ? null : await ctx.kv.get(key);
  if (cached) {
    try {
      return JSON.parse(cached);
    } catch {}
  }
  let found = null;
  try {
    const res = await ctx.fetch(origin + script, { headers: { "User-Agent": UA, Referer: `${origin}/` } });
    if (res.ok)
      found = parseScript(await res.text(), (s) => ctx.base64.decode(s));
  } catch (e) {
    ctx.log(`kodik: player script unavailable, using ${DEFAULT_POST_PATH}: ${String(e)}`);
  }
  if (!found)
    return { path: DEFAULT_POST_PATH };
  await ctx.kv.set(key, JSON.stringify(found), SCRIPT_TTL);
  return found;
}
function formBody(info) {
  const p = info.params;
  const str = (k) => typeof p[k] === "string" ? p[k] : "";
  let ref = str("ref");
  try {
    ref = decodeURIComponent(ref);
  } catch {}
  const fields = {
    d: str("d"),
    d_sign: str("d_sign"),
    pd: str("pd"),
    pd_sign: str("pd_sign"),
    ref,
    ref_sign: str("ref_sign"),
    bad_user: "true",
    cdn_is_working: "true",
    type: info.type,
    hash: info.hash,
    id: info.id,
    info: "{}"
  };
  return Object.entries(fields).map(([k, v]) => `${k}=${encodeURIComponent(v)}`).join("&");
}
async function requestLinks(pageUrl, path, info, ctx) {
  const origin = originOf(pageUrl);
  const res = await ctx.fetch(origin + path, {
    method: "POST",
    headers: {
      "User-Agent": UA,
      "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
      "X-Requested-With": "XMLHttpRequest",
      Accept: "application/json, text/javascript, */*; q=0.01",
      Origin: origin,
      Referer: pageUrl
    },
    body: formBody(info)
  });
  if (!res.ok)
    return null;
  try {
    return JSON.parse(await res.text());
  } catch {
    return null;
  }
}
function qualityOf(key, url) {
  const fromFile = /\/(\d{3,4})\.mp4/.exec(url);
  return Number(fromFile?.[1] ?? key) || undefined;
}
function toStreams(answer, pageUrl, hint, ctx) {
  const headers = { Referer: `${originOf(pageUrl)}/`, "User-Agent": UA };
  const streams = [];
  let shift;
  for (const [quality, variants] of Object.entries(answer.links ?? {})) {
    const src = variants?.[0]?.src;
    if (!src)
      continue;
    const decoded = decodeLink(src, (s) => ctx.base64.decode(s), shift ?? hint);
    if (!decoded)
      continue;
    shift = decoded.shift;
    let url = decoded.url.startsWith("//") ? `https:${decoded.url}` : decoded.url;
    if (/\.mp4$/.test(url))
      url += MANIFEST_SUFFIX;
    if (url.includes("/s/m/"))
      ctx.log("kodik: got a proxied /s/m/ link, Kodik is probably rate limiting this IP");
    if (streams.some((st) => st.url === url))
      continue;
    streams.push({ url, kind: "hls", quality: qualityOf(quality, url), headers });
  }
  streams.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
  return { streams, shift };
}
async function resolveOn(url, opts, ctx) {
  const info = await loadPage(url, opts, ctx);
  const origin = originOf(url);
  let ep = await endpoint(origin, info.script, false, ctx);
  let answer = await requestLinks(url, ep.path, info, ctx);
  if (!answer?.links) {
    const fresh = await endpoint(origin, info.script, true, ctx);
    if (fresh.path !== ep.path) {
      ep = fresh;
      answer = await requestLinks(url, ep.path, info, ctx);
    }
  }
  if (!answer)
    throw new Error(`kodik: link request to ${origin}${ep.path} failed`);
  if (answer.error)
    throw new Error(`kodik: ${answer.error}`);
  const stored = await ctx.kv.get(SHIFT_KEY);
  const hint = ep.shift ?? (stored === null ? undefined : Number(stored));
  const { streams, shift } = toStreams(answer, url, hint, ctx);
  if (streams.length === 0) {
    if (Object.keys(answer.links ?? {}).length > 0)
      throw new Error("kodik: could not decode the links, the cipher changed");
    ctx.fail.notFound(`kodik: no links for ${url}`);
  }
  if (shift !== undefined && String(shift) !== stored)
    await ctx.kv.set(SHIFT_KEY, String(shift));
  return streams;
}
async function resolveKodik(embedUrl, ctx, opts = {}) {
  const urls = candidateUrls(embedUrl, opts);
  let last;
  for (const url of urls) {
    try {
      return await resolveOn(url, opts, ctx);
    } catch (e) {
      const msg = String(e);
      if (/__EXT:(NotFound|Challenge|GeoBlocked)/.test(msg) || url === urls[urls.length - 1])
        throw e;
      last = e;
    }
  }
  throw last ?? new Error("kodik: no embed url");
}

// src/players/sibnet.ts
var ORIGIN = "https://video.sibnet.ru";
var UA2 = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
var SIBNET_HOSTS = ["video.sibnet.ru", "*.sibnet.ru"];
var HOST_RE2 = /^(?:https?:)?\/\/([^/?#:]+)/i;
var ID_RES = [/[?&]videoid=(\d+)/i, /\/video(\d+)/i, /\/v\/[0-9a-z]+\/(\d+)\.mp4/i];
var SRC_RE = /player\.src\(\s*\[\s*\{\s*src\s*:\s*["']([^"']+\.mp4[^"']*)["']/i;
var GONE_RE = /class=["']?videostatus/i;
function isSibnet(url) {
  const host = HOST_RE2.exec(url.trim())?.[1].toLowerCase() ?? "";
  return (host === "sibnet.ru" || host.endsWith(".sibnet.ru")) && ID_RES.some((re) => re.test(url));
}
function videoId(url) {
  for (const re of ID_RES) {
    const id = re.exec(url)?.[1];
    if (id)
      return id;
  }
  return;
}
function parseShell(html) {
  const src = SRC_RE.exec(html)?.[1];
  if (!src)
    return;
  if (/^https?:\/\//i.test(src))
    return src;
  return src.startsWith("//") ? `https:${src}` : `${ORIGIN}${src.startsWith("/") ? "" : "/"}${src}`;
}
async function resolveSibnet(embedUrl, ctx) {
  const id = videoId(embedUrl);
  if (!id)
    throw new Error(`sibnet: no video id in ${embedUrl}`);
  const shell = `${ORIGIN}/shell.php?videoid=${id}`;
  const headers = { Referer: `${ORIGIN}/` };
  const res = await ctx.fetch(shell, { headers: { ...headers, "User-Agent": UA2 } });
  const html = await res.text();
  const url = res.ok ? parseShell(html) : undefined;
  if (url)
    return [{ url, kind: "mp4", headers }];
  if (res.status === 404 || GONE_RE.test(html))
    ctx.fail.notFound(`sibnet: video ${id} is gone`);
  throw new Error(`sibnet: no player source on ${shell} (HTTP ${res.status})`);
}

// src/anixart.ts
var manifest = {
  id: "anixart",
  name: "Anixart",
  version: "0.1.1",
  apiVersion: 1,
  domains: [
    "https://api-s.anixsekai.com",
    "https://api.anixsekai.com",
    "https://api.anixart.tv",
    "https://api.anixart.app",
    "https://api-s2.anixart.tv"
  ],
  allowedHosts: ["*.libria.fun", ...KODIK_HOSTS, ...SIBNET_HOSTS]
};
var HEADERS = {
  "User-Agent": "AnixartApp/9.0 BETA 5-25062213 (Android 9; SDK 28; arm64-v8a; samsung SM-G975N; en)",
  "API-Version": "v2"
};
var SOURCES_TTL = 3600;
var EPISODES_TTL = 1800;
var MAX_MATCHES = 10;
var RANK = { libria: 0, kodik: 1, sibnet: 2, other: 3 };
var MAX_QUALITY = { libria: 1080, kodik: 720 };
var norm = (s) => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");
function playerOf(sourceName) {
  const n = sourceName.toLowerCase();
  if (n.includes("не работает"))
    return "other";
  if (n.includes("kodik"))
    return "kodik";
  if (n.includes("sibnet"))
    return "sibnet";
  if (n.includes("libria") || n.includes("liberty"))
    return "libria";
  return "other";
}
async function api(path, ctx, init) {
  const headers = init ? { ...HEADERS, "Content-Type": "application/json" } : HEADERS;
  const res = await ctx.fetch(`${ctx.baseUrl}${path}`, { ...init, headers });
  if (res.status === 404)
    ctx.fail.notFound(`anixart: ${path}`);
  if (!res.ok)
    throw new Error(`anixart: HTTP ${res.status} on ${path}`);
  const body = await res.json();
  if (body.code !== undefined && body.code !== 0)
    throw new Error(`anixart: code ${body.code} on ${path}`);
  return body;
}
async function cached(key, ttl, ctx, load) {
  const hit = await ctx.kv.get(key);
  if (hit) {
    try {
      return JSON.parse(hit);
    } catch {}
  }
  const value = await load();
  await ctx.kv.set(key, JSON.stringify(value), ttl);
  return value;
}
function titles(r) {
  const alt = (r.title_alt ?? "").split(/[\n;|]/);
  return [r.title_ru, r.title_original, ...alt].filter((t) => !!t && !!t.trim()).map(norm);
}
function score(r, q, wanted) {
  let s = titles(r).some((t) => wanted.has(t)) ? 4 : 0;
  const year = Number(r.year);
  if (q.year && year)
    s += Math.abs(year - q.year) <= 1 ? 2 : -3;
  if (q.episodes && r.episodes_total)
    s += r.episodes_total === q.episodes ? 1 : -1;
  return s;
}
async function search(q, ctx) {
  const wanted = new Set(q.names.map(norm).filter(Boolean));
  const found = new Map;
  for (const name of q.names.slice(0, 3)) {
    const body = await api("/search/releases/0", ctx, {
      method: "POST",
      body: JSON.stringify({ query: name, searchBy: 0 })
    });
    for (const r of body.releases ?? body.content ?? []) {
      if (!found.has(r.id))
        found.set(r.id, { r, s: score(r, q, wanted) });
    }
    const best = Math.max(-99, ...[...found.values()].map((f) => f.s));
    if (best >= 6 || best >= 4 && !q.year)
      break;
  }
  return [...found.values()].sort((a, b) => b.s - a.s).slice(0, MAX_MATCHES).map(({ r, s }, i) => ({
    id: String(r.id),
    title: r.title_ru || r.title_original || String(r.id),
    confidence: i === 0 && s >= 4 && (s >= 6 || !q.year) ? "exact" : "fuzzy"
  }));
}
var sourcesOf = (release, type, ctx) => cached(`src:${release}:${type}`, SOURCES_TTL, ctx, async () => ((await api(`/episode/${release}/${type}`, ctx)).sources ?? []).map((s) => ({
  id: s.id,
  name: s.name,
  episodes_count: s.episodes_count ?? 0
})));
var usable = (sources) => sources.filter((s) => playerOf(s.name) !== "other" && (s.episodes_count ?? 0) > 0).sort((a, b) => RANK[playerOf(a.name)] - RANK[playerOf(b.name)]);
async function episodes(m, ctx) {
  const types = ((await api(`/episode/${m.id}`, ctx)).types ?? []).filter((t) => (t.episodes_count ?? 0) > 0);
  const perType = await Promise.all(types.map((t) => sourcesOf(m.id, t.id, ctx).catch(() => null)));
  const dubs = [];
  const counts = [];
  types.forEach((t, i) => {
    const sources = perType[i];
    const good = sources ? usable(sources) : [];
    if (sources && good.length === 0)
      return;
    const dubId = String(t.id);
    dubs.push({ id: dubId, name: t.name.trim(), type: t.is_sub ? "subtitles" : "voice" });
    counts.push({
      dubId,
      count: good.length ? Math.max(...good.map((s) => s.episodes_count ?? 0)) : t.episodes_count ?? 0,
      maxQuality: good.length ? MAX_QUALITY[playerOf(good[0].name)] : undefined
    });
  });
  const total = Math.max(0, ...counts.map((c) => c.count));
  return {
    dubs,
    episodes: Array.from({ length: total }, (_, i) => ({
      number: i + 1,
      available: counts.filter((c) => c.count > i).map(({ dubId, maxQuality }) => ({ dubId, maxQuality }))
    }))
  };
}
function numbered(list) {
  const offset = list.some((e) => e.position === 0) ? 1 : 0;
  return list.filter((e) => !!e.url).map((e) => {
    const named = /^\s*(\d+)\s*серия/i.exec(e.name ?? "")?.[1];
    return { number: named ? Number(named) : e.position + offset, url: e.url };
  });
}
var targetsOf = (release, type, source, ctx) => cached(`eps:${release}:${type}:${source}`, EPISODES_TTL, ctx, async () => numbered((await api(`/episode/${release}/${type}/${source}`, ctx)).episodes ?? []));
function parseLibriaIframe(html, url) {
  const raw = /file\s*:\s*(\[[\s\S]*\])\s*\}\s*\)/.exec(html)?.[1];
  if (!raw)
    return [];
  let files;
  try {
    files = JSON.parse(raw);
  } catch {
    return [];
  }
  const ep = /[?&]ep=(\d+)/.exec(url)?.[1];
  const entry = ep && files.find((f) => f.id === `s${ep}`) || (files.length === 1 ? files[0] : undefined);
  const out = [];
  for (const m of (entry?.file ?? "").matchAll(/\[(\d+)p\]([^,[]+)/g)) {
    out.push({ url: m[2].trim(), kind: "hls", quality: Number(m[1]) });
  }
  return out.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
}
async function resolveLibria(url, ctx) {
  const res = await ctx.fetch(url, { headers: { Referer: "https://anixart.tv/" } });
  if (!res.ok)
    throw new Error(`anixart: libria iframe HTTP ${res.status}`);
  return parseLibriaIframe(await res.text(), url);
}
function play(url, ctx) {
  const abs = url.startsWith("//") ? `https:${url}` : url;
  if (/^https?:\/\/[^/]*libria\.fun\//i.test(abs))
    return resolveLibria(abs, ctx);
  if (isKodik(abs))
    return resolveKodik(abs, ctx);
  if (isSibnet(abs))
    return resolveSibnet(abs, ctx);
  if (/\.m3u8(\?|$)/i.test(abs))
    return Promise.resolve([{ url: abs, kind: "hls" }]);
  if (/\.mp4(\?|$)/i.test(abs))
    return Promise.resolve([{ url: abs, kind: "mp4" }]);
  return Promise.reject(new Error(`anixart: unsupported player ${abs}`));
}
async function resolve(m, episode, dubId, ctx) {
  const sources = usable(await sourcesOf(m.id, Number(dubId), ctx));
  let last;
  for (const s of sources) {
    const target = (await targetsOf(m.id, dubId, s.id, ctx)).find((t) => t.number === episode);
    if (!target)
      continue;
    try {
      const streams = await play(target.url, ctx);
      if (streams.length)
        return streams;
    } catch (e) {
      ctx.log(`anixart: ${s.name} failed: ${e}`);
      last = e;
    }
  }
  if (last)
    throw last;
  ctx.fail.notFound(`anixart: episode ${episode} of dub ${dubId}`);
}
export {
  episodes,
  manifest,
  numbered,
  parseLibriaIframe,
  resolve,
  search
};
