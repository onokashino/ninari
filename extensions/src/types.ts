// Extension API v1. Kept in sync with crates/core/src/extensions/types.rs in the app.

export interface Manifest {
  id: string;
  name: string;
  version: string;
  apiVersion: 1;
  /** Main domain first, then mirrors. ctx.baseUrl points at the one that works. */
  domains: string[];
  /** Extra hosts ctx.fetch may reach, exact or "*.suffix". */
  allowedHosts?: string[];
}

export interface SearchQuery {
  shikimoriId: number;
  malId: number;
  names: string[];
  year?: number;
  kind?: string;
  episodes?: number;
}

export interface Match {
  id: string;
  title: string;
  confidence: "exact" | "fuzzy";
  data?: unknown;
}

export interface Dub {
  id: string;
  name: string;
  type: "voice" | "subtitles" | "original";
}

export interface EpisodeList {
  dubs: Dub[];
  episodes: {
    number: number;
    available: { dubId: string; maxQuality?: number }[];
    skip?: { opening?: [number, number]; ending?: [number, number] };
  }[];
}

export interface Stream {
  url: string;
  kind: "hls" | "dash" | "mp4";
  quality?: number;
  headers?: Record<string, string>;
}

export interface FetchInit {
  method?: string;
  headers?: Record<string, string>;
  body?: string;
  /** Milliseconds before this request gives up (the app's default is 10 s). */
  timeout?: number;
}

export interface FetchResponse {
  status: number;
  url: string;
  ok: boolean;
  headers: Record<string, string>;
  text(): Promise<string>;
  json(): Promise<unknown>;
}

export interface HtmlNode {
  text: string;
  html: string;
  attrs: Record<string, string>;
}

export interface Ctx {
  baseUrl: string;
  fetch(url: string, init?: FetchInit): Promise<FetchResponse>;
  html: { select(html: string, css: string): HtmlNode[] };
  kv: {
    get(key: string): Promise<string | null>;
    set(key: string, value: string, ttlSec?: number): Promise<void>;
  };
  base64: { encode(s: string): string; decode(s: string): string };
  fail: {
    geoBlocked(msg?: string): never;
    notFound(msg?: string): never;
    challenge(url: string): never;
  };
  log(...args: unknown[]): void;
}
