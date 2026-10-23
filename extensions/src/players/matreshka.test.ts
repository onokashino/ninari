// Matreshka TV player: page parsing offline, resolution against the live site.
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import { isMatreshka, MATRESHKA_HOSTS, matreshkaId, parseMatreshka, resolveMatreshka } from "./matreshka";

const ctx = makeCtx("https://matreshka.tv");
// One Piece 1179, Ziggy Team subtitles, as Shikicinema lists it.
const EMBED = "https://matreshka.tv/embed/video/pwMABfNt8wg";

/** A master URL the way the page writes it: slashes as /, the rendition spec in base64. */
const master = (id: string, quality: number[], codec = "h264") =>
  `https:\\u002F\\u002Fc4-video.cmtv.ru\\u002Fhm\\u002FAAB\\u002F${btoa(JSON.stringify({ id, codec, userQuality: quality }))}\\u002Fmaster.m3u8?expires=1&md5=x`;

describe("matreshka (offline)", () => {
  test("recognizes embed forms", () => {
    expect(isMatreshka(EMBED)).toBe(true);
    expect(isMatreshka("https://matreshka.tv/video/pwMABfNt8wg")).toBe(true);
    expect(isMatreshka("https://matreshka.tv/channel/abc")).toBe(false);
    expect(matreshkaId(EMBED)).toBe("pwMABfNt8wg");
    expect(MATRESHKA_HOSTS).toContain("*.cmtv.ru");
  });

  test("adaptive master first, then single renditions best first; other videos and AV1 skipped", () => {
    const html = `dy.video_url={h264:{src:{hls:"${master("abc", [])}"}}};dy.abr={h264:{playlists:{
      "360":"${master("abc", [360])}","1080":"${master("abc", [1080])}"},av1:{playlists:{"1080":"${master("abc", [1080], "av1")}"}}}};
      related:[{hls:"${master("other", [])}"}]`;
    const streams = parseMatreshka(html, (s) => atob(s), "abc");
    expect(streams.map((s) => s.quality)).toEqual([1080, 1080, 360]);
    expect(streams.every((s) => s.kind === "hls" && s.url.startsWith("https://c4-video.cmtv.ru/hm/"))).toBe(true);
    expect(atob(streams[0].url.split("/")[5])).toContain('"userQuality":[]');
    expect(parseMatreshka("<html></html>", (s) => atob(s))).toEqual([]);
  });
});

describe("matreshka (live)", () => {
  test("resolves an embed into a playable adaptive master", async () => {
    const streams = await resolveMatreshka(EMBED, ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080 });
    const res = await fetch(streams[0].url);
    expect(res.status).toBe(200);
    expect(await res.text()).toContain("RESOLUTION=1920x1080");
  }, 60_000);

  test("an unknown video is NotFound", async () => {
    await expect(resolveMatreshka("https://matreshka.tv/embed/video/zzzzzzzzzzz", ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
