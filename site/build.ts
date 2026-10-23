// Builds the site into dist/: the pages of src/pages and the docs of src/docs, in Russian at the
// root and in English under en/, with one header, footer and docs layout for all of them.
//
//   bun site/build.ts          (from anywhere)
//
// No dependencies: the docs use a small Markdown subset, rendered below.
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const SITE = import.meta.dir;
const SRC = join(SITE, "src");
const DIST = join(SITE, "dist");
const REPO = "https://github.com/onokashino/ninari";

type Lang = "ru" | "en";
const LANGS: Lang[] = ["ru", "en"];

const T = {
  ru: {
    home: "ninari, на главную",
    sections: "Разделы",
    features: "Возможности",
    plugins: "Плагины",
    docs: "Документация",
    faq: "Вопросы",
    download: "Скачать",
    github: "ninari на GitHub",
    menu: "Меню",
    language: "Язык",
    other: "English",
    issues: "Сообщить о проблеме",
    guide: "Пользователям",
    dev: "Разработчикам",
    onPage: "На этой странице",
    prev: "Назад",
    next: "Дальше",
    edit: "Исправить на GitHub",
    docsTitle: "Документация ninari",
  },
  en: {
    home: "ninari, home",
    sections: "Sections",
    features: "Features",
    plugins: "Plugins",
    docs: "Docs",
    faq: "FAQ",
    download: "Download",
    github: "ninari on GitHub",
    menu: "Menu",
    language: "Language",
    other: "Русский",
    issues: "Report a problem",
    guide: "Using ninari",
    dev: "Developers",
    onPage: "On this page",
    prev: "Previous",
    next: "Next",
    edit: "Edit on GitHub",
    docsTitle: "ninari docs",
  },
} as const;

const GITHUB_ICON =
  '<svg viewBox="0 0 16 16" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>';

// ---------- the page shell ----------

interface Shell {
  lang: Lang;
  /** Path inside the language's folder, like "index.html" or "docs/player.html". */
  path: string;
  title: string;
  description: string;
  /** Which header link is the current page: "plugins", "docs" or none. */
  current?: string;
  body: string;
  bodyClass?: string;
}

/** "../" as many times as the page sits below the site's root. */
function rootOf(lang: Lang, path: string): string {
  const depth = path.split("/").length - 1 + (lang === "en" ? 1 : 0);
  return "../".repeat(depth);
}

function shell(p: Shell): string {
  const t = T[p.lang];
  const root = rootOf(p.lang, p.path);
  const here = p.lang === "en" ? `${root}en/` : root;
  const otherLang: Lang = p.lang === "ru" ? "en" : "ru";
  const other = otherLang === "en" ? `${root}en/${p.path}` : `${root}${p.path}`;
  const cur = (key: string) => (p.current === key ? ' aria-current="page"' : "");
  const links = [
    [`${here}index.html#features`, t.features, ' data-spy="features"'],
    [`${here}plugins.html`, t.plugins, cur("plugins")],
    [`${here}docs/index.html`, t.docs, cur("docs")],
    [`${here}index.html#faq`, t.faq, ' data-spy="faq"'],
  ];
  return `<!doctype html>
<html lang="${p.lang}">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${p.title}</title>
    <meta name="description" content="${p.description}" />
    <meta name="theme-color" content="#101117" />
    <link rel="icon" href="${root}assets/favicon.svg" type="image/svg+xml" />
    <link rel="alternate" hreflang="${otherLang}" href="${other}" />
    <link rel="preload" href="${root}assets/fonts/manrope-cyrillic-wght-normal.woff2" as="font" type="font/woff2" crossorigin />
    <link rel="stylesheet" href="${root}style.css" />
    <script>
      document.documentElement.classList.add("js");
    </script>
    <script src="${root}app.js" defer></script>
  </head>
  <body${p.bodyClass ? ` class="${p.bodyClass}"` : ""}>
    <header class="bar">
      <div class="bar-in">
        <a class="brand" href="${here}index.html" aria-label="${t.home}">
          <img src="${root}assets/mark.svg" alt="" width="30" height="30" />
          <span>ninari</span>
        </a>
        <nav class="nav" aria-label="${t.sections}">
          <span class="nav-glow" aria-hidden="true"></span>
          ${links.map(([href, label, extra]) => `<a href="${href}"${extra}>${label}</a>`).join("\n          ")}
        </nav>
        <div class="bar-end">
          <div class="lang" aria-label="${t.language}">
            ${
              p.lang === "ru"
                ? `<a aria-current="true">RU</a><a href="${other}" lang="en" hreflang="en">EN</a>`
                : `<a href="${other}" lang="ru" hreflang="ru">RU</a><a aria-current="true">EN</a>`
            }
          </div>
          <a class="icon-link" href="${REPO}" aria-label="${t.github}">${GITHUB_ICON}</a>
          <a class="button small" href="${here}download.html">${t.download}</a>
          <button class="menu-toggle" type="button" aria-expanded="false" aria-controls="menu" aria-label="${t.menu}"><i></i><i></i></button>
        </div>
      </div>
      <nav class="menu-sheet" id="menu" aria-label="${t.menu}" hidden>
        ${links.map(([href, label]) => `<a href="${href}">${label}</a>`).join("\n        ")}
        <a href="${here}download.html">${t.download}</a>
        <a href="${REPO}">GitHub</a>
        <a href="${other}" lang="${otherLang}" hreflang="${otherLang}">${t.other}</a>
      </nav>
    </header>

${p.body.replaceAll("{{root}}", root).replaceAll("{{here}}", here).replaceAll("{{github}}", GITHUB_ICON)}

    <footer class="foot">
      <span>ninari</span>
      <a href="${here}download.html">${t.download}</a>
      <a href="${here}docs/index.html">${t.docs}</a>
      <a href="${here}plugins.html">${t.plugins}</a>
      <a href="${REPO}">GitHub</a>
      <a href="${REPO}/issues">${t.issues}</a>
    </footer>
    <div class="scrollbar" aria-hidden="true"><i></i></div>
  </body>
</html>
`;
}

// ---------- pages ----------

/** A page source starts with a comment holding its title and description as JSON. */
function readPage(file: string): { meta: { title: string; description: string; current?: string }; body: string } {
  const text = readFileSync(file, "utf8");
  const m = text.match(/^<!--\s*(\{[\s\S]*?\})\s*-->\n?/);
  if (!m) throw new Error(`${file}: no meta comment`);
  return { meta: JSON.parse(m[1]), body: text.slice(m[0].length) };
}

// ---------- Markdown ----------

const escape = (s: string) => s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");

function slug(s: string): string {
  return s
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .trim()
    .replace(/\s+/g, "-");
}

/** Inline marks: `code`, **bold**, *italic*, [link](url) and raw tags like <kbd> pass through. */
function inline(s: string): string {
  const codes: string[] = [];
  s = s.replace(/`([^`]+)`/g, (_, c) => `\u0000${codes.push(`<code>${escape(c)}</code>`) - 1}\u0000`);
  s = s
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>")
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, text, href) => {
      const external = /^https?:/.test(href) ? ' rel="noopener"' : "";
      return `<a href="${href}"${external}>${text}</a>`;
    });
  return s.replace(/\u0000(\d+)\u0000/g, (_, i) => codes[Number(i)]);
}

interface Heading {
  level: number;
  text: string;
  id: string;
}

function markdown(src: string): { html: string; headings: Heading[] } {
  const lines = src.replace(/\r/g, "").split("\n");
  const out: string[] = [];
  const headings: Heading[] = [];
  let i = 0;
  const para: string[] = [];
  const flush = () => {
    if (para.length) out.push(`<p>${inline(para.join(" "))}</p>`);
    para.length = 0;
  };
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^```(\w*)/);
    if (fence) {
      flush();
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) code.push(lines[i++]);
      i++;
      out.push(
        `<div class="code"><pre><code${fence[1] ? ` data-lang="${fence[1]}"` : ""}>${escape(code.join("\n"))}</code></pre><button type="button" class="copy" data-copy-code></button></div>`,
      );
      continue;
    }
    const h = line.match(/^(#{1,3})\s+(.*)$/);
    if (h) {
      flush();
      const level = h[1].length;
      const id = slug(h[2]);
      headings.push({ level, text: h[2].replace(/<[^>]+>/g, ""), id });
      out.push(`<h${level} id="${id}">${inline(h[2])}</h${level}>`);
      i++;
      continue;
    }
    if (/^\|/.test(line)) {
      flush();
      const rows: string[][] = [];
      while (i < lines.length && /^\|/.test(lines[i])) {
        rows.push(lines[i].replace(/^\||\|$/g, "").split("|").map((c) => c.trim()));
        i++;
      }
      const [head, , ...body] = rows;
      out.push(
        `<div class="table"><table><thead><tr>${head.map((c) => `<th>${inline(c)}</th>`).join("")}</tr></thead><tbody>${body
          .map((r) => `<tr>${r.map((c) => `<td>${inline(c)}</td>`).join("")}</tr>`)
          .join("")}</tbody></table></div>`,
      );
      continue;
    }
    const list = line.match(/^(\s*)([-*]|\d+\.)\s+(.*)$/);
    if (list) {
      flush();
      const ordered = /\d/.test(list[2]);
      const items: string[] = [];
      while (i < lines.length) {
        const m = lines[i].match(/^\s*([-*]|\d+\.)\s+(.*)$/);
        if (m) items.push(m[2]);
        else if (/^\s{2,}\S/.test(lines[i]) && items.length) items[items.length - 1] += ` ${lines[i].trim()}`;
        else break;
        i++;
      }
      const tag = ordered ? "ol" : "ul";
      out.push(`<${tag}>${items.map((it) => `<li>${inline(it)}</li>`).join("")}</${tag}>`);
      continue;
    }
    const quote = line.match(/^>\s?(.*)$/);
    if (quote) {
      flush();
      const parts: string[] = [];
      while (i < lines.length && /^>/.test(lines[i])) parts.push(lines[i++].replace(/^>\s?/, ""));
      out.push(`<div class="note">${inline(parts.join(" "))}</div>`);
      continue;
    }
    if (line.trim() === "") flush();
    else para.push(line.trim());
    i++;
  }
  flush();
  return { html: out.join("\n"), headings };
}

// ---------- docs ----------

interface Doc {
  slug: string;
  title: string;
  description: string;
  section: "guide" | "dev";
  order: number;
  html: string;
  headings: Heading[];
}

function readDocs(lang: Lang): Doc[] {
  const dir = join(SRC, "docs", lang);
  return readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .map((f) => {
      const text = readFileSync(join(dir, f), "utf8");
      const m = text.match(/^---\n([\s\S]*?)\n---\n/);
      if (!m) throw new Error(`${lang}/${f}: no front matter`);
      const meta = Object.fromEntries(
        m[1].split("\n").map((l) => {
          const at = l.indexOf(":");
          return [l.slice(0, at).trim(), l.slice(at + 1).trim()];
        }),
      );
      const { html, headings } = markdown(text.slice(m[0].length));
      return {
        slug: f.replace(/^\d+-/, "").replace(/\.md$/, ""),
        title: meta.title,
        description: meta.description ?? meta.title,
        section: meta.section === "dev" ? "dev" : "guide",
        order: Number(f.match(/^(\d+)/)?.[1] ?? 99),
        html,
        headings,
      } as Doc;
    })
    .sort((a, b) => a.order - b.order);
}

function docPage(lang: Lang, docs: Doc[], doc: Doc): string {
  const t = T[lang];
  const at = docs.indexOf(doc);
  const prev = docs[at - 1];
  const next = docs[at + 1];
  const group = (section: Doc["section"], label: string) => `
          <p class="docs-group">${label}</p>
          ${docs
            .filter((d) => d.section === section)
            .map((d) => `<a href="${d.slug}.html"${d === doc ? ' aria-current="page"' : ""}>${d.title}</a>`)
            .join("\n          ")}`;
  const toc = doc.headings.filter((h) => h.level === 2);
  const source = `${REPO}/blob/master/site/src/docs/${lang}/${String(doc.order).padStart(2, "0")}-${doc.slug}.md`;
  const body = `    <main class="docs" id="top">
      <aside class="docs-nav" aria-label="${t.docs}">
        <nav>${group("guide", t.guide)}${group("dev", t.dev)}
        </nav>
      </aside>
      <article class="docs-body">
${doc.html}
        <nav class="docs-pager">
          ${prev ? `<a class="prev" href="${prev.slug}.html"><small>${t.prev}</small>${prev.title}</a>` : "<span></span>"}
          ${next ? `<a class="next" href="${next.slug}.html"><small>${t.next}</small>${next.title}</a>` : "<span></span>"}
        </nav>
        <p class="docs-edit"><a href="${source}">${t.edit}</a></p>
      </article>
      <aside class="docs-toc" aria-label="${t.onPage}">
        ${
          toc.length > 1
            ? `<p class="docs-group">${t.onPage}</p>\n        ${toc.map((h) => `<a href="#${h.id}">${h.text}</a>`).join("\n        ")}`
            : ""
        }
      </aside>
    </main>`;
  return shell({
    lang,
    path: `docs/${doc.slug}.html`,
    title: doc.slug === "index" ? t.docsTitle : `${doc.title} · ${t.docsTitle}`,
    description: doc.description,
    current: "docs",
    body,
    bodyClass: "docs-page",
  });
}

// ---------- build ----------

function write(lang: Lang, path: string, html: string) {
  const file = join(DIST, lang === "en" ? "en" : "", path);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, html);
}

rmSync(DIST, { recursive: true, force: true });
mkdirSync(DIST, { recursive: true });
for (const item of ["style.css", "app.js", "assets"]) cpSync(join(SITE, item), join(DIST, item), { recursive: true });

let pages = 0;
for (const lang of LANGS) {
  const dir = join(SRC, "pages", lang);
  for (const file of readdirSync(dir).filter((f) => f.endsWith(".html"))) {
    const { meta, body } = readPage(join(dir, file));
    write(lang, file, shell({ lang, path: file, ...meta, body }));
    pages++;
  }
  const docs = readDocs(lang);
  for (const doc of docs) {
    write(lang, `docs/${doc.slug}.html`, docPage(lang, docs, doc));
    pages++;
  }
}
// GitHub Pages serves the folder as is: no Jekyll.
writeFileSync(join(DIST, ".nojekyll"), "");
if (!existsSync(join(DIST, "index.html"))) throw new Error("no index.html");
console.log(`site: ${pages} pages in ${DIST}`);
