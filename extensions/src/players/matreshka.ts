// Matreshka TV (matreshka.tv) player: the embed page carries HLS masters on the cmtv.ru CDN. Shared by the catalog extensions.
import type { Ctx, FetchInit, Stream } from "../types";

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const MATRESHKA_HOSTS: string[] = ["matreshka.tv", "*.matreshka.tv", "*.cmtv.ru"];

const PAGE_TIMEOUT_MS = 8000;
const EMBED_RE = /^(?:https?:)?\/\/(?:www\.)?matreshka\.tv\/(?:embed\/)?video\/([0-9a-z_-]{6,})/i;
/** The page is a Nuxt payload: URLs have their slashes escaped as /. */
const MASTER_RE = /https:\/\/[^"'\s\\]+?\/master\.m3u8[^"'\s\\]*/g;
/** Each master names what it holds in a base64 path segment: {"id":..,"codec":"h264","userQuality":[720]}. */
const SPEC_RE = /\/(eyJ[0-9A-Za-z+/=_-]+)\//;

interface Spec {
  id?: string;
  codec?: string;
  userQuality?: number[];
}

/** True when the URL belongs to this player. */
export function isMatreshka(url: string): boolean {
  return EMBED_RE.test(url.trim());
}

export function matreshkaId(url: string): string | undefined {
  return EMBED_RE.exec(url.trim())?.[1];
}

/**
 * Streams of an embed page: the adaptive master first (quality: its best rendition), then the
 * single-quality masters, best first. H.264 only; AV1 masters are skipped. Empty when the page has no video.
 */
export function parseMatreshka(html: string, b64decode: (s: string) => string, videoId?: string): Stream[] {
  const text = html.replace(/\\u002F/gi, "/");
  let adaptive: string | undefined;
  const fixed = new Map<number, string>();
  for (const url of new Set(text.match(MASTER_RE) ?? [])) {
    const seg = SPEC_RE.exec(url)?.[1];
    if (!seg) continue;
    let spec: Spec;
    try {
      spec = JSON.parse(b64decode(seg)) as Spec;
    } catch {
      continue;
    }
    // Recommendations on the same page carry other videos.
    if ((videoId && spec.id && spec.id !== videoId) || (spec.codec && spec.codec !== "h264")) continue;
    const q = spec.userQuality ?? [];
    if (q.length === 0) adaptive ??= url;
    else if (q.length === 1 && q[0] > 0 && !fixed.has(q[0])) fixed.set(q[0], url);
  }
  const heights = [...fixed.keys()].sort((a, b) => b - a);
  const out: Stream[] = heights.map((h) => ({ url: fixed.get(h)!, kind: "hls", quality: h }));
  if (adaptive) out.unshift({ url: adaptive, kind: "hls", quality: heights[0] });
  return out;
}

/** Resolves a Matreshka embed (matreshka.tv/embed/video/<id>). The CDN needs no headers. */
export async function resolveMatreshka(embedUrl: string, ctx: Ctx): Promise<Stream[]> {
  const id = matreshkaId(embedUrl);
  if (!id) throw new Error(`matreshka: no video id in ${embedUrl}`);
  const page = `https://matreshka.tv/embed/video/${id}`;
  const init: FetchInit = { timeout: PAGE_TIMEOUT_MS };
  const res = await ctx.fetch(page, init);
  if (res.status === 404) ctx.fail.notFound(`matreshka: video ${id} is gone`);
  if (!res.ok) throw new Error(`matreshka: HTTP ${res.status} for ${page}`);
  const streams = parseMatreshka(await res.text(), (s) => ctx.base64.decode(s), id);
  // A removed video still answers 200 with the site shell, just without a player.
  if (streams.length === 0) ctx.fail.notFound(`matreshka: video ${id} has no streams`);
  return streams;
}
