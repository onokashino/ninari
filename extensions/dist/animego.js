// src/players/aniboom.ts
var ANIBOOM_HOSTS = ["aniboom.one"];
var DEFAULT_REFERER = "https://animego.me/";
var STREAM_HEADERS = { Referer: "https://aniboom.one/", Origin: "https://aniboom.one" };
var absolute = (url) => {
  const u = url.trim().replace(/&amp;/g, "&");
  return u.startsWith("//") ? `https:${u}` : u;
};
function hlsSrc(raw) {
  if (!raw || raw === "null")
    return;
  try {
    return JSON.parse(raw).src || undefined;
  } catch {
    return;
  }
}
var TIERS = [240, 360, 480, 720, 1080, 1440, 2160];
var tier = (height) => height && height > 0 ? TIERS.find((t) => t >= height) ?? height : undefined;
var isChallenge = (status, body) => (status === 403 || status === 503) && /ddos-guard|just a moment|cf-chl/i.test(body.slice(0, 4096));
async function resolveAniboom(embedUrl, ctx, referer = DEFAULT_REFERER) {
  const url = absolute(embedUrl);
  const res = await ctx.fetch(url, { headers: { Referer: referer } });
  const body = await res.text();
  if (isChallenge(res.status, body))
    ctx.fail.challenge(url);
  if (res.status === 404)
    ctx.fail.notFound(`aniboom has no such video: ${url}`);
  if (!res.ok)
    throw new Error(`aniboom: HTTP ${res.status} for ${url}`);
  const raw = ctx.html.select(body, "video[data-parameters]")[0]?.attrs["data-parameters"];
  if (!raw)
    throw new Error(`aniboom: no player parameters on ${url}`);
  const params = JSON.parse(raw);
  const out = [];
  const main = hlsSrc(params.hls);
  const fallback = hlsSrc(params.fallbackHls);
  if (main)
    out.push({ url: main, kind: "hls", quality: tier(params.qualityVideo), headers: { ...STREAM_HEADERS } });
  if (fallback && fallback !== main)
    out.push({ url: fallback, kind: "hls", headers: { ...STREAM_HEADERS } });
  if (out.length === 0)
    throw new Error(`aniboom: no HLS source on ${url}`);
  return out;
}

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

// src/animego.ts
var manifest = {
  id: "animego",
  name: "AnimeGO",
  version: "0.1.2",
  apiVersion: 1,
  domains: ["https://animego.me", "https://animego.org", "https://animego.one", "https://animego.club"],
  allowedHosts: [...ANIBOOM_HOSTS, ...CVH_HOSTS, ...KODIK_HOSTS]
};
var PLAYER_ORDER = ["aniboom", "cvh", "kodik"];
var HD = 1080;
var SD = 720;
var PROBE_BUDGET_MS = 2500;
var MAX_PROBES = 6;
var PLAYER_REUSE = 60;
var PLAYER_TTL = 600;
var PROBES_TTL = 3 * 3600;
var absolute2 = (url) => {
  const u = url.trim().replace(/&amp;/g, "&");
  return u.startsWith("//") ? `https:${u}` : u;
};
var norm = (s) => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");
function bigrams(s) {
  const out = new Map;
  for (let i = 0;i < s.length - 1; i++)
    out.set(s.slice(i, i + 2), (out.get(s.slice(i, i + 2)) ?? 0) + 1);
  return out;
}
function similarity(a, b) {
  const x = norm(a);
  const y = norm(b);
  if (!x || !y)
    return 0;
  if (x === y)
    return 1;
  const bx = bigrams(x);
  const by = bigrams(y);
  let common = 0;
  for (const [g, n] of bx)
    common += Math.min(n, by.get(g) ?? 0);
  return 2 * common / (x.length - 1 + y.length - 1 || 1);
}
async function xhr(path, ctx) {
  const res = await ctx.fetch(`${ctx.baseUrl}${path}`, {
    headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json", Referer: `${ctx.baseUrl}/` }
  });
  if (res.status === 404)
    ctx.fail.notFound(`animego ${path}`);
  if (!res.ok)
    throw new Error(`animego: HTTP ${res.status} for ${path}`);
  const body = await res.json();
  if (body.status !== "success")
    throw new Error(`animego: ${path} answered ${body.status}`);
  return body.data?.content ?? "";
}
function parseProviders(html, ctx) {
  return ctx.html.select(html, '[data-anime-player-target="provider"]').map((n) => ({
    tr: n.attrs["data-translation-id"] ?? "",
    name: (n.attrs["data-translation-title"] ?? "").trim(),
    slug: n.attrs["data-provider-slug"] ?? "",
    url: absolute2(n.attrs["data-player"] ?? "")
  })).filter((p) => p.tr && p.url);
}
function parseEpisodes(html, ctx) {
  return ctx.html.select(html, "[data-episode-number][data-episode]").map((n) => ({
    number: Number(n.attrs["data-episode-number"]),
    id: n.attrs["data-episode"] ?? "",
    active: /\bactive\b/.test(n.attrs.class ?? "")
  })).filter((e) => Number.isInteger(e.number) && e.id !== "");
}
function pagedNumbers(html, ctx) {
  const raw = ctx.html.select(html, "[data-anime-player-episodes-pages-value]")[0]?.attrs["data-anime-player-episodes-pages-value"];
  if (!raw)
    return [];
  try {
    const pages = JSON.parse(raw);
    const out = [];
    for (const p of pages) {
      if (p.count !== p.lastNumber - p.firstNumber + 1)
        continue;
      for (let n = p.firstNumber;n <= p.lastNumber; n++)
        out.push(n);
    }
    return out;
  } catch {
    return [];
  }
}
async function fetchPlayer(id, ctx) {
  const html = await xhr(`/player/${encodeURIComponent(id)}`, ctx);
  const eps = parseEpisodes(html, ctx);
  const providers = parseProviders(html, ctx);
  const names = {};
  for (const n of ctx.html.select(html, '[data-anime-player-target="translation"]')) {
    const tr = n.attrs["data-translation-id"];
    const name = n.text.replace("(ошибка)", "").replace(/\s+/g, " ").trim();
    if (tr && name)
      names[tr] = name;
  }
  for (const p of providers)
    if (p.name)
      names[p.tr] = p.name;
  const numbers = [...new Set([...eps.map((e) => e.number), ...pagedNumbers(html, ctx)])].sort((a, b) => a - b);
  if (numbers.length === 0 && providers.length > 0)
    numbers.push(1);
  return {
    at: Date.now(),
    ids: Object.fromEntries(eps.map((e) => [e.number, e.id])),
    numbers,
    current: eps.find((e) => e.active)?.number ?? numbers[0] ?? 1,
    providers,
    names
  };
}
async function player(id, ctx, maxAgeSec) {
  const cached = await ctx.kv.get(`player:${id}`);
  const p = cached ? JSON.parse(cached) : null;
  if (p && Date.now() - p.at < maxAgeSec * 1000)
    return p;
  const fresh = await fetchPlayer(id, ctx);
  await ctx.kv.set(`player:${id}`, JSON.stringify(fresh), PLAYER_TTL);
  return fresh;
}
var cvhIdOf = (providers) => {
  for (const p of providers) {
    if (p.slug !== "cvh")
      continue;
    const id = parseCvh(p.url)?.id;
    if (id)
      return id;
  }
  return;
};
async function episodeId(animeId, n, pl, ctx) {
  if (pl.ids[n])
    return pl.ids[n];
  const html = await xhr(`/player/${encodeURIComponent(animeId)}/episodes?number=${n}`, ctx);
  for (const e of parseEpisodes(html, ctx))
    pl.ids[e.number] = e.id;
  return pl.ids[n];
}
async function episodeProviders(animeId, n, pl, ctx) {
  const id = await episodeId(animeId, n, pl, ctx);
  if (!id)
    return n === pl.current ? pl.providers : [];
  return parseProviders(await xhr(`/player/videos/${id}`, ctx), ctx);
}
function toProbe(providers) {
  const out = {};
  for (const p of providers) {
    const e = out[p.tr] ??= { name: p.name, slugs: [] };
    if (!e.slugs.includes(p.slug))
      e.slugs.push(p.slug);
    if (p.slug === "cvh")
      e.cvh = parseCvh(p.url)?.studio ?? e.cvh;
  }
  return out;
}
async function searchPage(name, q, ctx) {
  const res = await ctx.fetch(`${ctx.baseUrl}/search/anime?q=${encodeURIComponent(name)}`);
  if (!res.ok)
    throw new Error(`animego: search answered HTTP ${res.status}`);
  const html = await res.text();
  const links = ctx.html.select(html, ".ani-grid__item-title a");
  const bodies = ctx.html.select(html, ".ani-grid__item-body");
  const hits = [];
  links.forEach((a, i) => {
    const id = /-(\d+)\/?$/.exec(a.attrs.href ?? "")?.[1];
    if (!id)
      return;
    const title = (a.attrs.title ?? a.text).trim();
    const lines = (bodies.length === links.length ? bodies[i].text : "").split(`
`).map((s) => s.trim()).filter(Boolean);
    const year = Number(lines.find((l) => /^(19|20)\d\d$/.test(l)));
    const names = [title, ...lines.filter((l) => l.length > 2 && !/^(19|20)\d\d$/.test(l))];
    let score = Math.max(...names.flatMap((n) => q.names.map((qn) => similarity(n, qn))));
    if (q.year && year)
      score += year === q.year ? 0.1 : Math.abs(year - q.year) > 1 ? -0.3 : 0;
    hits.push({ id, title, names, year: year || undefined, score });
  });
  return hits;
}
async function verify(hit, q, ctx) {
  let cvh;
  try {
    cvh = cvhIdOf((await player(hit.id, ctx, PLAYER_REUSE)).providers);
  } catch {
    cvh = undefined;
  }
  const ids = [String(q.malId), String(q.shikimoriId)];
  if (cvh && !ids.includes(cvh))
    return null;
  const nameAndYear = hit.score >= 1 && q.year != null && hit.year === q.year;
  const data = cvh ? { cvh } : {};
  return { id: hit.id, title: hit.title, confidence: cvh || nameAndYear ? "exact" : "fuzzy", data };
}
var MIN_SCORE = 0.8;
async function search(q, ctx) {
  const hits = new Map;
  for (const name of q.names.slice(0, 3)) {
    for (const h of await searchPage(name, q, ctx)) {
      const seen = hits.get(h.id);
      if (!seen || seen.score < h.score)
        hits.set(h.id, h);
    }
    if ([...hits.values()].some((h) => h.score >= 1))
      break;
  }
  const top = [...hits.values()].filter((h) => h.score >= MIN_SCORE).sort((a, b) => b.score - a.score).slice(0, 3);
  const verified = await Promise.all(top.map((h) => verify(h, q, ctx)));
  return verified.filter((m) => m !== null).sort((a, b) => Number(b.confidence === "exact") - Number(a.confidence === "exact"));
}
function coverage(probes, tr) {
  const points = Object.keys(probes).map(Number).sort((a, b) => a - b);
  const runs = [];
  let start;
  let prev = 0;
  for (const n of points) {
    if (probes[n][tr]) {
      start ??= n;
      prev = n;
    } else if (start !== undefined) {
      runs.push([start, prev]);
      start = undefined;
    }
  }
  if (start !== undefined)
    runs.push([start, prev]);
  return runs;
}
function nextProbe(probes, skip) {
  const points = Object.keys(probes).map(Number).sort((a, b) => a - b);
  const trs = new Set(points.flatMap((n) => Object.keys(probes[n])));
  let best;
  for (let i = 1;i < points.length; i++) {
    const [a, b] = [points[i - 1], points[i]];
    if (b - a < 2 || best && b - a <= best[1] - best[0])
      continue;
    for (const tr of trs) {
      if (!skip.has(tr) && !probes[a][tr] !== !probes[b][tr]) {
        best = [a, b];
        break;
      }
    }
  }
  return best && Math.floor((best[0] + best[1]) / 2);
}

class EpisodeProber {
  m;
  pl;
  ctx;
  probes = {};
  started = Date.now();
  constructor(m, pl, ctx) {
    this.m = m;
    this.pl = pl;
    this.ctx = ctx;
  }
  async probe(n) {
    this.probes[n] = toProbe(await episodeProviders(this.m.id, n, this.pl, this.ctx));
  }
  async ends() {
    const cached = await this.ctx.kv.get(`probes:${this.m.id}`);
    if (cached)
      Object.assign(this.probes, JSON.parse(cached));
    this.probes[this.pl.current] = toProbe(this.pl.providers);
    const { numbers, current } = this.pl;
    const ends = new Set([numbers[0], numbers[numbers.length - 1]].filter((n) => n !== undefined && n !== current));
    await Promise.all([...ends].map((n) => this.probe(n)));
  }
  async refine(skip) {
    for (let i = 0;i < MAX_PROBES && Date.now() - this.started < PROBE_BUDGET_MS; i++) {
      const n = nextProbe(this.probes, skip);
      if (n === undefined || this.probes[n])
        break;
      await this.probe(n);
    }
    await this.ctx.kv.set(`probes:${this.m.id}`, JSON.stringify(this.probes), PROBES_TTL);
    await this.ctx.kv.set(`player:${this.m.id}`, JSON.stringify(this.pl), PLAYER_TTL);
    return this.probes;
  }
}
function groupCvh(items) {
  const out = new Map;
  for (const i of items) {
    const name = cvhStudio(i);
    const k = cvhKey(name);
    if (!k)
      continue;
    const d = out.get(k) ?? { name, subtitles: true, episodes: new Set };
    d.episodes.add(i.episode ?? 1);
    d.subtitles &&= /субтитр/i.test(i.voiceType ?? "");
    out.set(k, d);
  }
  return out;
}
function cvhFor(iframeStudio, name, cvh) {
  for (const wanted of [iframeStudio, name]) {
    const k = cvhKey(wanted);
    if (!k)
      continue;
    if (cvh.has(k))
      return k;
    if (wanted === iframeStudio) {
      for (const key of cvh.keys())
        if (key.includes(k) || k.includes(key))
          return key;
    }
  }
  return;
}
function parseDubId(dubId) {
  const bar = dubId.indexOf("|");
  if (bar < 0)
    return { tr: dubId };
  return { tr: dubId.slice(0, bar) || undefined, studio: dubId.slice(bar + 1) || undefined };
}
async function episodes(m, ctx) {
  const pl = await player(m.id, ctx, PLAYER_REUSE);
  const cvhId = cvhIdOf(pl.providers) ?? m.data?.cvh;
  const prober = new EpisodeProber(m, pl, ctx);
  const [items] = await Promise.all([
    cvhId ? cvhPlaylist(cvhId, ctx).catch(() => []) : Promise.resolve([]),
    prober.ends()
  ]);
  const cvh = groupCvh(items);
  const mapped = new Set;
  for (const probe of Object.values(prober.probes)) {
    for (const [tr, e] of Object.entries(probe))
      if (cvhFor(e.cvh, pl.names[tr] ?? e.name, cvh))
        mapped.add(tr);
  }
  const probes = await prober.refine(mapped);
  const trs = [];
  const names = { ...pl.names };
  const iframeStudio = {};
  const aniboom = new Set;
  for (const n of Object.keys(probes).map(Number).sort((a, b) => a - b)) {
    for (const [tr, e] of Object.entries(probes[n])) {
      if (!trs.includes(tr))
        trs.push(tr);
      names[tr] ??= e.name;
      if (e.cvh)
        iframeStudio[tr] ??= e.cvh;
      if (e.slugs.includes("aniboom"))
        aniboom.add(tr);
    }
  }
  const order = Object.keys(pl.names);
  trs.sort((a, b) => (order.indexOf(a) + 1 || 1e9) - (order.indexOf(b) + 1 || 1e9));
  const dubs = [];
  const avail = new Map;
  const used = new Set;
  for (const tr of trs) {
    const name = names[tr] || `Озвучка ${tr}`;
    const key = cvhFor(iframeStudio[tr], name, cvh);
    const c = key ? cvh.get(key) : undefined;
    if (key)
      used.add(key);
    const id = c ? `${tr}|${c.name}` : tr;
    const subtitles = /субтитр/i.test(name) || (c?.subtitles ?? false);
    dubs.push({ id, name, type: subtitles ? "subtitles" : "voice" });
    const runs = coverage(probes, tr);
    const own = aniboom.has(tr) ? HD : SD;
    avail.set(id, (n) => c?.episodes.has(n) ? HD : runs.some(([a, b]) => n >= a && n <= b) ? own : undefined);
  }
  for (const [key, c] of cvh) {
    if (used.has(key))
      continue;
    const id = `|${c.name}`;
    dubs.push({ id, name: c.name, type: c.subtitles ? "subtitles" : "voice" });
    avail.set(id, (n) => c.episodes.has(n) ? HD : undefined);
  }
  const numbers = new Set(pl.numbers);
  for (const c of cvh.values())
    for (const n of c.episodes)
      numbers.add(n);
  return {
    dubs,
    episodes: [...numbers].sort((a, b) => a - b).map((number) => ({
      number,
      available: dubs.flatMap((d) => {
        const q = avail.get(d.id)?.(number);
        return q ? [{ dubId: d.id, maxQuality: q }] : [];
      })
    }))
  };
}
function attempts(providers, studio, cvhId, episode, ctx) {
  const out = [];
  const sorted = [...providers].sort((a, b) => (PLAYER_ORDER.indexOf(a.slug) + 1 || 99) - (PLAYER_ORDER.indexOf(b.slug) + 1 || 99));
  let cvhAdded = false;
  const addCvh = (fromIframe) => {
    if (cvhAdded)
      return;
    const parsed = fromIframe ? parseCvh(fromIframe) : null;
    const id = parsed?.id ?? cvhId;
    const dub = studio ?? parsed?.studio;
    if (!id || !dub)
      return;
    cvhAdded = true;
    const url = cvhPlaylistUrl({ id, studio: dub, episode, season: parsed?.season });
    out.push({ player: "cvh", run: () => resolveCvh(url, ctx) });
  };
  for (const p of sorted) {
    if (p.slug === "aniboom")
      out.push({ player: "aniboom", run: () => resolveAniboom(p.url, ctx, `${ctx.baseUrl}/`) });
    else if (p.slug === "cvh")
      addCvh(p.url);
    else if (p.slug === "kodik") {
      addCvh();
      out.push({ player: "kodik", run: () => resolveKodik(p.url, ctx) });
    }
  }
  addCvh();
  return out;
}
async function resolve(m, episode, dubId, ctx) {
  const { tr, studio } = parseDubId(dubId);
  const pl = await player(m.id, ctx, PLAYER_TTL);
  const cvhId = cvhIdOf(pl.providers) ?? m.data?.cvh;
  const providers = tr ? (await episodeProviders(m.id, episode, pl, ctx)).filter((p) => p.tr === tr) : [];
  const tries = attempts(providers, studio, cvhId, episode, ctx);
  if (tries.length === 0)
    ctx.fail.notFound(`AnimeGO has no episode ${episode} in this dub`);
  const failures = [];
  let challenge;
  let allNotFound = true;
  for (const t of tries) {
    try {
      const streams = await t.run();
      if (streams.length > 0)
        return streams;
      failures.push(`${t.player}: no streams`);
      allNotFound = false;
    } catch (e) {
      const err = e instanceof Error ? e : new Error(String(e));
      ctx.log(`animego: ${t.player} failed: ${err.message}`);
      failures.push(`${t.player}: ${err.message.replace(/__EXT:(\w+):/g, "$1: ")}`);
      if (err.message.includes("__EXT:Challenge:"))
        challenge ??= err;
      if (!err.message.includes("__EXT:NotFound:"))
        allNotFound = false;
    }
  }
  if (challenge)
    throw challenge;
  const summary = failures.join("; ");
  if (allNotFound)
    ctx.fail.notFound(summary);
  throw new Error(`animego: every player failed (${summary})`);
}
export {
  episodes,
  manifest,
  resolve,
  search
};
