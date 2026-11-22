// Run after `vite build`: writes one page per language from dist/index.html (the template),
// with that language's head tags and the server-rendered App in #root, then sitemap.xml and llms.txt.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { render } from "../src/entry-server";
import { langs, pagePaths } from "../src/i18n";
import type { Page } from "../src/i18n";
import { facts } from "../src/facts";
import { headHtml, llmsTxt, sitemapXml } from "../src/seo";

const dist = new URL("../dist/", import.meta.url);
const template = readFileSync(new URL("index.html", dist), "utf8");

function swap(html: string, marker: string | RegExp, value: string): string {
  if (!(typeof marker === "string" ? html.includes(marker) : marker.test(html))) {
    throw new Error(`prerender: ${marker} not found in dist/index.html`);
  }
  return html.replace(marker, () => value);
}

const pages: Page[] = ["home", "privacy"];
for (const page of pages) {
  for (const lang of langs) {
    let html = template;
    const dataPage = page === "home" ? "" : ` data-page="${page}"`;
    html = swap(html, '<html lang="en">', `<html lang="${lang}"${dataPage}>`);
    // The fallback title and description (for `npm run dev`) give way to the page's own tags.
    html = swap(html, /\n\s*<title>hoprs<\/title>/, "");
    html = swap(html, /\n\s*<meta name="description" [^>]*>/, "");
    html = swap(html, /<!-- app-head:[^>]*-->/, headHtml(lang, page));
    html = swap(html, '<div id="root"></div>', `<div id="root">${render(lang, page)}</div>`);
    const dir = new URL(`.${pagePaths[page][lang]}`, dist);
    mkdirSync(dir, { recursive: true });
    writeFileSync(new URL("index.html", dir), html);
    console.log(`prerendered dist${pagePaths[page][lang]}index.html (${lang}, ${page})`);
  }
}

// lastmod: the current release's date, so a rebuild alone never claims new content.
writeFileSync(new URL("sitemap.xml", dist), sitemapXml(facts.releaseDate));
writeFileSync(new URL("llms.txt", dist), llmsTxt());
console.log("wrote dist/sitemap.xml and dist/llms.txt");
