// VK Video player: turns a video_ext.php embed into okcdn.ru HLS and MP4. Shared by the catalog extensions.
import type { Ctx, FetchInit, Stream } from "../types";
import { OKCDN_UA } from "./okru";

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const VK_HOSTS: string[] = [
  "vk.com",
  "*.vk.com",
  "vk.ru",
  "*.vk.ru",
  "vkvideo.ru",
  "*.vkvideo.ru",
  // CDN: okcdn.ru today, the other two carried the files before 2024 and old pages may still point there.
  "*.okcdn.ru",
  "*.vkuser.net",
  "*.userapi.com",
];

const PAGE_TIMEOUT_MS = 8000;
const EMBED_RE = /^(?:https?:)?\/\/(?:www\.|m\.)?(?:vk\.com|vk\.ru|vkvideo\.ru)\/video_ext\.php\?/i;
const FILES_RE = /"files"\s*:\s*(\{[^{}]*\})/;
/** Before 2025 the page kept the links in playerParams as url240..url1080 and hls. */
const OLD_KEY_RE = /"(url\d{3,4}|hls)"\s*:\s*"((?:[^"\\]|\\.)+)"/g;
const MESSAGE_RE = /id="video_ext_msg"[^>]*>([^<]*)</;

/** True when the URL belongs to this player. */
export function isVk(url: string): boolean {
  const u = url.trim();
  return EMBED_RE.test(u) && /[?&]oid=-?\d+/.test(u) && /[?&]id=\d+/.test(u);
}

/** The canonical embed URL: vk.com, https, only the parameters that pick the video. */
export function vkEmbedUrl(url: string): string | undefined {
  const u = url.trim().replace(/&amp;/g, "&");
  const oid = /[?&]oid=(-?\d+)/.exec(u)?.[1];
  const id = /[?&]id=(\d+)/.exec(u)?.[1];
  if (!EMBED_RE.test(u) || !oid || !id) return undefined;
  const hash = /[?&]hash=([0-9a-f]+)/i.exec(u)?.[1];
  return `https://vk.com/video_ext.php?oid=${oid}&id=${id}${hash ? `&hash=${hash}` : ""}`;
}

/** Picture height of a file key: mp4_720 and url720 are 720p, anything else is not an MP4 rendition. */
const heightOf = (key: string): number | undefined => {
  const m = /^(?:mp4_|url)(\d{3,4})$/.exec(key);
  return m ? Number(m[1]) : undefined;
};

function toStreams(files: Record<string, unknown>, headers: Record<string, string>): Stream[] {
  const mp4: Stream[] = [];
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

/** Streams of a video_ext page, HLS first, then MP4 best first; empty when the page has none. */
export function parseVk(html: string, headers: Record<string, string>): Stream[] {
  const raw = FILES_RE.exec(html)?.[1];
  if (raw) {
    try {
      return toStreams(JSON.parse(raw) as Record<string, unknown>, headers);
    } catch {
      // try the old layout below
    }
  }
  const old: Record<string, string> = {};
  for (const m of html.matchAll(OLD_KEY_RE)) {
    try {
      old[m[1]] ??= JSON.parse(`"${m[2]}"`) as string;
    } catch {
      // skip a value that is not a JSON string
    }
  }
  return toStreams(old, headers);
}

/** The text VK shows instead of the player (removed, private, blocked on a rights holder's request). */
export function vkMessage(html: string): string | undefined {
  return MESSAGE_RE.exec(html)?.[1]?.trim() || undefined;
}

/**
 * Resolves a VK embed (vk.com, vk.ru or vkvideo.ru video_ext.php?oid=&id=&hash=). The page is
 * windows-1251, which the host decodes; only ASCII parts of it are read here. Private videos and
 * uploads taken down on a rights holder's request come back as NotFound.
 */
export async function resolveVk(embedUrl: string, ctx: Ctx): Promise<Stream[]> {
  const page = vkEmbedUrl(embedUrl);
  if (!page) throw new Error(`vk: not a video_ext url ${embedUrl}`);
  const init: FetchInit = { headers: { "User-Agent": OKCDN_UA }, timeout: PAGE_TIMEOUT_MS };
  // Now and then VK answers with a page that has neither the player nor a message; the next one is fine.
  for (let attempt = 0; attempt < 2; attempt++) {
    const res = await ctx.fetch(page, init);
    if (res.status === 404) ctx.fail.notFound(`vk: ${page} is gone`);
    if (!res.ok) throw new Error(`vk: HTTP ${res.status} for ${page}`);
    const html = await res.text();
    const streams = parseVk(html, { "User-Agent": OKCDN_UA, Referer: "https://vk.com/" });
    if (streams.length > 0) return streams;
    const msg = vkMessage(html);
    if (msg) ctx.fail.notFound(`vk: ${msg}`);
  }
  throw new Error(`vk: no player data on ${page}`);
}
