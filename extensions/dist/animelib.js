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

// src/animelib.ts
var manifest = {
  id: "animelib",
  name: "AnimeLib",
  version: "0.1.1",
  apiVersion: 1,
  domains: ["https://api.animelib.org", "https://api.cdnlibs.org"],
  allowedHosts: [...KODIK_HOSTS]
};
var HEADERS = { Accept: "application/json", "Site-Id": "5" };
var LIST_TTL = 1800;
var PLAYERS_TTL = 6 * 3600;

class Retry extends Error {
}
async function fetchOn(base, path, ctx) {
  let res;
  try {
    res = await ctx.fetch(`${base}/api${path}`, { headers: HEADERS });
  } catch (e) {
    if (/__EXT:(Challenge|Blocked)/.test(String(e)))
      throw new Retry(String(e));
    throw e;
  }
  if (res.status === 404)
    ctx.fail.notFound(`animelib: ${path}`);
  if (res.status === 403 || res.status >= 500)
    throw new Retry(`animelib: HTTP ${res.status} on ${base}`);
  if (!res.ok)
    throw new Error(`animelib: HTTP ${res.status} on ${path}`);
  return res.json();
}
async function api(path, ctx) {
  const bases = [ctx.baseUrl, ...manifest.domains.filter((d) => d !== ctx.baseUrl)];
  let last;
  for (const base of bases) {
    try {
      return await fetchOn(base, path, ctx);
    } catch (e) {
      if (!(e instanceof Retry))
        throw e;
      last = e;
    }
  }
  throw last;
}
async function cached(key, ttl, ctx, load, fresh = false) {
  const hit = fresh ? null : await ctx.kv.get(key);
  if (hit) {
    try {
      return JSON.parse(hit);
    } catch {}
  }
  const value = await load();
  await ctx.kv.set(key, JSON.stringify(value), ttl);
  return value;
}
var shikiId = (href) => Number(/\/animes\/[a-z]*(\d+)/.exec(href ?? "")?.[1] ?? NaN);
var norm = (s) => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");
async function search(q, ctx) {
  const wanted = new Set(q.names.map(norm).filter(Boolean));
  const fuzzy = new Map;
  for (const name of q.names.slice(0, 3)) {
    const found = (await api(`/anime?q=${encodeURIComponent(name)}`, ctx)).data ?? [];
    for (const a of found) {
      const title = a.rus_name || a.name || a.eng_name || String(a.id);
      if (shikiId(a.shikimori_href) === q.shikimoriId)
        return [{ id: String(a.id), title, confidence: "exact" }];
      const named = [a.rus_name, a.name, a.eng_name].some((t) => !!t && wanted.has(norm(t)));
      if (!fuzzy.has(a.id))
        fuzzy.set(a.id, { id: String(a.id), title, confidence: "fuzzy", named });
    }
  }
  return [...fuzzy.values()].sort((a, b) => Number(b.named) - Number(a.named)).map(({ named: _, ...m }) => m);
}
function numbered(list) {
  const byNumber = list.map((e) => ({ id: e.id, n: Number(e.number) }));
  const unique = new Set(byNumber.map((e) => e.n)).size === byNumber.length;
  const eps = unique ? byNumber : list.map((e, i) => ({ id: e.id, n: e.item_number ?? i + 1 }));
  return eps.filter((e) => Number.isInteger(e.n));
}
var episodeList = (anime, ctx) => cached(`eps:${anime}`, LIST_TTL, ctx, async () => numbered((await api(`/episodes?anime_id=${encodeURIComponent(anime)}`, ctx)).data ?? []));
function dubType(p) {
  const label = (p.translation_type?.label ?? "").toLowerCase();
  if (p.translation_type?.id === 1 || label.includes("субтитр"))
    return "subtitles";
  if (label.includes("оригинал"))
    return "original";
  return "voice";
}
function reducePlayers(players) {
  return players.filter((p) => p.team && p.src).map((p) => {
    const type = dubType(p);
    return { dub: `${p.team.id}-${type[0]}`, name: p.team.name.trim(), type, player: p.player, src: p.src };
  });
}
var playersOf = (episodeId, ctx, fresh = false) => cached(`pl:${episodeId}`, PLAYERS_TTL, ctx, async () => reducePlayers((await api(`/episodes/${episodeId}`, ctx)).data?.players ?? []), fresh);
async function knownPlayers(episodeId, ctx) {
  const hit = await ctx.kv.get(`pl:${episodeId}`);
  if (!hit)
    return null;
  try {
    return JSON.parse(hit);
  } catch {
    return null;
  }
}
var maxQuality = (p) => isKodik(p.src) ? Number(/\/(\d{3,4})p$/.exec(p.src)?.[1] ?? 720) : undefined;
function availability(eps, known) {
  const dubs = new Map;
  const last = new Map;
  const quality = new Map;
  known.forEach((plays, i) => {
    for (const p of plays ?? []) {
      if (!dubs.has(p.dub))
        dubs.set(p.dub, { id: p.dub, name: p.name, type: p.type });
      last.set(p.dub, Math.max(i, last.get(p.dub) ?? -1));
      quality.set(p.dub, Math.max(quality.get(p.dub) ?? 0, maxQuality(p) ?? 0) || undefined);
    }
  });
  return {
    dubs: [...dubs.values()],
    episodes: eps.map((e, i) => {
      const exact = known[i];
      const ids = exact ? [...new Set(exact.map((p) => p.dub))] : [...last].filter(([, k]) => k >= i).map(([d]) => d);
      return { number: e.n, available: ids.map((dubId) => ({ dubId, maxQuality: quality.get(dubId) })) };
    })
  };
}
async function episodes(m, ctx) {
  const eps = await episodeList(m.id, ctx);
  if (eps.length === 0)
    return { dubs: [], episodes: [] };
  const sampled = new Set([0, eps.length - 1]);
  const known = await Promise.all(eps.map((e, i) => sampled.has(i) ? playersOf(e.id, ctx) : knownPlayers(e.id, ctx)));
  return availability(eps, known);
}
async function play(p, ctx) {
  if (isKodik(p.src))
    return resolveKodik(p.src, ctx);
  throw new Error(`animelib: unsupported player ${p.player}`);
}
async function resolve(m, episode, dubId, ctx) {
  const ep = (await episodeList(m.id, ctx)).find((e) => e.n === episode);
  if (!ep)
    ctx.fail.notFound(`animelib: episode ${episode}`);
  let plays = (await playersOf(ep.id, ctx)).filter((p) => p.dub === dubId);
  if (plays.length === 0)
    plays = (await playersOf(ep.id, ctx, true)).filter((p) => p.dub === dubId);
  if (plays.length === 0)
    ctx.fail.notFound(`animelib: episode ${episode} has no dub ${dubId}`);
  let last;
  for (const p of plays) {
    try {
      const streams = await play(p, ctx);
      if (streams.length)
        return streams;
    } catch (e) {
      ctx.log(`animelib: ${p.player} failed: ${e}`);
      last = e;
    }
  }
  throw last ?? new Error(`animelib: no playable player for dub ${dubId}`);
}
export {
  availability,
  episodes,
  manifest,
  numbered,
  reducePlayers,
  resolve,
  search
};
