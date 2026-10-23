// Sibnet player: turns an embed URL into playable streams. Shared by the catalog extensions.
import type { Ctx, Stream } from "../types";

const ORIGIN = "https://video.sibnet.ru";
/** Sibnet refuses curl-like agents, a browser one is always accepted. */
const UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/** Hosts the extensions using this player must list in manifest.allowedHosts. */
export const SIBNET_HOSTS: string[] = ["video.sibnet.ru", "*.sibnet.ru"];

const HOST_RE = /^(?:https?:)?\/\/([^/?#:]+)/i;
/** shell.php?videoid=N, /videoN-slug/ and the direct /v/<hash>/N.mp4 file. */
const ID_RES = [/[?&]videoid=(\d+)/i, /\/video(\d+)/i, /\/v\/[0-9a-z]+\/(\d+)\.mp4/i];
const SRC_RE = /player\.src\(\s*\[\s*\{\s*src\s*:\s*["']([^"']+\.mp4[^"']*)["']/i;
/** Status block of a missing or removed video. Its text is windows-1251, which the host does not decode. */
const GONE_RE = /class=["']?videostatus/i;

/** True when the URL belongs to this player. */
export function isSibnet(url: string): boolean {
  const host = HOST_RE.exec(url.trim())?.[1].toLowerCase() ?? "";
  return (host === "sibnet.ru" || host.endsWith(".sibnet.ru")) && ID_RES.some((re) => re.test(url));
}

export function videoId(url: string): string | undefined {
  for (const re of ID_RES) {
    const id = re.exec(url)?.[1];
    if (id) return id;
  }
  return undefined;
}

/** The MP4 path from a shell page, absolute. */
export function parseShell(html: string): string | undefined {
  const src = SRC_RE.exec(html)?.[1];
  if (!src) return undefined;
  if (/^https?:\/\//i.test(src)) return src;
  return src.startsWith("//") ? `https:${src}` : `${ORIGIN}${src.startsWith("/") ? "" : "/"}${src}`;
}

/**
 * Resolves a Sibnet embed (shell.php?videoid=, /videoN page or a /v/ file link) into one MP4.
 * The /v/ link answers 403 without a Sibnet Referer and redirects to a signed dvNN.sibnet.ru file,
 * so the Referer travels with the stream.
 */
export async function resolveSibnet(embedUrl: string, ctx: Ctx): Promise<Stream[]> {
  const id = videoId(embedUrl);
  if (!id) throw new Error(`sibnet: no video id in ${embedUrl}`);
  const shell = `${ORIGIN}/shell.php?videoid=${id}`;
  const headers = { Referer: `${ORIGIN}/` };
  const res = await ctx.fetch(shell, { headers: { ...headers, "User-Agent": UA } });
  const html = await res.text();
  const url = res.ok ? parseShell(html) : undefined;
  if (url) return [{ url, kind: "mp4", headers }];
  if (res.status === 404 || GONE_RE.test(html)) ctx.fail.notFound(`sibnet: video ${id} is gone`);
  throw new Error(`sibnet: no player source on ${shell} (HTTP ${res.status})`);
}
