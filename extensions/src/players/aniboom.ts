// Aniboom player: turns an aniboom.one embed into its HLS master. Shared by the catalog extensions.
// Approach as in YaNesyTortiK/AnimeParsers parser_animego.py (MIT), re-checked live.
import type { Ctx, Stream } from "../types";

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const ANIBOOM_HOSTS: string[] = ["aniboom.one"];

/** The embed answers 404 without a Referer; any partner origin will do. */
const DEFAULT_REFERER = "https://animego.me/";
/** The CDN (rotating *.ya-*.site hosts) is fine without them today, but the player sends them. */
const STREAM_HEADERS = { Referer: "https://aniboom.one/", Origin: "https://aniboom.one" };

interface Parameters {
  hls?: string;
  fallbackHls?: string;
  qualityVideo?: number;
}

/** True when the URL belongs to this player. */
export function isAniboom(url: string): boolean {
  return /(^|\/\/|\.)aniboom\.one\/embed\//i.test(url);
}

const absolute = (url: string): string => {
  const u = url.trim().replace(/&amp;/g, "&");
  return u.startsWith("//") ? `https:${u}` : u;
};

/** `hls` in data-parameters is itself a JSON string: {"src": "...master.m3u8", "type": "application/x-mpegURL"}. */
function hlsSrc(raw: string | undefined): string | undefined {
  if (!raw || raw === "null") return undefined;
  try {
    return (JSON.parse(raw) as { src?: string }).src || undefined;
  } catch {
    return undefined;
  }
}

/** qualityVideo is the real frame height (1920x804 gives 804): report the nominal tier it belongs to. */
const TIERS = [240, 360, 480, 720, 1080, 1440, 2160];
const tier = (height: number | undefined): number | undefined =>
  height && height > 0 ? (TIERS.find((t) => t >= height) ?? height) : undefined;

const isChallenge = (status: number, body: string): boolean =>
  (status === 403 || status === 503) && /ddos-guard|just a moment|cf-chl/i.test(body.slice(0, 4096));

/**
 * embedUrl: https://aniboom.one/embed/<id>?episode=<n>&translation=<aniboomTranslationId>, exactly as the
 * catalog (Animego) gives it; movies have no episode. Returns the HLS master (360p-1080p, fMP4 segments,
 * audio in EXT-X-MEDIA; newer uploads are AV1 + Opus) with the headers the CDN expects, then the H.264
 * fallback master when there is one (lower resolution, quality unknown). aniboom.one sits behind
 * DDoS-Guard: a challenge page is reported as a Challenge error.
 */
export async function resolveAniboom(embedUrl: string, ctx: Ctx, referer = DEFAULT_REFERER): Promise<Stream[]> {
  const url = absolute(embedUrl);
  const res = await ctx.fetch(url, { headers: { Referer: referer } });
  const body = await res.text();
  if (isChallenge(res.status, body)) ctx.fail.challenge(url);
  if (res.status === 404) ctx.fail.notFound(`aniboom has no such video: ${url}`);
  if (!res.ok) throw new Error(`aniboom: HTTP ${res.status} for ${url}`);
  const raw = ctx.html.select(body, "video[data-parameters]")[0]?.attrs["data-parameters"];
  if (!raw) throw new Error(`aniboom: no player parameters on ${url}`);
  const params = JSON.parse(raw) as Parameters;
  const out: Stream[] = [];
  const main = hlsSrc(params.hls);
  const fallback = hlsSrc(params.fallbackHls);
  if (main) out.push({ url: main, kind: "hls", quality: tier(params.qualityVideo), headers: { ...STREAM_HEADERS } });
  if (fallback && fallback !== main) out.push({ url: fallback, kind: "hls", headers: { ...STREAM_HEADERS } });
  if (out.length === 0) throw new Error(`aniboom: no HLS source on ${url}`);
  return out;
}
