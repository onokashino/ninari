// Odnoklassniki (ok.ru) player: turns a /videoembed/ page into okcdn.ru HLS and MP4. Shared by the catalog extensions.
import type { Ctx, FetchInit, Stream } from "../types";

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const OKRU_HOSTS: string[] = ["ok.ru", "*.ok.ru", "*.okcdn.ru"];

/**
 * okcdn links carry the agent that asked for the page (srcAg=CHROME) and stall until timeout for any
 * other agent, so the page is fetched with a browser agent and the same one travels with the stream.
 */
export const OKCDN_UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/** Odnoklassniki quality names -> picture height (the same names serve VK and Dzen uploads). */
export const OK_QUALITIES: Record<string, number> = {
  mobile: 144,
  lowest: 240,
  low: 360,
  sd: 480,
  hd: 720,
  full: 1080,
  quad: 1440,
  ultra: 2160,
};

const PAGE_TIMEOUT_MS = 8000;
const EMBED_RE = /^(?:https?:)?\/\/(?:www\.|m\.)?(?:ok\.ru|odnoklassniki\.ru)\/(?:videoembed|video)\/(\d+)/i;
const OPTIONS_RE = /data-options="([^"]*)"/;
const STUB_RE = /vp_video_stub_txt[^>]*>([^<]*)</;

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };

/** Attribute values come HTML-escaped (&quot; and friends). */
const unescapeHtml = (s: string): string =>
  s.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (m, e: string) =>
    e[0] === "#" ? String.fromCodePoint(e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : Number(e.slice(1))) : (ENTITIES[e.toLowerCase()] ?? m),
  );

interface OkVideo {
  name?: string;
  url?: string;
  disallowed?: boolean;
}

export interface OkMetadata {
  hlsManifestUrl?: string;
  ondemandHls?: string;
  videos?: OkVideo[];
}

/** True when the URL belongs to this player. */
export function isOkru(url: string): boolean {
  return EMBED_RE.test(url.trim());
}

export function okruId(url: string): string | undefined {
  return EMBED_RE.exec(url.trim())?.[1];
}

/** The player metadata of an embed page, or the text of the stub it shows instead (blocked, removed). */
export function parseOkruPage(html: string): { metadata: OkMetadata } | { stub: string } | null {
  const raw = OPTIONS_RE.exec(html)?.[1];
  if (raw) {
    try {
      const options = JSON.parse(unescapeHtml(raw)) as { flashvars?: { metadata?: string | OkMetadata } };
      const md = options.flashvars?.metadata;
      // Older pages keep the metadata as a JSON string inside the options.
      const metadata = typeof md === "string" ? (JSON.parse(md) as OkMetadata) : md;
      if (metadata) return { metadata };
    } catch {
      // fall through to the stub check
    }
  }
  const stub = STUB_RE.exec(html)?.[1]?.trim();
  return stub ? { stub } : null;
}

/** HLS first (it switches between all the renditions), then the progressive MP4 files, best first. */
export function okruStreams(md: OkMetadata, headers: Record<string, string>): Stream[] {
  const mp4: Stream[] = [];
  for (const v of md.videos ?? []) {
    const quality = OK_QUALITIES[v.name ?? ""];
    if (!v.url || v.disallowed || !quality || mp4.some((s) => s.quality === quality)) continue;
    mp4.push({ url: v.url, kind: "mp4", quality, headers });
  }
  mp4.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
  const hls = md.hlsManifestUrl || md.ondemandHls;
  return hls ? [{ url: hls, kind: "hls", quality: mp4[0]?.quality, headers }, ...mp4] : mp4;
}

/**
 * Resolves an ok.ru embed (https, http or protocol-relative /videoembed/<id> or /video/<id>).
 * Most anime uploads there are blocked for copyright: the stub text comes back as NotFound.
 * The links are bound to the caller's IP and expire within hours, so resolve right before playback.
 */
export async function resolveOkru(embedUrl: string, ctx: Ctx): Promise<Stream[]> {
  const id = okruId(embedUrl);
  if (!id) throw new Error(`okru: no video id in ${embedUrl}`);
  const page = `https://ok.ru/videoembed/${id}`;
  const init: FetchInit = { headers: { "User-Agent": OKCDN_UA }, timeout: PAGE_TIMEOUT_MS };
  const res = await ctx.fetch(page, init);
  if (res.status === 404) ctx.fail.notFound(`okru: video ${id} is gone`);
  if (!res.ok) throw new Error(`okru: HTTP ${res.status} for ${page}`);
  const parsed = parseOkruPage(await res.text());
  if (!parsed) throw new Error(`okru: no player data on ${page}`);
  if ("stub" in parsed) ctx.fail.notFound(`okru: ${parsed.stub}`);
  const streams = okruStreams(parsed.metadata, { "User-Agent": OKCDN_UA, Referer: "https://ok.ru/" });
  if (streams.length === 0) ctx.fail.notFound(`okru: video ${id} has no playable sources`);
  return streams;
}
