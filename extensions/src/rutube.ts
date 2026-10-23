// Rutube: dub teams upload their episodes to official channels, and Rutube's catalogue groups them
// into a show with seasons ("metainfo/tv"). Only videos in such a catalogue entry are used, never
// loose uploads found by search. The streams are open HLS, often up to 1080p.
import type { Ctx, Dub, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";

export const manifest: Manifest = {
  id: "rutube",
  name: "Rutube",
  version: "0.1.0",
  apiVersion: 1,
  domains: ["https://rutube.ru"],
  allowedHosts: ["*.rutube.ru", "*.rtbcdn.ru"],
};

const HEADERS = { Accept: "application/json" };
const VIDEOS_TTL = 600;
const QUALITY_TTL = 7 * 24 * 3600;
const MAX_PAGES = 20;

// ---------- names ----------

const normalize = (s: string) =>
  s
    .toLowerCase()
    .replace(/ё/g, "е")
    .replace(/[^\p{L}\p{N}]+/gu, "");

const CHINESE_DIGITS: Record<string, number> = { 一: 1, 二: 2, 三: 3, 四: 4, 五: 5, 六: 6, 七: 7, 八: 8, 九: 9, 十: 10 };
// JavaScript's \b knows only Latin letters, so Russian words are bounded by hand.
const PART = /\b(?:part|cour)\s*\d|(?<![а-яё])часть(?![а-яё])/i;
const MARKERS: RegExp[] = [
  /\s*\b(\d+)(?:st|nd|rd|th)\s+season\b/i,
  /\s*\bseason\s+(\d+)\b/i,
  /\s*\b(\d+)(?:-?й)?\s+сезон(?![а-яё])/i,
  /\s*第([一二三四五六七八九十\d]+)季/,
];
/** "Трон, отмеченный богом 2": Shikimori's Russian names number sequels at the end. */
const TRAILING = /\s+(\d{1,2})$/;

const toNumber = (s: string) => (/^\d+$/.test(s) ? Number(s) : (CHINESE_DIGITS[s] ?? NaN));

/**
 * The season of the show a Shikimori title is, and its names without the season. `null` for a
 * season split into parts: where such a part starts in the show cannot be told from its name.
 */
export function seasonOf(names: string[]): { season: number; names: string[] } | null {
  if (names.some((n) => PART.test(n))) return null;
  let marked: number | undefined;
  let trailing: number | undefined;
  const bases: string[] = [];
  for (const name of names) {
    let base = name.trim();
    for (const re of MARKERS) {
      const m = re.exec(base);
      if (!m) continue;
      marked ??= toNumber(m[1]);
      base = base.replace(re, "").trim();
    }
    const t = TRAILING.exec(base);
    if (t && Number(t[1]) >= 2 && Number(t[1]) <= 20) {
      trailing ??= Number(t[1]);
      base = base.replace(TRAILING, "").trim();
    }
    if (base && !bases.includes(base)) bases.push(base);
  }
  const season = marked ?? trailing ?? 1;
  return Number.isFinite(season) ? { season, names: bases } : null;
}

/** A catalogue name like "Трон, отмеченный Богом / Shen Yin Wang Zuo" lists the show's names. */
export function feedMatches(feedName: string, names: string[]): boolean {
  const parts = feedName.split("/").map(normalize).filter(Boolean);
  return names.some((n) => parts.includes(normalize(n)));
}

// ---------- episodes ----------

export interface Video {
  id: string;
  season: number;
  episode: number;
  title: string;
  author: { id: number; name: string };
}

/** "Трон, отмеченный Богом, 3 сезон, 16 серия (68 серия)": the number through the whole show. */
const showWide = (title: string) => {
  const m = /\((\d+)\s*серия\)/i.exec(title);
  return m ? Number(m[1]) : undefined;
};

/**
 * Shikimori episode number -> video id, per uploader. A Shikimori season starts at the first
 * episode of the same season here. When a team numbers the whole show, the count runs on through
 * later seasons by that number, so a season missing on Rutube leaves a gap instead of shifting
 * everything after it; otherwise only the season itself is used. `limit` (0: unknown) is the
 * title's episode count: past it the next Shikimori title begins.
 */
export function episodeMap(videos: Video[], season: number, limit: number): Map<string, Map<number, string>> {
  const byAuthor = new Map<string, Video[]>();
  for (const v of videos) {
    if (v.season < 1 || v.episode < 1) continue;
    const key = String(v.author.id);
    byAuthor.set(key, [...(byAuthor.get(key) ?? []), v]);
  }
  const out = new Map<string, Map<number, string>>();
  for (const [author, list] of byAuthor) {
    const starts = list
      .filter((v) => v.season === season)
      .map((v) => {
        const n = showWide(v.title);
        return n === undefined ? undefined : n - v.episode + 1;
      })
      .filter((n): n is number => n !== undefined);
    const start = starts.length > 0 ? Math.min(...starts) : undefined;
    const numbered: [number, string][] = [];
    for (const v of list) {
      const wide = showWide(v.title);
      const n =
        start !== undefined && wide !== undefined ? wide - start + 1 : v.season === season ? v.episode : undefined;
      if (n === undefined || n < 1 || (limit > 0 && n > limit)) continue;
      if (!numbered.some(([k]) => k === n)) numbered.push([n, v.id]);
    }
    numbered.sort((a, b) => a[0] - b[0]);
    if (numbered.length > 0) out.set(author, new Map(numbered));
  }
  return out;
}

// ---------- streams ----------

/** A picture size as the usual quality step; a wide 1920x800 film frame is still 1080p. */
const qualityOf = (w: number, h: number) =>
  w >= 1900 || h >= 1000 ? 1080 : w >= 1260 || h >= 700 ? 720 : w >= 840 || h >= 460 ? 480 : w >= 620 || h >= 340 ? 360 : h;

/** One stream per quality from an HLS master playlist, best first; Rutube lists each size per CDN. */
export function variants(master: string): { url: string; quality: number }[] {
  const lines = master.split(/\r?\n/).map((l) => l.trim());
  const out: { url: string; quality: number }[] = [];
  for (let i = 0; i < lines.length; i++) {
    const m = /^#EXT-X-STREAM-INF:.*RESOLUTION=(\d+)x(\d+)/.exec(lines[i]);
    if (!m) continue;
    const url = lines.slice(i + 1).find((l) => l && !l.startsWith("#"));
    const quality = qualityOf(Number(m[1]), Number(m[2]));
    if (url?.startsWith("http") && !out.some((v) => v.quality === quality)) out.push({ url, quality });
  }
  return out.sort((a, b) => b.quality - a.quality);
}

// ---------- API ----------

async function api<T>(path: string, ctx: Ctx): Promise<T> {
  const res = await ctx.fetch(`${ctx.baseUrl}${path}`, { headers: HEADERS });
  if (!res.ok) throw new Error(`rutube: HTTP ${res.status} on ${path}`);
  return (await res.json()) as T;
}

async function cached<T>(key: string, ttl: number, ctx: Ctx, load: () => Promise<T>): Promise<T> {
  const hit = await ctx.kv.get(key);
  if (hit) {
    try {
      return JSON.parse(hit) as T;
    } catch {
      // reload below
    }
  }
  const value = await load();
  await ctx.kv.set(key, JSON.stringify(value), ttl);
  return value;
}

interface SearchHit {
  feed_url?: string | null;
  feed_name?: string | null;
}

interface Page<T> {
  results: T[];
  has_next?: boolean;
}

interface ApiVideo {
  id: string;
  title?: string;
  season?: number | null;
  episode?: number | null;
  author?: { id?: number; name?: string } | null;
}

const hasCyrillic = (s: string) => /[а-яё]/i.test(s);

export async function search(q: SearchQuery, ctx: Ctx): Promise<Match[]> {
  const s = seasonOf(q.names);
  if (!s) return [];
  const queries = [...s.names.filter(hasCyrillic), ...s.names.filter((n) => !hasCyrillic(n))].slice(0, 2);
  for (const query of queries) {
    const page = await api<Page<SearchHit>>(`/api/search/video/?query=${encodeURIComponent(query)}`, ctx);
    for (const hit of page.results ?? []) {
      const feed = /\/metainfo\/tv\/(\d+)/.exec(hit.feed_url ?? "")?.[1];
      if (!feed || !hit.feed_name || !feedMatches(hit.feed_name, s.names)) continue;
      const name = hit.feed_name.split("/")[0].trim();
      return [
        {
          id: feed,
          title: s.season > 1 ? `${name} ${s.season}` : name,
          confidence: "exact",
          data: { season: s.season, limit: q.episodes ?? 0 },
        },
      ];
    }
  }
  return [];
}

/** Every episode video of a catalogue entry, all seasons. */
function feedVideos(feed: string, ctx: Ctx): Promise<Video[]> {
  return cached(`videos:${feed}`, VIDEOS_TTL, ctx, async () => {
    const seasons = await api<{ number: number }[]>(`/api/metainfo/tv/${feed}/season/`, ctx);
    const out: Video[] = [];
    for (const { number } of seasons) {
      if (number < 1) continue;
      for (let page = 1; page <= MAX_PAGES; page++) {
        const p = await api<Page<ApiVideo>>(`/api/metainfo/tv/${feed}/video?season=${number}&show_all=1&page=${page}`, ctx);
        for (const v of p.results ?? []) {
          if (!v.author?.id || v.season == null || v.episode == null) continue;
          out.push({
            id: v.id,
            season: v.season,
            episode: v.episode,
            title: v.title ?? "",
            author: { id: v.author.id, name: v.author.name ?? "Rutube" },
          });
        }
        if (!p.has_next) break;
      }
    }
    return out;
  });
}

const settings = (m: Match) => {
  const d = (m.data ?? {}) as { season?: number; limit?: number };
  return { season: d.season ?? 1, limit: d.limit ?? 0 };
};

async function master(videoId: string, ctx: Ctx): Promise<string> {
  const options = await api<{ video_balancer?: { m3u8?: string } | null }>(
    `/api/play/options/${videoId}/?no_404=true&referer=${encodeURIComponent("https://rutube.ru")}`,
    ctx,
  );
  const url = options.video_balancer?.m3u8;
  if (!url) ctx.fail.notFound("rutube: видео недоступно");
  const res = await ctx.fetch(url);
  if (!res.ok) throw new Error(`rutube: HTTP ${res.status} on the playlist`);
  return res.text();
}

/** The best picture of one video, used as the team's quality for the whole show. */
function bestQuality(videoId: string, ctx: Ctx): Promise<number | null> {
  return cached(`quality:${videoId}`, QUALITY_TTL, ctx, async () => {
    try {
      return variants(await master(videoId, ctx))[0]?.quality ?? null;
    } catch (e) {
      ctx.log(`rutube: no quality for ${videoId}: ${String(e)}`);
      return null;
    }
  });
}

export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const { season, limit } = settings(m);
  const videos = await feedVideos(m.id, ctx);
  const map = episodeMap(videos, season, limit);
  const dubs: Dub[] = [];
  const available = new Map<number, { dubId: string; maxQuality?: number }[]>();
  for (const [author, eps] of map) {
    const own = videos.filter((v) => String(v.author.id) === author);
    const subtitled = own.filter((v) => /субтитр/i.test(v.title)).length > own.length / 2;
    dubs.push({ id: author, name: own[0].author.name, type: subtitled ? "subtitles" : "voice" });
    const latest = [...eps.values()].pop()!;
    const quality = (await bestQuality(latest, ctx)) ?? undefined;
    for (const n of eps.keys()) {
      available.set(n, [...(available.get(n) ?? []), quality ? { dubId: author, maxQuality: quality } : { dubId: author }]);
    }
  }
  return {
    dubs,
    episodes: [...available.entries()].sort((a, b) => a[0] - b[0]).map(([number, avail]) => ({ number, available: avail })),
  };
}

export async function resolve(m: Match, episode: number, dubId: string, ctx: Ctx): Promise<Stream[]> {
  const { season, limit } = settings(m);
  const videoId = episodeMap(await feedVideos(m.id, ctx), season, limit).get(dubId)?.get(episode);
  if (!videoId) ctx.fail.notFound(`rutube: серии ${episode} нет`);
  return variants(await master(videoId, ctx)).map((v) => ({ url: v.url, kind: "hls" as const, quality: v.quality }));
}
