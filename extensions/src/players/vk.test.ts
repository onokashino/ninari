// VK Video player: page parsing offline, resolution against the live site.
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import { isVk, parseVk, resolveVk, vkEmbedUrl, vkMessage, VK_HOSTS } from "./vk";

const ctx = makeCtx("https://vk.com");
// Frieren 1 and 5, subtitles, as Shikicinema lists them. The fifth was taken down on a rights holder's request.
const PLAYABLE = "https://vk.com/video_ext.php?oid=793268683&id=456240561&hash=429a0aa4be52242d";
const TAKEN_DOWN = "https://vk.com/video_ext.php?oid=793268683&id=456240729&hash=a2ee06106266748e";

describe("vk (offline)", () => {
  test("recognizes embed forms and keeps only what picks the video", () => {
    expect(isVk(PLAYABLE)).toBe(true);
    expect(isVk("http://vk.com/video_ext.php?oid=-12345&id=456239091&hash=da0d98c8a105e226")).toBe(true);
    expect(isVk("https://vk.com/video-12345_456239091")).toBe(false);
    expect(isVk("https://vk.com/video_ext.php?oid=1")).toBe(false);
    expect(vkEmbedUrl("http://vkvideo.ru/video_ext.php?oid=-1&amp;id=2&amp;hash=ab&amp;hd=2")).toBe(
      "https://vk.com/video_ext.php?oid=-1&id=2&hash=ab",
    );
    expect(VK_HOSTS).toEqual(expect.arrayContaining(["vk.com", "*.okcdn.ru"]));
  });

  test("reads the files of the current page layout, HLS first", () => {
    const html = `var playerParams = {"response":{"items":[{"files":{"mp4_240":"https:\\/\\/vkvd1.okcdn.ru\\/?type=0",
      "mp4_1080":"https:\\/\\/vkvd1.okcdn.ru\\/?type=5","hls":"https:\\/\\/vkvd1.okcdn.ru\\/video.m3u8?id=1",
      "dash_sep":"https:\\/\\/vkvd1.okcdn.ru\\/?type=1"}}]}};`;
    const streams = parseVk(html, {});
    expect(streams.map((s) => [s.kind, s.quality, s.url])).toEqual([
      ["hls", 1080, "https://vkvd1.okcdn.ru/video.m3u8?id=1"],
      ["mp4", 1080, "https://vkvd1.okcdn.ru/?type=5"],
      ["mp4", 240, "https://vkvd1.okcdn.ru/?type=0"],
    ]);
  });

  test("reads the older playerParams layout", () => {
    const html = `var playerParams = {"params":[{"url480":"https:\\/\\/cs1.vkuser.net\\/480.mp4","url720":"https:\\/\\/cs1.vkuser.net\\/720.mp4"}]};`;
    expect(parseVk(html, {}).map((s) => [s.kind, s.quality])).toEqual([
      ["mp4", 720],
      ["mp4", 480],
    ]);
  });

  test("reads the message shown instead of the player", () => {
    const html = `<div id="video_ext_msg">\nВидео недоступно в связи с обращением правообладателя\n</div>`;
    expect(vkMessage(html)).toBe("Видео недоступно в связи с обращением правообладателя");
    expect(parseVk(html, {})).toEqual([]);
  });
});

describe("vk (live)", () => {
  test("resolves an embed into HLS that answers with the browser agent", async () => {
    const streams = await resolveVk(PLAYABLE, ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080 });
    const res = await fetch(streams[0].url, { headers: streams[0].headers });
    expect(res.status).toBe(200);
    expect(await res.text()).toStartWith("#EXTM3U");
  }, 60_000);

  test("a taken down video is NotFound", async () => {
    await expect(resolveVk(TAKEN_DOWN, ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
