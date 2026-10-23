// Dzen (dzen.ru) player: the embed page lists okcdn.ru streams (HLS, DASH, MP4). Shared by the catalog extensions.
import type { Ctx, FetchInit, Stream } from "../types";
import { OKCDN_UA } from "./okru";

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const DZEN_HOSTS: string[] = ["dzen.ru", "*.dzen.ru", "*.okcdn.ru"];

/** Stream types on the page -> picture height. "hls" and "dash" switch between them. */
const TYPES: Record<string, number> = {
  tiny: 144,
  lowest: 240,
  low: 360,
  medium: 480,
  high: 720,
  fullhd: 1080,
  quadhd: 1440,
  ultrahd: 2160,
};

const PAGE_TIMEOUT_MS = 8000;
const EMBED_RE = /^(?:https?:)?\/\/(?:www\.)?(?:dzen\.ru|zen\.yandex\.ru)\/(?:embed|video\/watch)\/([0-9a-z_-]{6,})/i;
const STREAMS_RE = /"streams"\s*:\s*(\[[^\]]*\])/;

interface DzenStream {
  url?: string;
  type?: string;
}

/** True when the URL belongs to this player. */
export function isDzen(url: string): boolean {
  return EMBED_RE.test(url.trim());
}

export function dzenId(url: string): string | undefined {
  return EMBED_RE.exec(url.trim())?.[1];
}

/** HLS first (its quality is the best MP4 next to it), then the MP4 files, best first. DASH is skipped. */
export function parseDzen(html: string, headers: Record<string, string>): Stream[] {
  const raw = STREAMS_RE.exec(html)?.[1];
  if (!raw) return [];
  let list: DzenStream[];
  try {
    list = JSON.parse(raw) as DzenStream[];
  } catch {
    return [];
  }
  const mp4: Stream[] = [];
  let hls: string | undefined;
  for (const s of list) {
    if (!s.url) continue;
    if (s.type === "hls") hls ??= s.url;
    const quality = TYPES[s.type ?? ""];
    if (quality && !mp4.some((m) => m.quality === quality)) mp4.push({ url: s.url, kind: "mp4", quality, headers });
  }
  mp4.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
  return hls ? [{ url: hls, kind: "hls", quality: mp4[0]?.quality, headers }, ...mp4] : mp4;
}

/**
 * Resolves a Dzen embed (dzen.ru/embed/<id>?...). The page is large (about 0.5 MB) but carries the
 * stream list inline. The okcdn links need the same browser agent that fetched the page (see OKCDN_UA)
 * and are bound to the caller's IP.
 */
export async function resolveDzen(embedUrl: string, ctx: Ctx): Promise<Stream[]> {
  const id = dzenId(embedUrl);
  if (!id) throw new Error(`dzen: no video id in ${embedUrl}`);
  const page = `https://dzen.ru/embed/${id}`;
  const init: FetchInit = { headers: { "User-Agent": OKCDN_UA }, timeout: PAGE_TIMEOUT_MS };
  const res = await ctx.fetch(page, init);
  if (res.status === 404) ctx.fail.notFound(`dzen: video ${id} is gone`);
  if (!res.ok) throw new Error(`dzen: HTTP ${res.status} for ${page}`);
  const streams = parseDzen(await res.text(), { "User-Agent": OKCDN_UA, Referer: "https://dzen.ru/" });
  if (streams.length === 0) ctx.fail.notFound(`dzen: video ${id} has no streams`);
  return streams;
}
