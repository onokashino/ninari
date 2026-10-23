// src/players/cvh.ts
var CVH_HOSTS = ["plapi.cdnvideohub.com"];
var API = "https://plapi.cdnvideohub.com/api/v1/player/sv";
var PUB_ANIMEGO = "747";
var PUB_YUMMY = "745";
var MP4_KEYS = [
  ["mpeg4kUrl", 2160],
  ["mpeg2kUrl", 1440],
  ["mpegQhdUrl", 1440],
  ["mpegFullHdUrl", 1080],
  ["mpegHighUrl", 720],
  ["mpegMediumUrl", 480],
  ["mpegLowUrl", 360],
  ["mpegLowestUrl", 240],
  ["mpegTinyUrl", 144],
  ["mpegMobileUrl", 144]
];
var decode = (s) => {
  try {
    return decodeURIComponent(s.replace(/\+/g, " "));
  } catch {
    return s;
  }
};
function queryOf(url) {
  const q = {};
  const start = url.indexOf("?");
  if (start < 0)
    return q;
  for (const pair of url.slice(start + 1).split("#")[0].split("&")) {
    const eq = pair.indexOf("=");
    if (eq > 0)
      q[decode(pair.slice(0, eq))] = decode(pair.slice(eq + 1));
  }
  return q;
}
var num = (s) => s && /^\d+$/.test(s) ? Number(s) : undefined;
var cvhKey = (name) => (name ?? "").toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");
var cvhStudio = (item) => item.voiceStudio || item.voiceType || "";
function parseCvh(input) {
  const url = input.trim().replace(/&amp;/g, "&");
  if (/^\d+$/.test(url))
    return { pub: PUB_ANIMEGO, aggr: "mali", id: url };
  const video = /\/player\/sv\/video\/(\d+)/.exec(url);
  if (video)
    return { pub: PUB_ANIMEGO, aggr: "mali", video: video[1] };
  const q = queryOf(url);
  if (/cdnvideohub\.com\/api\/v1\/player\/sv\/playlist/.test(url) && q.id) {
    return {
      pub: q.pub || PUB_ANIMEGO,
      aggr: q.aggr || "mali",
      id: q.id,
      studio: q.dubbing_code || undefined,
      season: num(q.season),
      episode: num(q.episode)
    };
  }
  if (/iframeCVH\.html/i.test(url) && q.anime_id) {
    return {
      pub: PUB_YUMMY,
      aggr: "mali",
      id: q.anime_id,
      studio: q.dubbing_code || undefined,
      season: num(q.season),
      episode: num(q.episode)
    };
  }
  const iframe = /\/cdn-iframe\/(\d+)((?:\/[^/?#]+){0,3})/.exec(url);
  if (!iframe)
    return null;
  const segments = iframe[2].split("/").filter(Boolean).map(decode);
  const pathStudio = segments.length > 0 && !/^\d+$/.test(segments[0]) ? segments.shift() : undefined;
  return {
    pub: PUB_ANIMEGO,
    aggr: "mali",
    id: iframe[1],
    studio: q.dubbing || pathStudio,
    season: num(segments[0]),
    episode: num(segments[1])
  };
}
function isCvh(url) {
  return /cdnvideohub\.com|\/cdn-iframe\/\d+|iframeCVH\.html/i.test(url) && parseCvh(url) !== null;
}
function cvhPlaylistUrl(r) {
  let url = `${API}/playlist?pub=${r.pub ?? PUB_ANIMEGO}&aggr=${r.aggr ?? "mali"}&id=${encodeURIComponent(String(r.id))}`;
  if (r.studio)
    url += `&dubbing_code=${encodeURIComponent(r.studio)}`;
  if (r.season != null)
    url += `&season=${r.season}`;
  if (r.episode != null)
    url += `&episode=${r.episode}`;
  return url;
}
async function cvhPlaylist(id, ctx, pub = PUB_ANIMEGO, aggr = "mali") {
  const res = await ctx.fetch(cvhPlaylistUrl({ id, pub, aggr }), { headers: { Accept: "application/json" } });
  if (res.status === 204)
    return [];
  if (!res.ok)
    throw new Error(`cvh: playlist ${aggr}/${id}: HTTP ${res.status}`);
  const text = await res.text();
  if (!text.trim())
    return [];
  const data = JSON.parse(text);
  return (data.items ?? []).filter((i) => i && i.vkId);
}
var QUALITY_TTL = 30 * 24 * 3600;
async function videoQuality(vkId, ctx) {
  const key = `cvh-quality:${vkId}`;
  const known = await ctx.kv.get(key);
  if (known !== null)
    return Number(known) || undefined;
  try {
    const best = Math.max(0, ...(await cvhVideo(vkId, ctx)).map((s) => s.quality ?? 0));
    await ctx.kv.set(key, String(best), QUALITY_TTL);
    return best || undefined;
  } catch {
    return;
  }
}
async function cvhDubQualities(id, ctx, pub = PUB_ANIMEGO, aggr = "mali") {
  const out = new Map;
  let items;
  try {
    items = await cvhPlaylist(id, ctx, pub, aggr);
  } catch {
    return out;
  }
  const firstOfDub = new Map;
  for (const item of items) {
    const key = cvhKey(cvhStudio(item));
    if (key && !firstOfDub.has(key))
      firstOfDub.set(key, item);
  }
  await Promise.all([...firstOfDub].map(async ([key, item]) => {
    const q = await videoQuality(item.vkId, ctx);
    if (q)
      out.set(key, q);
  }));
  return out;
}
function byStudio(items, studio) {
  const want = cvhKey(studio);
  const exact = items.filter((i) => cvhKey(cvhStudio(i)) === want);
  if (exact.length > 0 || !want)
    return exact;
  return items.filter((i) => {
    const k = cvhKey(cvhStudio(i));
    return k !== "" && (k.includes(want) || want.includes(k));
  });
}
function pickCvhItem(items, req, ctx) {
  if (items.length === 0)
    ctx.fail.notFound(`CVH has no title ${req.aggr}/${req.id}`);
  const pool = req.studio ? byStudio(items, req.studio) : items;
  if (pool.length === 0)
    ctx.fail.notFound(`CVH has no dub "${req.studio}" for ${req.aggr}/${req.id}`);
  const episode = req.episode ?? Math.min(...pool.map((i) => i.episode ?? 1));
  const hits = pool.filter((i) => (i.episode ?? 1) === episode);
  if (hits.length === 0)
    ctx.fail.notFound(`CVH has no episode ${episode} in "${req.studio ?? "any dub"}"`);
  return hits.find((i) => req.season != null && i.season === req.season) ?? hits[0];
}
async function cvhVideo(vkId, ctx) {
  const res = await ctx.fetch(`${API}/video/${encodeURIComponent(vkId)}`, { headers: { Accept: "application/json" } });
  if (!res.ok)
    throw new Error(`cvh: video ${vkId}: HTTP ${res.status}`);
  const sources = (await res.json()).sources ?? {};
  const mp4 = [];
  for (const [key, quality] of MP4_KEYS) {
    const url = sources[key];
    if (url && !mp4.some((s) => s.quality === quality))
      mp4.push({ url, kind: "mp4", quality });
  }
  const out = [];
  if (sources.hlsUrl)
    out.push({ url: sources.hlsUrl, kind: "hls", quality: mp4[0]?.quality });
  out.push(...mp4);
  if (out.length === 0)
    throw new Error(`cvh: video ${vkId} has no sources`);
  return out;
}
async function resolveCvh(embedOrId, ctx) {
  const req = parseCvh(embedOrId);
  if (!req)
    throw new Error(`cvh: unsupported player url ${embedOrId}`);
  if (req.video)
    return cvhVideo(req.video, ctx);
  const items = await cvhPlaylist(req.id ?? "", ctx, req.pub, req.aggr);
  return cvhVideo(pickCvhItem(items, req, ctx).vkId, ctx);
}

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

// src/yummy.ts
var manifest = {
  id: "yummy",
  name: "YummyAnime",
  version: "0.1.2",
  apiVersion: 1,
  domains: ["https://api.yani.tv", "https://ru.yummyani.me/api"],
  allowedHosts: [...KODIK_HOSTS, ...CVH_HOSTS, ...SIBNET_HOSTS]
};
var HEADERS = { Accept: "application/json", Lang: "ru" };
var VIDEOS_TTL = 600;
var PLAYERS = [
  { id: "cvh", quality: 1080, matches: isCvh, resolve: resolveCvh },
  { id: "kodik", quality: 720, matches: isKodik, resolve: (url, ctx) => resolveKodik(url, ctx) },
  { id: "sibnet", matches: isSibnet, resolve: resolveSibnet }
];
var normalize = (s) => s.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
var httpsUrl = (url) => url.trim().replace(/^(?:https?:)?\/\//i, "https://");
var span = (s) => s && s.length > 0 ? [s.time, s.time + s.length] : undefined;
function playerOf(url) {
  return PLAYERS.find((p) => {
    try {
      return p.matches(url);
    } catch {
      return false;
    }
  });
}
async function api(path, ctx) {
  const res = await ctx.fetch(`${ctx.baseUrl}${path}`, { headers: HEADERS });
  if (res.status === 404)
    ctx.fail.notFound(`yummy: ${path}`);
  if (!res.ok)
    throw new Error(`yummy: HTTP ${res.status} for ${path}`);
  return (await res.json()).response;
}
function toMatch(a, confidence) {
  return { id: String(a.anime_id), title: a.title ?? String(a.anime_id), confidence };
}
function rankFuzzy(found, q) {
  const names = new Set(q.names.map(normalize));
  const score = (a) => (names.has(normalize(a.title ?? "")) ? 4 : 0) + (q.year && a.year === q.year ? 2 : 0) + (q.kind && a.type?.alias === q.kind ? 1 : 0);
  const conflicts = (a) => !!q.shikimoriId && !!a.remote_ids?.shikimori_id && a.remote_ids.shikimori_id !== q.shikimoriId;
  return found.filter((a) => !conflicts(a)).sort((a, b) => score(b) - score(a)).map((a) => toMatch(a, "fuzzy"));
}
async function search(q, ctx) {
  const byId = [["shikimori_ids", q.shikimoriId]];
  if (q.malId && q.malId !== q.shikimoriId)
    byId.push(["mal_ids", q.malId]);
  for (const [param, id] of byId) {
    if (!id)
      continue;
    const found = await api(`/anime?${param}=${id}&limit=5`, ctx);
    const hit = found.find((a) => a.remote_ids?.shikimori_id === q.shikimoriId || q.malId && a.remote_ids?.myanimelist_id === q.malId);
    if (hit)
      return [toMatch(hit, "exact")];
  }
  const found = new Map;
  for (const name of q.names.slice(0, 3)) {
    for (const a of await api(`/anime?q=${encodeURIComponent(name)}&limit=10`, ctx)) {
      if (q.shikimoriId && a.remote_ids?.shikimori_id === q.shikimoriId)
        return [toMatch(a, "exact")];
      if (!found.has(a.anime_id))
        found.set(a.anime_id, a);
    }
  }
  return rankFuzzy([...found.values()], q);
}
function toEntry(v) {
  const number = (v.number ?? "").trim();
  const dub = (v.data?.dubbing ?? "").trim();
  if (!/^\d+$/.test(number) || !dub || !v.iframe_url)
    return;
  const url = httpsUrl(v.iframe_url);
  const player = playerOf(url);
  if (!player)
    return;
  return { n: Number(number), dub, url, player: player.id, op: span(v.skips?.opening), ed: span(v.skips?.ending) };
}
async function videos(id, ctx) {
  const key = `videos:${id}`;
  const cached = await ctx.kv.get(key);
  if (cached) {
    try {
      return JSON.parse(cached);
    } catch {}
  }
  const list = await api(`/anime/${encodeURIComponent(id)}/videos`, ctx);
  const entries = (list ?? []).map(toEntry).filter((e) => e !== undefined);
  await ctx.kv.set(key, JSON.stringify(entries), VIDEOS_TTL);
  return entries;
}
function dubOf(raw) {
  const sub = /^Субтитры\s*(.*)$/i.exec(raw);
  if (sub)
    return { id: raw, name: sub[1] || "Субтитры", type: "subtitles" };
  const name = raw.replace(/^Озвучка\s+/i, "") || raw;
  return { id: raw, name, type: /^(Оригинал|Original)\b/i.test(name) ? "original" : "voice" };
}
function commonSpan(spans) {
  const counts = new Map;
  for (const s of spans)
    if (s)
      counts.set(JSON.stringify(s), (counts.get(JSON.stringify(s)) ?? 0) + 1);
  let best;
  for (const [k, c] of counts)
    if (best === undefined || c > counts.get(best))
      best = k;
  return best === undefined ? undefined : JSON.parse(best);
}
var qualityOf2 = (e, cvh) => {
  if (e.player === "cvh") {
    const studio = parseCvh(e.url)?.studio;
    const known = studio ? cvh.get(cvhKey(studio)) : undefined;
    if (known)
      return known;
  }
  return PLAYERS.find((p) => p.id === e.player)?.quality;
};
var maxDefined = (a, b) => a === undefined ? b : b === undefined ? a : Math.max(a, b);
async function episodes(m, ctx) {
  const entries = await videos(m.id, ctx);
  const cvhEntry = entries.find((e) => e.player === "cvh");
  const cvhReq = cvhEntry ? parseCvh(cvhEntry.url) : null;
  const cvh = cvhReq?.id ? await cvhDubQualities(cvhReq.id, ctx, cvhReq.pub, cvhReq.aggr) : new Map;
  const dubs = new Map;
  const byEpisode = new Map;
  for (const e of entries) {
    if (!dubs.has(e.dub))
      dubs.set(e.dub, dubOf(e.dub));
    const bucket = byEpisode.get(e.n);
    if (bucket)
      bucket.push(e);
    else
      byEpisode.set(e.n, [e]);
  }
  const list = [...byEpisode.entries()].sort(([a], [b]) => a - b);
  return {
    dubs: [...dubs.values()],
    episodes: list.map(([number, es]) => {
      const best = new Map;
      for (const e of es)
        best.set(e.dub, maxDefined(best.get(e.dub), qualityOf2(e, cvh)));
      return {
        number,
        available: [...best].map(([dubId, maxQuality]) => ({ dubId, maxQuality })),
        skip: { opening: commonSpan(es.map((e) => e.op)), ending: commonSpan(es.map((e) => e.ed)) }
      };
    })
  };
}
var rank = (e) => PLAYERS.findIndex((p) => p.id === e.player);
async function resolve(m, episode, dubId, ctx) {
  const entries = (await videos(m.id, ctx)).filter((e) => e.n === episode && e.dub === dubId).sort((a, b) => rank(a) - rank(b));
  if (entries.length === 0)
    ctx.fail.notFound(`episode ${episode} in ${dubId}`);
  const errors = [];
  for (const e of entries) {
    const player = PLAYERS.find((p) => p.id === e.player);
    if (!player)
      continue;
    try {
      const streams = await player.resolve(e.url, ctx);
      if (streams.length > 0)
        return streams;
    } catch (err) {
      ctx.log(`yummy: ${player.id} failed for ${e.url}: ${String(err)}`);
      errors.push(err);
    }
  }
  throw errors.find((err) => String(err).includes("__EXT:")) ?? errors[errors.length - 1] ?? new Error("yummy: no playable player");
}
export {
  episodes,
  manifest,
  resolve,
  search
};
