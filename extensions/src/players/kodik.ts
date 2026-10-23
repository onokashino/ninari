// Kodik player: turns an embed URL into HLS streams. Shared by the catalog extensions.
// Extraction flow ported from ialakey/anime-dl-core and YaNesyTortiK/AnimeParsers (both MIT).
import type { Ctx, Stream } from "../types";

/** Where Kodik serves embeds today. */
const MAIN_HOST = "kodikplayer.com";
/** Old embed domains that catalogs still hand out. They no longer resolve (NXDOMAIN), same paths work on MAIN_HOST. */
const LEGACY_HOSTS = ["kodik.info", "kodik.biz", "kodik.cc", "aniqit.com"];

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const KODIK_HOSTS: string[] = [
  `*.${MAIN_HOST}`,
  ...LEGACY_HOSTS.map((h) => `*.${h}`),
  // CDN: links point at cloud.solodcdn.com, which redirects to a node like calfium.cloud.solodcdn.com.
  // The other two carried the links before 2025 and still show up.
  "*.solodcdn.com",
  "*.kodik-storage.com",
  "*.kodik-cdn.com",
];

const UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const DEFAULT_POST_PATH = "/ftor";
const MANIFEST_SUFFIX = ":hls:manifest.m3u8";
const SCRIPT_TTL = 12 * 3600;
const SHIFT_KEY = "kodik:shift";
/** Shifts Kodik has used, most recent first. Anything else is found by brute force. */
const KNOWN_SHIFTS = [18, 13];

const HOST_RE = /^(?:https?:)?\/\/([^/?#:]+)/i;
const PATH_RE = /\/(uv|video|seria|episode|season|serial)\/(\d+)\/([0-9a-z]+)/i;
const LINK_RE = /^(?:https?:)?\/\/[a-z0-9.-]+\.[a-z]{2,}(?::\d+)?\/[\x21-\x7e]+$/i;

export interface KodikOptions {
  /** Page the player is embedded in. Some embeds are locked to their catalog's domain. */
  referer?: string;
  /** For season/serial embeds: which episode (and season) to open. Kodik picks it server side. */
  episode?: number;
  season?: number;
}

interface PageInfo {
  params: Record<string, unknown>;
  type: string;
  hash: string;
  id: string;
  script?: string;
}

interface Endpoint {
  path: string;
  shift?: number;
}

interface LinksAnswer {
  links?: Record<string, { src?: string }[]>;
  error?: string;
}

const hostOf = (url: string) => HOST_RE.exec(url.trim())?.[1].toLowerCase() ?? "";

const onHost = (host: string, domain: string) => host === domain || host.endsWith(`.${domain}`);

/** True when the URL belongs to this player. */
export function isKodik(url: string): boolean {
  const host = hostOf(url);
  const known = [MAIN_HOST, ...LEGACY_HOSTS].some((d) => onHost(host, d)) || /(^|\.)kodik[\w-]*\.[a-z]+$/.test(host);
  return known && PATH_RE.test(url);
}

/** Sets query parameters on a URL without the URL class (the sandbox has none). */
function withQuery(url: string, set: Record<string, string>): string {
  const [head, hash = ""] = url.split("#", 2);
  const [path, query = ""] = head.split("?", 2);
  const keys = Object.keys(set);
  const pairs = query.split("&").filter((p) => p && !keys.includes(p.split("=")[0]));
  for (const [k, v] of Object.entries(set)) pairs.push(`${k}=${encodeURIComponent(v)}`);
  return `${path}${pairs.length ? `?${pairs.join("&")}` : ""}${hash ? `#${hash}` : ""}`;
}

/** Absolute https URLs to try for an embed: its own host first (unless it is a dead legacy one), then MAIN_HOST. */
function candidateUrls(embedUrl: string, opts: KodikOptions): string[] {
  let url = embedUrl.trim().replace(/^(?:https?:)?\/\//i, "https://");
  const set: Record<string, string> = {};
  if (opts.episode !== undefined) set.episode = String(opts.episode);
  if (opts.season !== undefined) set.season = String(opts.season);
  if (Object.keys(set).length) url = withQuery(url, set);
  const host = hostOf(url);
  const onMain = url.replace(HOST_RE, `https://${MAIN_HOST}`);
  if (onHost(host, MAIN_HOST)) return [url];
  if (LEGACY_HOSTS.some((d) => onHost(host, d))) return [onMain];
  return [url, onMain];
}

function caesar(s: string, shift: number): string {
  return s.replace(/[a-zA-Z]/g, (c) => {
    const base = c <= "Z" ? 65 : 97;
    return String.fromCharCode(((c.charCodeAt(0) - base + shift) % 26) + base);
  });
}

const padded = (s: string) => s + "=".repeat((4 - (s.length % 4)) % 4);

/**
 * Kodik hides each link as base64 whose latin letters are Caesar-shifted (13 until 2025, 18 in 2025-2026;
 * it changes with player builds). The shift read from the player script goes first, then the known ones,
 * then all 26. Builds before 2024 reversed the string instead, so that variant is tried as well.
 */
export function decodeLink(
  src: string,
  b64decode: (s: string) => string,
  hint?: number,
): { url: string; shift: number } | null {
  if (src.includes("//")) return { url: src, shift: hint ?? 0 };
  const shifts = [...new Set([...(hint === undefined ? [] : [hint]), ...KNOWN_SHIFTS, ...Array.from({ length: 26 }, (_, i) => i)])];
  const reversed = [...src].reverse().join("");
  for (const shift of shifts) {
    for (const variant of [src, reversed]) {
      let out = "";
      try {
        out = b64decode(padded(caesar(variant, shift)));
      } catch {
        continue;
      }
      if (LINK_RE.test(out)) return { url: out, shift };
    }
  }
  return null;
}

/** Pulls the link endpoint (base64 inside the player script) and the cipher shift out of the player script. */
export function parseScript(js: string, b64decode: (s: string) => string): Endpoint | null {
  const encoded = /type:\s*["']POST["']\s*,\s*url:\s*atob\(\s*["']([A-Za-z0-9+/=]+)["']\s*\)/.exec(js)?.[1];
  if (!encoded) return null;
  let path = "";
  try {
    path = b64decode(padded(encoded));
  } catch {
    return null;
  }
  if (!/^\/[\w\-/]{1,40}$/.test(path)) return null;
  const shift = /charCodeAt\(0\)\s*\+\s*(\d{1,2})\s*\)/.exec(js)?.[1];
  return { path, shift: shift === undefined ? undefined : Number(shift) % 26 };
}

export function parsePage(html: string, url: string): PageInfo | null {
  const raw = /urlParams\s*=\s*'(\{[^']*\})'/.exec(html)?.[1] ?? /urlParams\s*=\s*(\{[^;\n]*\})\s*;/.exec(html)?.[1];
  if (!raw) return null;
  let params: Record<string, unknown>;
  try {
    params = JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
  const info: Record<string, string> = {};
  for (const m of html.matchAll(/vInfo\.(type|hash|id)\s*=\s*['"]([^'"]+)['"]/g)) info[m[1]] ??= m[2];
  const path = PATH_RE.exec(url);
  const type = info.type ?? path?.[1];
  const id = info.id ?? path?.[2];
  const hash = info.hash ?? path?.[3];
  if (!type || !id || !hash) return null;
  const script =
    /src=["'](\/assets\/js\/app\.player_single[^"']*\.js)["']/.exec(html)?.[1] ??
    /src=["'](\/assets\/js\/app\.[^"']*\.js)["']/.exec(html)?.[1];
  return { params, type, id, hash, script };
}

const originOf = (url: string) => `https://${hostOf(url)}`;

async function loadPage(url: string, opts: KodikOptions, ctx: Ctx): Promise<PageInfo> {
  const headers: Record<string, string> = { "User-Agent": UA, Accept: "text/html,*/*" };
  if (opts.referer) headers.Referer = opts.referer;
  const res = await ctx.fetch(url, { headers });
  const html = await res.text();
  const info = res.ok ? parsePage(html, url) : null;
  if (info) return info;
  if (res.status === 404 || /not found|не найден|не существует/i.test(html)) ctx.fail.notFound(`kodik: ${url}`);
  if (/заблокирован|недоступн\S* в ваш|blocked in your/i.test(html)) ctx.fail.geoBlocked("kodik: видео недоступно в этом регионе");
  throw new Error(`kodik: unexpected embed page (HTTP ${res.status}) ${url}`);
}

async function endpoint(origin: string, script: string | undefined, fresh: boolean, ctx: Ctx): Promise<Endpoint> {
  if (!script) return { path: DEFAULT_POST_PATH };
  const key = `kodik:script:${script}`;
  const cached = fresh ? null : await ctx.kv.get(key);
  if (cached) {
    try {
      return JSON.parse(cached) as Endpoint;
    } catch {
      // fall through and read the script again
    }
  }
  let found: Endpoint | null = null;
  try {
    const res = await ctx.fetch(origin + script, { headers: { "User-Agent": UA, Referer: `${origin}/` } });
    if (res.ok) found = parseScript(await res.text(), (s) => ctx.base64.decode(s));
  } catch (e) {
    ctx.log(`kodik: player script unavailable, using ${DEFAULT_POST_PATH}: ${String(e)}`);
  }
  if (!found) return { path: DEFAULT_POST_PATH };
  await ctx.kv.set(key, JSON.stringify(found), SCRIPT_TTL);
  return found;
}

function formBody(info: PageInfo): string {
  const p = info.params;
  const str = (k: string) => (typeof p[k] === "string" ? (p[k] as string) : "");
  let ref = str("ref");
  try {
    ref = decodeURIComponent(ref);
  } catch {
    // keep as is
  }
  const fields: Record<string, string> = {
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
    info: "{}",
  };
  return Object.entries(fields)
    .map(([k, v]) => `${k}=${encodeURIComponent(v)}`)
    .join("&");
}

async function requestLinks(pageUrl: string, path: string, info: PageInfo, ctx: Ctx): Promise<LinksAnswer | null> {
  const origin = originOf(pageUrl);
  const res = await ctx.fetch(origin + path, {
    method: "POST",
    headers: {
      "User-Agent": UA,
      "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
      "X-Requested-With": "XMLHttpRequest",
      Accept: "application/json, text/javascript, */*; q=0.01",
      Origin: origin,
      Referer: pageUrl,
    },
    body: formBody(info),
  });
  if (!res.ok) return null;
  try {
    return JSON.parse(await res.text()) as LinksAnswer;
  } catch {
    return null;
  }
}

/** The quality of a link: the file name tells the truth ("480.mp4"), the answer key may not. */
export function qualityOf(key: string, url: string): number | undefined {
  const fromFile = /\/(\d{3,4})\.mp4/.exec(url);
  return Number(fromFile?.[1] ?? key) || undefined;
}

function toStreams(answer: LinksAnswer, pageUrl: string, hint: number | undefined, ctx: Ctx): { streams: Stream[]; shift?: number } {
  const headers = { Referer: `${originOf(pageUrl)}/`, "User-Agent": UA };
  const streams: Stream[] = [];
  let shift: number | undefined;
  for (const [quality, variants] of Object.entries(answer.links ?? {})) {
    const src = variants?.[0]?.src;
    if (!src) continue;
    const decoded = decodeLink(src, (s) => ctx.base64.decode(s), shift ?? hint);
    if (!decoded) continue;
    shift = decoded.shift;
    let url = decoded.url.startsWith("//") ? `https:${decoded.url}` : decoded.url;
    if (/\.mp4$/.test(url)) url += MANIFEST_SUFFIX;
    if (url.includes("/s/m/")) ctx.log("kodik: got a proxied /s/m/ link, Kodik is probably rate limiting this IP");
    // Two keys may point at the same file on old releases; list it once.
    if (streams.some((st) => st.url === url)) continue;
    streams.push({ url, kind: "hls", quality: qualityOf(quality, url), headers });
  }
  streams.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
  return { streams, shift };
}

async function resolveOn(url: string, opts: KodikOptions, ctx: Ctx): Promise<Stream[]> {
  const info = await loadPage(url, opts, ctx);
  const origin = originOf(url);
  let ep = await endpoint(origin, info.script, false, ctx);
  let answer = await requestLinks(url, ep.path, info, ctx);
  if (!answer?.links) {
    // A cached endpoint may be stale after a player update: read the script again once.
    const fresh = await endpoint(origin, info.script, true, ctx);
    if (fresh.path !== ep.path) {
      ep = fresh;
      answer = await requestLinks(url, ep.path, info, ctx);
    }
  }
  if (!answer) throw new Error(`kodik: link request to ${origin}${ep.path} failed`);
  if (answer.error) throw new Error(`kodik: ${answer.error}`);
  const stored = await ctx.kv.get(SHIFT_KEY);
  const hint = ep.shift ?? (stored === null ? undefined : Number(stored));
  const { streams, shift } = toStreams(answer, url, hint, ctx);
  if (streams.length === 0) {
    if (Object.keys(answer.links ?? {}).length > 0) throw new Error("kodik: could not decode the links, the cipher changed");
    ctx.fail.notFound(`kodik: no links for ${url}`);
  }
  if (shift !== undefined && String(shift) !== stored) await ctx.kv.set(SHIFT_KEY, String(shift));
  return streams;
}

/**
 * Resolves a Kodik embed (https or protocol-relative, any Kodik domain) into HLS streams, best first.
 * Links are bound to the requesting IP and expire within hours, so resolve right before playback.
 */
export async function resolveKodik(embedUrl: string, ctx: Ctx, opts: KodikOptions = {}): Promise<Stream[]> {
  const urls = candidateUrls(embedUrl, opts);
  let last: unknown;
  for (const url of urls) {
    try {
      return await resolveOn(url, opts, ctx);
    } catch (e) {
      const msg = String(e);
      // A missing episode or a challenge will not change on another domain.
      if (/__EXT:(NotFound|Challenge|GeoBlocked)/.test(msg) || url === urls[urls.length - 1]) throw e;
      last = e;
    }
  }
  throw last ?? new Error("kodik: no embed url");
}
