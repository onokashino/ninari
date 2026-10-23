// CdnVideoHub (CVH) player: plapi.cdnvideohub.com playlist -> Odnoklassniki CDN (okcdn.ru) HLS and MP4.
// Shared by the catalog extensions. Parsing ported from ialakey/anime-dl-core players/cvh.py (MIT).
import type { Ctx, Stream } from "../types";

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const CVH_HOSTS: string[] = ["plapi.cdnvideohub.com"];

const API = "https://plapi.cdnvideohub.com/api/v1/player/sv";
/** Publisher ids the sites put into their <video-player> tag. The API answers the same for any of them. */
const PUB_ANIMEGO = "747";
const PUB_YUMMY = "745";

/** Odnoklassniki source keys -> picture height, best first. */
const MP4_KEYS: [string, number][] = [
  ["mpeg4kUrl", 2160],
  ["mpeg2kUrl", 1440],
  ["mpegQhdUrl", 1440],
  ["mpegFullHdUrl", 1080],
  ["mpegHighUrl", 720],
  ["mpegMediumUrl", 480],
  ["mpegLowUrl", 360],
  ["mpegLowestUrl", 240],
  ["mpegTinyUrl", 144],
  ["mpegMobileUrl", 144],
];

/** What to play: a title in the CVH catalog (aggregator + id) narrowed by dub and episode, or one video. */
export interface CvhRequest {
  pub: string;
  aggr: string;
  id?: string;
  studio?: string;
  season?: number;
  episode?: number;
  video?: string;
}

/** One playlist entry: an episode in one dub. Movies have no season and episode. */
export interface CvhItem {
  vkId: string;
  voiceStudio?: string;
  voiceType?: string;
  season?: number;
  episode?: number;
}

interface CvhVideo {
  sources?: Record<string, string | null | undefined>;
}

const decode = (s: string): string => {
  try {
    return decodeURIComponent(s.replace(/\+/g, " "));
  } catch {
    return s;
  }
};

function queryOf(url: string): Record<string, string> {
  const q: Record<string, string> = {};
  const start = url.indexOf("?");
  if (start < 0) return q;
  for (const pair of url.slice(start + 1).split("#")[0].split("&")) {
    const eq = pair.indexOf("=");
    if (eq > 0) q[decode(pair.slice(0, eq))] = decode(pair.slice(eq + 1));
  }
  return q;
}

const num = (s: string | undefined): number | undefined => (s && /^\d+$/.test(s) ? Number(s) : undefined);

/** Dub names are spelled loosely across sites ("СВ дубль" vs "СВ-Дубль"): compare letters and digits only. */
export const cvhKey = (name: string | undefined): string =>
  (name ?? "").toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");

/** The dub name of an item; teams without a name (plain subtitles) are called by their voice type. */
export const cvhStudio = (item: CvhItem): string => item.voiceStudio || item.voiceType || "";

/**
 * Understands every form the catalogs hand out (protocol-relative "//host/..." and "&amp;" are fine):
 * - Animego iframe (publisher 747): /cdn-iframe/<malId>/<studio>/<season>/<episode>,
 *   /cdn-iframe/<malId>/<season>/<episode>?dubbing=<studio>, /cdn-iframe/<malId>?dubbing=<studio> (movie)
 * - YummyAnime iframe (publisher 745): .../iframeCVH.html?dubbing_code=<studio>&anime_id=<malId>&episode=<n>
 * - canonical API form: https://plapi.cdnvideohub.com/api/v1/player/sv/playlist?pub=<pub>&aggr=<aggr>&id=<id>
 *   [&dubbing_code=<studio>][&season=<n>][&episode=<n>] (see cvhPlaylistUrl)
 * - one video: https://plapi.cdnvideohub.com/api/v1/player/sv/video/<vkId>
 * - a bare MAL id: the first episode in any dub
 * Returns null for anything else.
 */
export function parseCvh(input: string): CvhRequest | null {
  const url = input.trim().replace(/&amp;/g, "&");
  if (/^\d+$/.test(url)) return { pub: PUB_ANIMEGO, aggr: "mali", id: url };
  const video = /\/player\/sv\/video\/(\d+)/.exec(url);
  if (video) return { pub: PUB_ANIMEGO, aggr: "mali", video: video[1] };
  const q = queryOf(url);
  if (/cdnvideohub\.com\/api\/v1\/player\/sv\/playlist/.test(url) && q.id) {
    return {
      pub: q.pub || PUB_ANIMEGO,
      aggr: q.aggr || "mali",
      id: q.id,
      studio: q.dubbing_code || undefined,
      season: num(q.season),
      episode: num(q.episode),
    };
  }
  if (/iframeCVH\.html/i.test(url) && q.anime_id) {
    return {
      pub: PUB_YUMMY,
      aggr: "mali",
      id: q.anime_id,
      studio: q.dubbing_code || undefined,
      season: num(q.season),
      episode: num(q.episode),
    };
  }
  const iframe = /\/cdn-iframe\/(\d+)((?:\/[^/?#]+){0,3})/.exec(url);
  if (!iframe) return null;
  const segments = iframe[2].split("/").filter(Boolean).map(decode);
  // The old Animego shape keeps the dub first in the path, the new one moved it to ?dubbing=.
  const pathStudio = segments.length > 0 && !/^\d+$/.test(segments[0]) ? segments.shift() : undefined;
  return {
    pub: PUB_ANIMEGO,
    aggr: "mali",
    id: iframe[1],
    studio: q.dubbing || pathStudio,
    season: num(segments[0]),
    episode: num(segments[1]),
  };
}

/** True when the URL belongs to this player. */
export function isCvh(url: string): boolean {
  return /cdnvideohub\.com|\/cdn-iframe\/\d+|iframeCVH\.html/i.test(url) && parseCvh(url) !== null;
}

/** Canonical input for resolveCvh, handy for catalogs that know the MAL id and the dub name. */
export function cvhPlaylistUrl(r: { id: string | number; studio?: string; episode?: number; season?: number; pub?: string; aggr?: string }): string {
  let url = `${API}/playlist?pub=${r.pub ?? PUB_ANIMEGO}&aggr=${r.aggr ?? "mali"}&id=${encodeURIComponent(String(r.id))}`;
  if (r.studio) url += `&dubbing_code=${encodeURIComponent(r.studio)}`;
  if (r.season != null) url += `&season=${r.season}`;
  if (r.episode != null) url += `&episode=${r.episode}`;
  return url;
}

/** Every episode in every dub of a title; empty when CVH does not have it (the API answers 204). */
export async function cvhPlaylist(id: string, ctx: Ctx, pub = PUB_ANIMEGO, aggr = "mali"): Promise<CvhItem[]> {
  const res = await ctx.fetch(cvhPlaylistUrl({ id, pub, aggr }), { headers: { Accept: "application/json" } });
  if (res.status === 204) return [];
  if (!res.ok) throw new Error(`cvh: playlist ${aggr}/${id}: HTTP ${res.status}`);
  const text = await res.text();
  if (!text.trim()) return [];
  const data = JSON.parse(text) as { items?: CvhItem[] | null };
  return (data.items ?? []).filter((i) => i && i.vkId);
}

/** An upload never changes, so its quality is remembered for a month. */
const QUALITY_TTL = 30 * 24 * 3600;

async function videoQuality(vkId: string, ctx: Ctx): Promise<number | undefined> {
  const key = `cvh-quality:${vkId}`;
  const known = await ctx.kv.get(key);
  if (known !== null) return Number(known) || undefined;
  try {
    const best = Math.max(0, ...(await cvhVideo(vkId, ctx)).map((s) => s.quality ?? 0));
    await ctx.kv.set(key, String(best), QUALITY_TTL);
    return best || undefined;
  } catch {
    return undefined;
  }
}

/**
 * The best quality of every dub of a title, by cvhKey of the studio. The playlist carries no
 * quality, so one video per dub is asked (4K uploads exist, 1080p is not a ceiling).
 */
export async function cvhDubQualities(id: string, ctx: Ctx, pub = PUB_ANIMEGO, aggr = "mali"): Promise<Map<string, number>> {
  const out = new Map<string, number>();
  let items: CvhItem[];
  try {
    items = await cvhPlaylist(id, ctx, pub, aggr);
  } catch {
    return out;
  }
  const firstOfDub = new Map<string, CvhItem>();
  for (const item of items) {
    const key = cvhKey(cvhStudio(item));
    if (key && !firstOfDub.has(key)) firstOfDub.set(key, item);
  }
  await Promise.all(
    [...firstOfDub].map(async ([key, item]) => {
      const q = await videoQuality(item.vkId, ctx);
      if (q) out.set(key, q);
    }),
  );
  return out;
}

function byStudio(items: CvhItem[], studio: string): CvhItem[] {
  const want = cvhKey(studio);
  const exact = items.filter((i) => cvhKey(cvhStudio(i)) === want);
  if (exact.length > 0 || !want) return exact;
  return items.filter((i) => {
    const k = cvhKey(cvhStudio(i));
    return k !== "" && (k.includes(want) || want.includes(k));
  });
}

/**
 * Picks the entry for a dub and episode. The episode number decides: sites disagree with CVH about
 * seasons (Animego says 1x1 where CVH files a third season as 3x1), so the season only breaks ties.
 */
export function pickCvhItem(items: CvhItem[], req: CvhRequest, ctx: Ctx): CvhItem {
  if (items.length === 0) ctx.fail.notFound(`CVH has no title ${req.aggr}/${req.id}`);
  const pool = req.studio ? byStudio(items, req.studio) : items;
  if (pool.length === 0) ctx.fail.notFound(`CVH has no dub "${req.studio}" for ${req.aggr}/${req.id}`);
  const episode = req.episode ?? Math.min(...pool.map((i) => i.episode ?? 1));
  const hits = pool.filter((i) => (i.episode ?? 1) === episode);
  if (hits.length === 0) ctx.fail.notFound(`CVH has no episode ${episode} in "${req.studio ?? "any dub"}"`);
  return hits.find((i) => req.season != null && i.season === req.season) ?? hits[0];
}

/** Streams of one video: the HLS master first, then progressive MP4, best quality first. DASH is skipped. */
export async function cvhVideo(vkId: string, ctx: Ctx): Promise<Stream[]> {
  const res = await ctx.fetch(`${API}/video/${encodeURIComponent(vkId)}`, { headers: { Accept: "application/json" } });
  if (!res.ok) throw new Error(`cvh: video ${vkId}: HTTP ${res.status}`);
  const sources = ((await res.json()) as CvhVideo).sources ?? {};
  const mp4: Stream[] = [];
  for (const [key, quality] of MP4_KEYS) {
    const url = sources[key];
    if (url && !mp4.some((s) => s.quality === quality)) mp4.push({ url, kind: "mp4", quality });
  }
  const out: Stream[] = [];
  if (sources.hlsUrl) out.push({ url: sources.hlsUrl, kind: "hls", quality: mp4[0]?.quality });
  out.push(...mp4);
  if (out.length === 0) throw new Error(`cvh: video ${vkId} has no sources`);
  return out;
}

/** Input forms are listed at parseCvh. The okcdn links are bound to the caller's IP and need no headers. */
export async function resolveCvh(embedOrId: string, ctx: Ctx): Promise<Stream[]> {
  const req = parseCvh(embedOrId);
  if (!req) throw new Error(`cvh: unsupported player url ${embedOrId}`);
  if (req.video) return cvhVideo(req.video, ctx);
  const items = await cvhPlaylist(req.id ?? "", ctx, req.pub, req.aggr);
  return cvhVideo(pickCvhItem(items, req, ctx).vkId, ctx);
}
