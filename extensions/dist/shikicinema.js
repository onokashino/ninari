// src/players/okru.ts
var OKRU_HOSTS = ["ok.ru", "*.ok.ru", "*.okcdn.ru"];
var OKCDN_UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
var OK_QUALITIES = {
  mobile: 144,
  lowest: 240,
  low: 360,
  sd: 480,
  hd: 720,
  full: 1080,
  quad: 1440,
  ultra: 2160
};
var PAGE_TIMEOUT_MS = 8000;
var EMBED_RE = /^(?:https?:)?\/\/(?:www\.|m\.)?(?:ok\.ru|odnoklassniki\.ru)\/(?:videoembed|video)\/(\d+)/i;
var OPTIONS_RE = /data-options="([^"]*)"/;
var STUB_RE = /vp_video_stub_txt[^>]*>([^<]*)</;
var ENTITIES = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
var unescapeHtml = (s) => s.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (m, e) => e[0] === "#" ? String.fromCodePoint(e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : Number(e.slice(1))) : ENTITIES[e.toLowerCase()] ?? m);
function isOkru(url) {
  return EMBED_RE.test(url.trim());
}
function okruId(url) {
  return EMBED_RE.exec(url.trim())?.[1];
}
function parseOkruPage(html) {
  const raw = OPTIONS_RE.exec(html)?.[1];
  if (raw) {
    try {
      const options = JSON.parse(unescapeHtml(raw));
      const md = options.flashvars?.metadata;
      const metadata = typeof md === "string" ? JSON.parse(md) : md;
      if (metadata)
        return { metadata };
    } catch {}
  }
  const stub = STUB_RE.exec(html)?.[1]?.trim();
  return stub ? { stub } : null;
}
function okruStreams(md, headers) {
  const mp4 = [];
  for (const v of md.videos ?? []) {
    const quality = OK_QUALITIES[v.name ?? ""];
    if (!v.url || v.disallowed || !quality || mp4.some((s) => s.quality === quality))
      continue;
    mp4.push({ url: v.url, kind: "mp4", quality, headers });
  }
  mp4.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
  const hls = md.hlsManifestUrl || md.ondemandHls;
  return hls ? [{ url: hls, kind: "hls", quality: mp4[0]?.quality, headers }, ...mp4] : mp4;
}
async function resolveOkru(embedUrl, ctx) {
  const id = okruId(embedUrl);
  if (!id)
    throw new Error(`okru: no video id in ${embedUrl}`);
  const page = `https://ok.ru/videoembed/${id}`;
  const init = { headers: { "User-Agent": OKCDN_UA }, timeout: PAGE_TIMEOUT_MS };
  const res = await ctx.fetch(page, init);
  if (res.status === 404)
    ctx.fail.notFound(`okru: video ${id} is gone`);
  if (!res.ok)
    throw new Error(`okru: HTTP ${res.status} for ${page}`);
  const parsed = parseOkruPage(await res.text());
  if (!parsed)
    throw new Error(`okru: no player data on ${page}`);
  if ("stub" in parsed)
    ctx.fail.notFound(`okru: ${parsed.stub}`);
  const streams = okruStreams(parsed.metadata, { "User-Agent": OKCDN_UA, Referer: "https://ok.ru/" });
  if (streams.length === 0)
    ctx.fail.notFound(`okru: video ${id} has no playable sources`);
  return streams;
}

// src/players/dzen.ts
var DZEN_HOSTS = ["dzen.ru", "*.dzen.ru", "*.okcdn.ru"];
var TYPES = {
  tiny: 144,
  lowest: 240,
  low: 360,
  medium: 480,
  high: 720,
  fullhd: 1080,
  quadhd: 1440,
  ultrahd: 2160
};
var PAGE_TIMEOUT_MS2 = 8000;
var EMBED_RE2 = /^(?:https?:)?\/\/(?:www\.)?(?:dzen\.ru|zen\.yandex\.ru)\/(?:embed|video\/watch)\/([0-9a-z_-]{6,})/i;
var STREAMS_RE = /"streams"\s*:\s*(\[[^\]]*\])/;
function isDzen(url) {
  return EMBED_RE2.test(url.trim());
}
function dzenId(url) {
  return EMBED_RE2.exec(url.trim())?.[1];
}
function parseDzen(html, headers) {
  const raw = STREAMS_RE.exec(html)?.[1];
  if (!raw)
    return [];
  let list;
  try {
    list = JSON.parse(raw);
  } catch {
    return [];
  }
  const mp4 = [];
  let hls;
  for (const s of list) {
    if (!s.url)
      continue;
    if (s.type === "hls")
      hls ??= s.url;
    const quality = TYPES[s.type ?? ""];
    if (quality && !mp4.some((m) => m.quality === quality))
      mp4.push({ url: s.url, kind: "mp4", quality, headers });
  }
  mp4.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
  return hls ? [{ url: hls, kind: "hls", quality: mp4[0]?.quality, headers }, ...mp4] : mp4;
}
async function resolveDzen(embedUrl, ctx) {
  const id = dzenId(embedUrl);
  if (!id)
    throw new Error(`dzen: no video id in ${embedUrl}`);
  const page = `https://dzen.ru/embed/${id}`;
  const init = { headers: { "User-Agent": OKCDN_UA }, timeout: PAGE_TIMEOUT_MS2 };
  const res = await ctx.fetch(page, init);
  if (res.status === 404)
    ctx.fail.notFound(`dzen: video ${id} is gone`);
  if (!res.ok)
    throw new Error(`dzen: HTTP ${res.status} for ${page}`);
  const streams = parseDzen(await res.text(), { "User-Agent": OKCDN_UA, Referer: "https://dzen.ru/" });
  if (streams.length === 0)
    ctx.fail.notFound(`dzen: video ${id} has no streams`);
  return streams;
}

// src/players/kinogram.ts
var KINOGRAM_HOSTS = ["*.kinogram.best", "*.interkh.com"];
var PAGE_TIMEOUT_MS3 = 8000;
var MASTER_TIMEOUT_MS = 7000;
var AUDIO_TTL = 6 * 3600;
var EMBED_RE3 = /^(?:https?:)?\/\/((?:api\d*\.)?kinogram\.best)\/embed\/([a-z]+)\/(\d+)/i;
function isKinogram(url) {
  return EMBED_RE3.test(url.trim());
}
function parseKinogramUrl(url) {
  const u = url.trim().replace(/&amp;/g, "&");
  const m = EMBED_RE3.exec(u);
  if (!m)
    return;
  const num = (name) => {
    const v = new RegExp(`[?&]${name}=(\\d+)`).exec(u)?.[1];
    return v === undefined ? undefined : Number(v);
  };
  return { page: `https://${m[1].toLowerCase()}/embed/${m[2].toLowerCase()}/${m[3]}`, season: num("season"), episode: num("episode") };
}
function sliceJson(text, from) {
  const open = text[from];
  if (open !== "[" && open !== "{")
    return;
  let depth = 0;
  let inString = false;
  for (let i = from;i < text.length; i++) {
    const c = text[i];
    if (inString) {
      if (c === "\\")
        i++;
      else if (c === '"')
        inString = false;
    } else if (c === '"')
      inString = true;
    else if (c === "[" || c === "{")
      depth++;
    else if (c === "]" || c === "}") {
      depth--;
      if (depth === 0)
        return text.slice(from, i + 1);
    }
  }
  return;
}
function jsonAfter(text, re) {
  const m = re.exec(text);
  if (!m)
    return;
  const raw = sliceJson(text, m.index + m[0].length);
  if (!raw)
    return;
  try {
    return JSON.parse(raw);
  } catch {
    return;
  }
}
function audioNames(a) {
  return (a?.names ?? []).filter((n) => typeof n === "string" && n.trim() !== "");
}
function parseKinogram(html) {
  const seasons = jsonAfter(html, /\bseasons\s*:\s*/);
  if (seasons) {
    const episodes = {};
    for (const s of seasons) {
      for (const e of s.episodes ?? []) {
        if (!e.hls || s.season === undefined || e.episode === undefined)
          continue;
        episodes[`${s.season}:${Number(e.episode)}`] = { hls: e.hls, audio: audioNames(e.audio), blocked: s.blocked || undefined };
      }
    }
    return { episodes };
  }
  const hls = /\bhls\s*:\s*"([^"]+)"/.exec(html)?.[1];
  if (!hls)
    return null;
  return { episodes: {}, movie: { hls, audio: audioNames(jsonAfter(html, /\baudio\s*:\s*/)) } };
}
function pickKinogram(page, ref) {
  if (ref.episode === undefined)
    return page.movie;
  return page.episodes[`${ref.season ?? 1}:${ref.episode}`];
}
var TIERS = [240, 360, 480, 720, 1080, 1440, 2160];
function masterHeight(m3u8) {
  let best = 0;
  for (const m of m3u8.matchAll(/RESOLUTION=\d+x(\d+)/g))
    best = Math.max(best, Number(m[1]));
  return best > 0 ? TIERS.find((t) => t >= best) ?? best : undefined;
}
function tracksOf(parsed) {
  const out = {};
  for (const [k, v] of Object.entries(parsed.episodes))
    out[k] = v.audio;
  if (parsed.movie)
    out.movie = parsed.movie.audio;
  return out;
}
async function fetchPage(page, ctx) {
  const init = { timeout: PAGE_TIMEOUT_MS3 };
  const res = await ctx.fetch(page, init);
  if (res.status === 404)
    ctx.fail.notFound(`kinogram: ${page} is gone`);
  if (!res.ok)
    throw new Error(`kinogram: HTTP ${res.status} for ${page}`);
  const parsed = parseKinogram(await res.text());
  if (!parsed)
    throw new Error(`kinogram: no playlist on ${page}`);
  await ctx.kv.set(`kinogram:audio:${page}`, JSON.stringify(tracksOf(parsed)), AUDIO_TTL);
  return parsed;
}
var kinogramKey = (ref) => ref.episode === undefined ? "movie" : `${ref.season ?? 1}:${ref.episode}`;
async function kinogramTracks(page, ctx) {
  const cached = await ctx.kv.get(`kinogram:audio:${page}`);
  if (cached) {
    try {
      return JSON.parse(cached);
    } catch {}
  }
  return tracksOf(await fetchPage(page, ctx));
}
async function resolveKinogram(embedUrl, ctx) {
  const ref = parseKinogramUrl(embedUrl);
  if (!ref)
    throw new Error(`kinogram: unsupported url ${embedUrl}`);
  const video = pickKinogram(await fetchPage(ref.page, ctx), ref);
  if (!video)
    ctx.fail.notFound(`kinogram: no season ${ref.season ?? 1} episode ${ref.episode} on ${ref.page}`);
  if (video.blocked)
    ctx.fail.notFound(`kinogram: the season is blocked on ${ref.page}`);
  const init = { timeout: MASTER_TIMEOUT_MS };
  let res;
  try {
    res = await ctx.fetch(video.hls, init);
  } catch (e) {
    ctx.log(`kinogram: master playlist did not answer: ${String(e)}`);
    return [{ url: video.hls, kind: "hls" }];
  }
  if (res.status === 404 || res.status === 410)
    ctx.fail.notFound(`kinogram: master playlist is gone`);
  if (!res.ok)
    throw new Error(`kinogram: master playlist answered HTTP ${res.status}`);
  const body = await res.text();
  if (!body.startsWith("#EXTM3U"))
    throw new Error("kinogram: master playlist is not HLS");
  return [{ url: video.hls, kind: "hls", quality: masterHeight(body) }];
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

// src/players/matreshka.ts
var MATRESHKA_HOSTS = ["matreshka.tv", "*.matreshka.tv", "*.cmtv.ru"];
var PAGE_TIMEOUT_MS4 = 8000;
var EMBED_RE4 = /^(?:https?:)?\/\/(?:www\.)?matreshka\.tv\/(?:embed\/)?video\/([0-9a-z_-]{6,})/i;
var MASTER_RE = /https:\/\/[^"'\s\\]+?\/master\.m3u8[^"'\s\\]*/g;
var SPEC_RE = /\/(eyJ[0-9A-Za-z+/=_-]+)\//;
function isMatreshka(url) {
  return EMBED_RE4.test(url.trim());
}
function matreshkaId(url) {
  return EMBED_RE4.exec(url.trim())?.[1];
}
function parseMatreshka(html, b64decode, videoId) {
  const text = html.replace(/\\u002F/gi, "/");
  let adaptive;
  const fixed = new Map;
  for (const url of new Set(text.match(MASTER_RE) ?? [])) {
    const seg = SPEC_RE.exec(url)?.[1];
    if (!seg)
      continue;
    let spec;
    try {
      spec = JSON.parse(b64decode(seg));
    } catch {
      continue;
    }
    if (videoId && spec.id && spec.id !== videoId || spec.codec && spec.codec !== "h264")
      continue;
    const q = spec.userQuality ?? [];
    if (q.length === 0)
      adaptive ??= url;
    else if (q.length === 1 && q[0] > 0 && !fixed.has(q[0]))
      fixed.set(q[0], url);
  }
  const heights = [...fixed.keys()].sort((a, b) => b - a);
  const out = heights.map((h) => ({ url: fixed.get(h), kind: "hls", quality: h }));
  if (adaptive)
    out.unshift({ url: adaptive, kind: "hls", quality: heights[0] });
  return out;
}
async function resolveMatreshka(embedUrl, ctx) {
  const id = matreshkaId(embedUrl);
  if (!id)
    throw new Error(`matreshka: no video id in ${embedUrl}`);
  const page = `https://matreshka.tv/embed/video/${id}`;
  const init = { timeout: PAGE_TIMEOUT_MS4 };
  const res = await ctx.fetch(page, init);
  if (res.status === 404)
    ctx.fail.notFound(`matreshka: video ${id} is gone`);
  if (!res.ok)
    throw new Error(`matreshka: HTTP ${res.status} for ${page}`);
  const streams = parseMatreshka(await res.text(), (s) => ctx.base64.decode(s), id);
  if (streams.length === 0)
    ctx.fail.notFound(`matreshka: video ${id} has no streams`);
  return streams;
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

// src/players/vk.ts
var VK_HOSTS = [
  "vk.com",
  "*.vk.com",
  "vk.ru",
  "*.vk.ru",
  "vkvideo.ru",
  "*.vkvideo.ru",
  "*.okcdn.ru",
  "*.vkuser.net",
  "*.userapi.com"
];
var PAGE_TIMEOUT_MS5 = 8000;
var EMBED_RE5 = /^(?:https?:)?\/\/(?:www\.|m\.)?(?:vk\.com|vk\.ru|vkvideo\.ru)\/video_ext\.php\?/i;
var FILES_RE = /"files"\s*:\s*(\{[^{}]*\})/;
var OLD_KEY_RE = /"(url\d{3,4}|hls)"\s*:\s*"((?:[^"\\]|\\.)+)"/g;
var MESSAGE_RE = /id="video_ext_msg"[^>]*>([^<]*)</;
function isVk(url) {
  const u = url.trim();
  return EMBED_RE5.test(u) && /[?&]oid=-?\d+/.test(u) && /[?&]id=\d+/.test(u);
}
function vkEmbedUrl(url) {
  const u = url.trim().replace(/&amp;/g, "&");
  const oid = /[?&]oid=(-?\d+)/.exec(u)?.[1];
  const id = /[?&]id=(\d+)/.exec(u)?.[1];
  if (!EMBED_RE5.test(u) || !oid || !id)
    return;
  const hash = /[?&]hash=([0-9a-f]+)/i.exec(u)?.[1];
  return `https://vk.com/video_ext.php?oid=${oid}&id=${id}${hash ? `&hash=${hash}` : ""}`;
}
var heightOf = (key) => {
  const m = /^(?:mp4_|url)(\d{3,4})$/.exec(key);
  return m ? Number(m[1]) : undefined;
};
function toStreams2(files, headers) {
  const mp4 = [];
  for (const [key, url] of Object.entries(files)) {
    const quality = heightOf(key);
    if (quality && typeof url === "string" && url && !mp4.some((s) => s.quality === quality)) {
      mp4.push({ url, kind: "mp4", quality, headers });
    }
  }
  mp4.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
  const hls = typeof files.hls === "string" ? files.hls : typeof files.hls_ondemand === "string" ? files.hls_ondemand : "";
  return hls ? [{ url: hls, kind: "hls", quality: mp4[0]?.quality, headers }, ...mp4] : mp4;
}
function parseVk(html, headers) {
  const raw = FILES_RE.exec(html)?.[1];
  if (raw) {
    try {
      return toStreams2(JSON.parse(raw), headers);
    } catch {}
  }
  const old = {};
  for (const m of html.matchAll(OLD_KEY_RE)) {
    try {
      old[m[1]] ??= JSON.parse(`"${m[2]}"`);
    } catch {}
  }
  return toStreams2(old, headers);
}
function vkMessage(html) {
  return MESSAGE_RE.exec(html)?.[1]?.trim() || undefined;
}
async function resolveVk(embedUrl, ctx) {
  const page = vkEmbedUrl(embedUrl);
  if (!page)
    throw new Error(`vk: not a video_ext url ${embedUrl}`);
  const init = { headers: { "User-Agent": OKCDN_UA }, timeout: PAGE_TIMEOUT_MS5 };
  for (let attempt = 0;attempt < 2; attempt++) {
    const res = await ctx.fetch(page, init);
    if (res.status === 404)
      ctx.fail.notFound(`vk: ${page} is gone`);
    if (!res.ok)
      throw new Error(`vk: HTTP ${res.status} for ${page}`);
    const html = await res.text();
    const streams = parseVk(html, { "User-Agent": OKCDN_UA, Referer: "https://vk.com/" });
    if (streams.length > 0)
      return streams;
    const msg = vkMessage(html);
    if (msg)
      ctx.fail.notFound(`vk: ${msg}`);
  }
  throw new Error(`vk: no player data on ${page}`);
}

// src/shikicinema.ts
var manifest = {
  id: "shikicinema",
  name: "Shikicinema",
  version: "0.1.0",
  apiVersion: 1,
  domains: ["https://smarthard.net"],
  allowedHosts: [
    ...new Set([...SIBNET_HOSTS, ...OKRU_HOSTS, ...DZEN_HOSTS, ...VK_HOSTS, ...MATRESHKA_HOSTS, ...KINOGRAM_HOSTS, ...KODIK_HOSTS])
  ]
};
var API = "/api/shikivideos";
var HEADERS = { Accept: "application/json" };
var FULL_LIST_MAX = 64;
var PROBES_PER_CALL = 16;
var PROBE_MAX_AGE = 7 * 24 * 3600;
var TAIL_MAX_AGE = 3600;
var LENGTH_TTL = 600;
var DEAD_TTL = 7 * 24 * 3600;
var RESOLVE_BUDGET_MS = 7000;
var PLAYERS = [
  { id: "dzen", matches: isDzen, resolve: resolveDzen },
  { id: "matreshka", matches: isMatreshka, resolve: resolveMatreshka },
  { id: "kinogram", matches: isKinogram, resolve: resolveKinogram },
  { id: "kodik", matches: isKodik, resolve: (url, ctx) => resolveKodik(url, ctx) },
  { id: "sibnet", matches: isSibnet, resolve: resolveSibnet },
  { id: "vk", matches: isVk, resolve: resolveVk },
  { id: "okru", matches: isOkru, resolve: resolveOkru }
];
var KINDS = { "озвучка": "voice", "субтитры": "subtitles", "оригинал": "original" };
var TYPE_CODE = { voice: "v", subtitles: "s", original: "o" };
var GENERIC_RE = /^(?:\d+|часть\d*|(?:рус|русск(?:ий|ая|ие)|укр|англ|eng|проф|профессиональн(?:ый|ая)|любительск(?:ий|ая)|многоголосый|одноголосый|двухголосый|закадровый|озвучка|субтитры|оригинал|original|sub|dub)+)$/;
var norm = (s) => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");
var nowSec = () => Math.floor(Date.now() / 1000);
function teamOf(author) {
  let a = (author ?? "").replace(/\s+/g, " ").trim();
  for (;; ) {
    const cast = /^(.*?\S)\s*\([^()]*\)$/.exec(a);
    if (!cast || (cast[1].match(/[\p{L}\p{N}]/gu) ?? []).length < 2)
      break;
    a = cast[1].trim();
  }
  return a && !GENERIC_RE.test(norm(a)) ? a : undefined;
}
function dubOf(kind, author) {
  const type = KINDS[(kind ?? "").trim().toLowerCase()];
  if (!type)
    return;
  const team = type === "original" ? "Оригинал" : teamOf(author) ?? (type === "subtitles" ? "Субтитры" : undefined);
  if (!team)
    return;
  return { id: `${TYPE_CODE[type]}|${norm(team)}`, name: team, type };
}
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
  if (!res.ok)
    throw new Error(`shikicinema: HTTP ${res.status} for ${path}`);
  return await res.json();
}
function animeId(m) {
  const id = Number(m.id);
  if (!Number.isInteger(id) || id <= 0)
    throw new Error(`shikicinema: bad match id ${m.id}`);
  return id;
}
async function episodeCount(id, ctx) {
  const key = `length:${id}`;
  const cached = await ctx.kv.get(key);
  if (cached !== null)
    return Number(cached) || 0;
  const { length } = await api(`${API}/${id}/length`, ctx);
  const n = Number(length) || 0;
  await ctx.kv.set(key, String(n), LENGTH_TTL);
  return n;
}
async function loadDead(id, ctx) {
  const raw = await ctx.kv.get(`dead:${id}`);
  if (!raw)
    return {};
  try {
    const all = JSON.parse(raw);
    const now = nowSec();
    return Object.fromEntries(Object.entries(all).filter(([, until]) => until > now));
  } catch {
    return {};
  }
}
async function toEntries(videos, dead, ctx, dubs, tracks = new Map) {
  const out = [];
  for (const v of videos) {
    const url = (v.url ?? "").trim();
    const n = Number(v.episode);
    if (!url || !Number.isInteger(n) || n < 1 || dead[String(v.id)])
      continue;
    if (/^(english|en)$/i.test(v.language ?? ""))
      continue;
    const player = playerOf(url);
    if (!player)
      continue;
    let dub = dubOf(v.kind, v.author);
    if (player.id === "kinogram") {
      const ref = parseKinogramUrl(url);
      if (!ref)
        continue;
      if (!tracks.has(ref.page))
        tracks.set(ref.page, kinogramTracks(ref.page, ctx).catch(() => ({})));
      const first = (await tracks.get(ref.page))[kinogramKey(ref)]?.[0];
      if (!first)
        continue;
      dub = /original|оригинал/i.test(first) ? dubOf("оригинал", null) : dubOf("озвучка", first);
    }
    if (!dub)
      continue;
    dubs[dub.id] ??= { name: dub.name, type: dub.type };
    out.push({ id: v.id, n, dub: dub.id, url, player: player.id, watches: v.watches_count ?? 0 });
  }
  return out;
}
async function search(q, ctx) {
  if (!q.shikimoriId)
    return [];
  if (await episodeCount(q.shikimoriId, ctx) === 0)
    return [];
  return [{ id: String(q.shikimoriId), title: q.names[0] ?? `Shikimori ${q.shikimoriId}`, confidence: "exact" }];
}
function toList(dubs, byEpisode) {
  const used = new Set;
  const episodes = [...byEpisode.entries()].filter(([, ids]) => ids.size > 0).sort(([a], [b]) => a - b).map(([number, ids]) => {
    for (const d of ids)
      used.add(d);
    return { number, available: [...ids].map((dubId) => ({ dubId })) };
  });
  return { dubs: [...used].map((id) => ({ id, ...dubs[id] })), episodes };
}
async function listAll(id, ctx) {
  const dubs = {};
  const [videos, dead] = await Promise.all([api(`${API}/${id}?limit=all`, ctx), loadDead(id, ctx)]);
  const byEpisode = new Map;
  for (const e of await toEntries(videos ?? [], dead, ctx, dubs)) {
    const set = byEpisode.get(e.n) ?? new Set;
    set.add(e.dub);
    byEpisode.set(e.n, set);
  }
  return toList(dubs, byEpisode);
}
async function loadProbes(id, ctx) {
  const raw = await ctx.kv.get(`probes:${id}`);
  const probes = { dubs: {}, eps: {} };
  if (!raw)
    return probes;
  try {
    const cached = JSON.parse(raw);
    const oldest = nowSec() - PROBE_MAX_AGE;
    probes.dubs = cached.dubs ?? {};
    for (const [n, p] of Object.entries(cached.eps ?? {}))
      if (p[0] >= oldest)
        probes.eps[n] = p;
  } catch {}
  return probes;
}
var saveProbes = (id, p, ctx) => ctx.kv.set(`probes:${id}`, JSON.stringify(p), PROBE_MAX_AGE);
function record(p, n, entries, dubs) {
  const ids = [...new Set(entries.filter((e) => e.n === n).map((e) => e.dub))];
  for (const d of ids)
    p.dubs[d] ??= dubs[d];
  p.eps[String(n)] = [nowSec(), ids];
}
var SAME_ENDS_WEIGHT = 0.25;
function nextProbes(eps, length, count, near, staleTail = []) {
  const known = new Set(Object.keys(eps).map(Number));
  const picks = [];
  const take = (n) => {
    if (picks.length < count && n >= 1 && n <= length && !known.has(n)) {
      known.add(n);
      picks.push(n);
    }
  };
  if (near)
    [near + 1, near + 2].forEach(take);
  for (const n of staleTail) {
    if (picks.length < count && n >= 1 && n <= length && !picks.includes(n)) {
      known.add(n);
      picks.push(n);
    }
  }
  take(1);
  take(length);
  const sameEnds = (a, b) => {
    const x = eps[String(a)]?.[1];
    const y = eps[String(b)]?.[1];
    return !!x && !!y && x.length === y.length && x.every((d) => y.includes(d));
  };
  const points = [...known].sort((a, b) => a - b);
  const gaps = [];
  for (let i = 1;i < points.length; i++) {
    const [a, b] = [points[i - 1], points[i]];
    if (b - a > 1)
      gaps.push({ a, b, w: sameEnds(a, b) ? SAME_ENDS_WEIGHT : 1 });
  }
  while (picks.length < count && gaps.length > 0) {
    let best = 0;
    for (let i = 1;i < gaps.length; i++)
      if ((gaps[i].b - gaps[i].a) * gaps[i].w > (gaps[best].b - gaps[best].a) * gaps[best].w)
        best = i;
    const { a, b, w } = gaps.splice(best, 1)[0];
    const mid = Math.floor((a + b) / 2);
    take(mid);
    if (mid - a > 1)
      gaps.push({ a, b: mid, w });
    if (b - mid > 1)
      gaps.push({ a: mid, b, w });
  }
  return picks;
}
function coverage(eps, length) {
  const points = Object.keys(eps).map(Number).filter((n) => n <= length).sort((a, b) => a - b);
  const out = new Map;
  const add = (n, d) => {
    const set = out.get(n) ?? new Set;
    set.add(d);
    out.set(n, set);
  };
  points.forEach((n, i) => {
    const here = eps[String(n)][1];
    for (const d of here)
      add(n, d);
    if (i === 0)
      return;
    const prev = points[i - 1];
    const shared = eps[String(prev)][1].filter((d) => here.includes(d));
    for (let k = prev + 1;k < n; k++)
      for (const d of shared)
        add(k, d);
  });
  return out;
}
async function listSampled(id, length, ctx) {
  const [probes, dead, nearRaw] = await Promise.all([loadProbes(id, ctx), loadDead(id, ctx), ctx.kv.get(`near:${id}`)]);
  const staleBefore = nowSec() - TAIL_MAX_AGE;
  const staleTail = [length, length - 1].filter((n) => n >= 1 && (probes.eps[String(n)]?.[0] ?? 0) < staleBefore);
  const picks = nextProbes(probes.eps, length, PROBES_PER_CALL, Number(nearRaw) || undefined, staleTail);
  const dubs = { ...probes.dubs };
  const tracks = new Map;
  await Promise.all(picks.map(async (n) => {
    try {
      const videos = await api(`${API}/${id}?episode=${n}`, ctx);
      record(probes, n, await toEntries(videos ?? [], dead, ctx, dubs, tracks), dubs);
    } catch (e) {
      ctx.log(`shikicinema: episode ${n} of ${id} not sampled: ${String(e)}`);
    }
  }));
  await saveProbes(id, probes, ctx);
  return toList(probes.dubs, coverage(probes.eps, length));
}
async function episodes(m, ctx) {
  const id = animeId(m);
  const length = await episodeCount(id, ctx);
  if (length === 0)
    return { dubs: [], episodes: [] };
  return length <= FULL_LIST_MAX ? listAll(id, ctx) : listSampled(id, length, ctx);
}
var rank = (e) => PLAYERS.findIndex((p) => p.id === e.player);
var byPreference = (a, b) => rank(a) - rank(b) || b.watches - a.watches || b.id - a.id;
async function resolve(m, episode, dubId, ctx) {
  const started = Date.now();
  const id = animeId(m);
  const dead = await loadDead(id, ctx);
  const dubs = {};
  const videos = await api(`${API}/${id}?episode=${episode}`, ctx);
  const entries = await toEntries(videos ?? [], dead, ctx, dubs);
  const tries = entries.filter((e) => e.n === episode && e.dub === dubId).sort(byPreference).filter((e, i, all) => all.findIndex((x) => x.url === e.url) === i);
  const failures = [];
  let challenge;
  let streams;
  for (const [i, e] of tries.entries()) {
    if (Date.now() - started > RESOLVE_BUDGET_MS) {
      failures.push(`${tries.length - i} more not tried in time`);
      break;
    }
    const player = PLAYERS.find((p) => p.id === e.player);
    try {
      const out = await player.resolve(e.url, ctx);
      if (out.length > 0) {
        streams = out;
        break;
      }
      failures.push(`${player.id}: no streams`);
    } catch (err) {
      const msg = String(err instanceof Error ? err.message : err);
      ctx.log(`shikicinema: ${player.id} failed for ${e.url}: ${msg}`);
      failures.push(`${player.id}: ${msg.replace(/__EXT:(\w+):/g, "$1: ")}`);
      if (msg.includes("__EXT:NotFound:"))
        dead[String(e.id)] = nowSec() + DEAD_TTL;
      if (msg.includes("__EXT:Challenge:"))
        challenge ??= err;
    }
  }
  await ctx.kv.set(`dead:${id}`, JSON.stringify(dead), DEAD_TTL);
  await ctx.kv.set(`near:${id}`, String(episode), PROBE_MAX_AGE);
  const probes = await ctx.kv.get(`probes:${id}`);
  if (probes) {
    const p = await loadProbes(id, ctx);
    record(p, episode, entries.filter((e) => !dead[String(e.id)]), dubs);
    await saveProbes(id, p, ctx);
  }
  if (streams)
    return streams;
  if (challenge)
    throw challenge;
  return ctx.fail.notFound(tries.length === 0 ? `Shikicinema has no episode ${episode} in this dub` : failures.join("; "));
}
export {
  coverage,
  dubOf,
  episodes,
  manifest,
  nextProbes,
  resolve,
  search,
  teamOf
};
