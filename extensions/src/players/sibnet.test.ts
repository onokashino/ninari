// Sibnet player: URL parsing offline, resolution against the live site.
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import { isSibnet, parseShell, resolveSibnet, SIBNET_HOSTS, videoId } from "./sibnet";

const ctx = makeCtx("https://video.sibnet.ru");
// Jigoku Shoujo 01 (AniDUB), the Sibnet source Anixart hands out for release 2.
const SHELL = "https://video.sibnet.ru/shell.php?videoid=2991867";

describe("sibnet (offline)", () => {
  test("recognizes embed forms", () => {
    expect(isSibnet(SHELL)).toBe(true);
    expect(isSibnet("//video.sibnet.ru/shell.php?videoid=1")).toBe(true);
    expect(isSibnet("https://video.sibnet.ru/video2991867-_AniDub_/")).toBe(true);
    expect(isSibnet("https://kodikplayer.com/seria/1/abc/720p")).toBe(false);
    expect(isSibnet("https://notsibnet.ru/shell.php?videoid=1")).toBe(false);
    expect(SIBNET_HOSTS).toContain("video.sibnet.ru");
  });

  test("extracts the video id", () => {
    expect(videoId(SHELL)).toBe("2991867");
    expect(videoId("https://video.sibnet.ru/video123-x/")).toBe("123");
    expect(videoId("https://video.sibnet.ru/v/abcdef0123/456.mp4")).toBe("456");
  });

  test("reads the mp4 path from the player call", () => {
    const html = `<script>player.src([{src: "/v/861c8a/2991867.mp4", type: "video/mp4"},]);</script>`;
    expect(parseShell(html)).toBe("https://video.sibnet.ru/v/861c8a/2991867.mp4");
    expect(parseShell("<div class=videostatus>x</div>")).toBeUndefined();
  });
});

describe("sibnet (live)", () => {
  test("resolves a shell page into an mp4 with a Referer", async () => {
    const [s] = await resolveSibnet(SHELL, ctx);
    expect(s.kind).toBe("mp4");
    expect(s.url).toMatch(/^https:\/\/video\.sibnet\.ru\/v\/[0-9a-f]+\/2991867\.mp4$/);
    expect(s.headers?.Referer).toContain("video.sibnet.ru");
    const head = await fetch(s.url, { headers: { ...s.headers, Range: "bytes=0-15" } });
    expect([200, 206]).toContain(head.status);
  }, 60_000);

  test("a missing video is NotFound", async () => {
    await expect(resolveSibnet("https://video.sibnet.ru/shell.php?videoid=999999999", ctx)).rejects.toThrow(
      "__EXT:NotFound",
    );
  }, 60_000);
});
