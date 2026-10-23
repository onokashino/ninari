// Kinogram (api.kinogram.best, a Collaps player) embed: one HLS master per episode with several audio tracks
// (dubs plus the original) on the interkh.com CDN. Shared by the catalog extensions.
import type { Ctx, FetchInit, Stream } from "../types";

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const KINOGRAM_HOSTS: string[] = ["*.kinogram.best", "*.interkh.com"];

const PAGE_TIMEOUT_MS = 8000;
/**
 * The CDN name resolves to five addresses, and now and then some of them stall new connections (seen
 * in 2026-09). The app's client gives each address 2 s of its 10 s connect timeout, so two stalled ones
 * cost 4 s before a live one answers.
 */
const MASTER_TIMEOUT_MS = 7000;
/** Audio track names change rarely; episodes() reads them from this cache. */
const AUDIO_TTL = 6 * 3600;
const EMBED_RE = /^(?:https?:)?\/\/((?:api\d*\.)?kinogram\.best)\/embed\/([a-z]+)\/(\d+)/i;

/** One video: its HLS master and audio track names in the master's order (the first one is its DEFAULT). */
export interface KinogramVideo {
  hls: string;
  audio: string[];
  blocked?: boolean;
}

/** A parsed embed page: episodes by "<season>:<episode>", or the single video of a movie. */
export interface KinogramPage {
  episodes: Record<string, KinogramVideo>;
  movie?: KinogramVideo;
}

export interface KinogramRef {
  /** The embed page without the episode choice: the same page lists every season. */
  page: string;
  season?: number;
  episode?: number;
}

interface RawAudio {
  names?: string[];
}

interface RawEpisode {
  episode?: string | number;
  hls?: string;
  audio?: RawAudio;
}

interface RawSeason {
  season?: number;
  blocked?: boolean;
  episodes?: RawEpisode[];
}

/** True when the URL belongs to this player. */
export function isKinogram(url: string): boolean {
  return EMBED_RE.test(url.trim());
}

export function parseKinogramUrl(url: string): KinogramRef | undefined {
  const u = url.trim().replace(/&amp;/g, "&");
  const m = EMBED_RE.exec(u);
  if (!m) return undefined;
  const num = (name: string) => {
    const v = new RegExp(`[?&]${name}=(\\d+)`).exec(u)?.[1];
    return v === undefined ? undefined : Number(v);
  };
  return { page: `https://${m[1].toLowerCase()}/embed/${m[2].toLowerCase()}/${m[3]}`, season: num("season"), episode: num("episode") };
}

/** The JSON value (array or object) that starts at `from`, cut out by bracket matching. */
function sliceJson(text: string, from: number): string | undefined {
  const open = text[from];
  if (open !== "[" && open !== "{") return undefined;
  let depth = 0;
  let inString = false;
  for (let i = from; i < text.length; i++) {
    const c = text[i];
    if (inString) {
      if (c === "\\") i++;
      else if (c === '"') inString = false;
    } else if (c === '"') inString = true;
    else if (c === "[" || c === "{") depth++;
    else if (c === "]" || c === "}") {
      depth--;
      if (depth === 0) return text.slice(from, i + 1);
    }
  }
  return undefined;
}

function jsonAfter<T>(text: string, re: RegExp): T | undefined {
  const m = re.exec(text);
  if (!m) return undefined;
  const raw = sliceJson(text, m.index + m[0].length);
  if (!raw) return undefined;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return undefined;
  }
}

/**
 * Track names as the master lists them. The master marks its first track DEFAULT=YES and that is what
 * mpv plays; `order` only tells the site's own player which one to pick (Jujutsu Kaisen: names
 * ["Japan Original", ..., "Anilibria"], order [2, 0, 1], the master starts with the Japanese track).
 */
function audioNames(a?: RawAudio): string[] {
  return (a?.names ?? []).filter((n): n is string => typeof n === "string" && n.trim() !== "");
}

/** Episodes of every season (series) or the one video (movie) of an embed page; null when it has neither. */
export function parseKinogram(html: string): KinogramPage | null {
  const seasons = jsonAfter<RawSeason[]>(html, /\bseasons\s*:\s*/);
  if (seasons) {
    const episodes: Record<string, KinogramVideo> = {};
    for (const s of seasons) {
      for (const e of s.episodes ?? []) {
        if (!e.hls || s.season === undefined || e.episode === undefined) continue;
        episodes[`${s.season}:${Number(e.episode)}`] = { hls: e.hls, audio: audioNames(e.audio), blocked: s.blocked || undefined };
      }
    }
    return { episodes };
  }
  const hls = /\bhls\s*:\s*"([^"]+)"/.exec(html)?.[1];
  if (!hls) return null;
  return { episodes: {}, movie: { hls, audio: audioNames(jsonAfter<RawAudio>(html, /\baudio\s*:\s*/)) } };
}

/** The video a reference points at; a series reference without a season means season 1. */
export function pickKinogram(page: KinogramPage, ref: KinogramRef): KinogramVideo | undefined {
  if (ref.episode === undefined) return page.movie;
  return page.episodes[`${ref.season ?? 1}:${ref.episode}`];
}

const TIERS = [240, 360, 480, 720, 1080, 1440, 2160];

/** The tallest rendition a master playlist lists, as a nominal tier (1920x800 counts as 1080). */
export function masterHeight(m3u8: string): number | undefined {
  let best = 0;
  for (const m of m3u8.matchAll(/RESOLUTION=\d+x(\d+)/g)) best = Math.max(best, Number(m[1]));
  return best > 0 ? (TIERS.find((t) => t >= best) ?? best) : undefined;
}

function tracksOf(parsed: KinogramPage): Record<string, string[]> {
  const out: Record<string, string[]> = {};
  for (const [k, v] of Object.entries(parsed.episodes)) out[k] = v.audio;
  if (parsed.movie) out.movie = parsed.movie.audio;
  return out;
}

async function fetchPage(page: string, ctx: Ctx): Promise<KinogramPage> {
  const init: FetchInit = { timeout: PAGE_TIMEOUT_MS };
  const res = await ctx.fetch(page, init);
  if (res.status === 404) ctx.fail.notFound(`kinogram: ${page} is gone`);
  if (!res.ok) throw new Error(`kinogram: HTTP ${res.status} for ${page}`);
  const parsed = parseKinogram(await res.text());
  if (!parsed) throw new Error(`kinogram: no playlist on ${page}`);
  await ctx.kv.set(`kinogram:audio:${page}`, JSON.stringify(tracksOf(parsed)), AUDIO_TTL);
  return parsed;
}

/** Key of a reference in a kinogramTracks map. */
export const kinogramKey = (ref: KinogramRef): string => (ref.episode === undefined ? "movie" : `${ref.season ?? 1}:${ref.episode}`);

/**
 * Audio track names of every video on an embed page (see KinogramRef.page), the default track first,
 * by kinogramKey. One page request per title, cached for hours.
 */
export async function kinogramTracks(page: string, ctx: Ctx): Promise<Record<string, string[]>> {
  const cached = await ctx.kv.get(`kinogram:audio:${page}`);
  if (cached) {
    try {
      return JSON.parse(cached) as Record<string, string[]>;
    } catch {
      // read the page again below
    }
  }
  return tracksOf(await fetchPage(page, ctx));
}

/** Audio track names of the video an embed URL points at, the default one first; empty when it is not there. */
export async function kinogramAudio(url: string, ctx: Ctx): Promise<string[]> {
  const ref = parseKinogramUrl(url);
  return ref ? ((await kinogramTracks(ref.page, ctx))[kinogramKey(ref)] ?? []) : [];
}

/**
 * Resolves a Kinogram embed (api.kinogram.best/embed/kp/<id>?season=<s>&episode=<e>, or without them for
 * a movie) into its HLS master. The master carries every audio track; players start on its DEFAULT one,
 * the first (see kinogramAudio). Its quality is read from the master itself (720p is common); when the
 * CDN does not answer in time the stream is still returned, without a quality. No headers needed.
 */
export async function resolveKinogram(embedUrl: string, ctx: Ctx): Promise<Stream[]> {
  const ref = parseKinogramUrl(embedUrl);
  if (!ref) throw new Error(`kinogram: unsupported url ${embedUrl}`);
  const video = pickKinogram(await fetchPage(ref.page, ctx), ref);
  if (!video) ctx.fail.notFound(`kinogram: no season ${ref.season ?? 1} episode ${ref.episode} on ${ref.page}`);
  if (video.blocked) ctx.fail.notFound(`kinogram: the season is blocked on ${ref.page}`);
  const init: FetchInit = { timeout: MASTER_TIMEOUT_MS };
  let res;
  try {
    res = await ctx.fetch(video.hls, init);
  } catch (e) {
    // The player's own connection may well get a live address: hand the stream over without a quality.
    ctx.log(`kinogram: master playlist did not answer: ${String(e)}`);
    return [{ url: video.hls, kind: "hls" }];
  }
  if (res.status === 404 || res.status === 410) ctx.fail.notFound(`kinogram: master playlist is gone`);
  if (!res.ok) throw new Error(`kinogram: master playlist answered HTTP ${res.status}`);
  const body = await res.text();
  if (!body.startsWith("#EXTM3U")) throw new Error("kinogram: master playlist is not HLS");
  return [{ url: video.hls, kind: "hls", quality: masterHeight(body) }];
}
