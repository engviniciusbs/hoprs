// @vitest-environment node
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { facts } from "./facts";
import { dicts, pagePath, pagePaths } from "./i18n";
import type { Lang } from "./i18n";
import { downloadUrls, pageUrl } from "./seo";

const dist = new URL("../dist/", import.meta.url);
const built = existsSync(new URL("index.html", dist));
const read = (path: string) => readFileSync(new URL(path, dist), "utf8");

type Node = Record<string, unknown>;
function ldGraph(html: string): Node[] {
  const m = /<script type="application\/ld\+json">([\s\S]*?)<\/script>/.exec(html);
  expect(m, "JSON-LD block").toBeTruthy();
  const data = JSON.parse(m![1]) as { "@context": string; "@graph": Node[] };
  expect(data["@context"]).toBe("https://schema.org");
  return data["@graph"];
}

// Runs after `npm run build`. Without dist/ it is skipped, and the name says why.
describe.skipIf(!built).each([
  { lang: "en" as Lang, file: "index.html", ogLocale: "en_US", altLocale: "pt_BR" },
  { lang: "pt-BR" as Lang, file: "pt-br/index.html", ogLocale: "pt_BR", altLocale: "en_US" },
])("dist/$file (skipped when dist/ is missing: run npm run build first)", ({ lang, file, ogLocale, altLocale }) => {
  const t = dicts[lang];

  it("has the language, title, canonical and hreflang links", () => {
    const html = read(file);
    expect(html).toContain(`<html lang="${lang}">`);
    expect(html).toContain(`<title>${t.meta.title}</title>`);
    expect(html).toContain(`<link rel="canonical" href="${facts.siteUrl}${pagePath[lang]}" />`);
    expect(html).toContain(`<link rel="alternate" hreflang="en" href="https://hoprs.online/" />`);
    expect(html).toContain(`<link rel="alternate" hreflang="pt-BR" href="https://hoprs.online/pt-br/" />`);
    expect(html).toContain(`<link rel="alternate" hreflang="x-default" href="https://hoprs.online/" />`);
    expect(html).toContain(`<meta property="og:locale" content="${ogLocale}" />`);
    expect(html).toContain(`<meta property="og:locale:alternate" content="${altLocale}" />`);
    expect(html).toContain(`<meta property="og:image:width" content="1200" />`);
    expect(html).toContain(`<link rel="icon" href="/favicon.ico" sizes="32x32" />`);
    expect(html).toContain(`<link rel="apple-touch-icon" href="/apple-touch-icon.png" />`);
    expect(html).not.toContain("app-head");
    // The fallback title and description of index.html are gone: one of each, in this language.
    expect(html.match(/<title>/g)?.length).toBe(1);
    expect(html.match(/<meta name="description"/g)?.length).toBe(1);
    // Asset URLs stay absolute, so /pt-br/ loads the same bundle.
    expect(html).toMatch(/<script type="module" crossorigin src="\/assets\/[^"]+\.js"><\/script>/);
  });

  it("has the content in the HTML", () => {
    const html = read(file);
    expect(html).toContain(t.hero.headline);
    expect(html).toContain(`sudo apt install ./${facts.debName}`);
    // Every download is in the HTML, so a visitor without scripts can reach all of them.
    for (const url of downloadUrls) expect(html).toContain(`href="${url}"`);
    // The prerender lists Linux first: the .deb is the primary hero button.
    expect(html).toContain(`class="btn btn-primary" href="${facts.debUrl}"`);
    expect(html).toContain(".os-panel[hidden] { display: grid !important; }");
    const esc = (s: string) => s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
    for (const { q } of t.faq.items) expect(html).toContain(esc(q));
  });

  it("has valid JSON-LD that matches the page", () => {
    const html = read(file);
    const graph = ldGraph(html);
    expect(graph.map((n) => n["@type"]).sort()).toEqual(["FAQPage", "SoftwareApplication", "WebSite"]);
    const app = graph.find((n) => n["@type"] === "SoftwareApplication")!;
    expect(app.softwareVersion).toBe(facts.version);
    expect(app.downloadUrl).toEqual([facts.debUrl, facts.dmgUrl, facts.msiUrl, facts.zipUrl]);
    expect(app.operatingSystem).toBe("Linux, macOS, Windows");
    expect(app.isAccessibleForFree).toBe(true);
    expect(app).not.toHaveProperty("aggregateRating");
    expect(app.sameAs).toEqual([facts.repoUrl]);
    const faqPage = graph.find((n) => n["@type"] === "FAQPage")!;
    const rendered = html.match(/<details>/g)?.length;
    expect((faqPage.mainEntity as unknown[]).length).toBe(rendered);
    expect((faqPage.mainEntity as { name: string }[])[0].name).toBe(t.faq.items[0].q);
    // One WebSite and one app across both pages; the FAQ is per page and language.
    const site = graph.find((n) => n["@type"] === "WebSite")!;
    expect(site["@id"]).toBe("https://hoprs.online/#website");
    expect(site.url).toBe("https://hoprs.online/");
    expect(app["@id"]).toBe("https://hoprs.online/#app");
    expect(app.url).toBe("https://hoprs.online/");
    expect(app.description).toBe(t.meta.description);
    expect(faqPage.url).toBe(pageUrl(lang));
    expect(faqPage.inLanguage).toBe(lang);
  });
});

it.skipIf(!built)("robots.txt points to the sitemap; llms.txt names the downloads", () => {
  expect(read("robots.txt")).toContain("Sitemap: https://hoprs.online/sitemap.xml");
  const llms = read("llms.txt");
  expect(llms.startsWith("# hoprs\n")).toBe(true);
  expect(llms).toMatch(/^> .+/m);
  for (const url of downloadUrls) expect(llms).toContain(url);
  expect(llms).toContain("https://hoprs.online/pt-br/");
});

// The privacy pages: same checks for their own head tags and content.
describe.skipIf(!built).each([
  { lang: "en" as Lang, file: "privacy/index.html" },
  { lang: "pt-BR" as Lang, file: "pt-br/privacy/index.html" },
])("dist/$file (skipped when dist/ is missing: run npm run build first)", ({ lang, file }) => {
  const t = dicts[lang].privacy;
  const esc = (s: string) => s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");

  it("has the language, page marker, title, canonical and hreflang links", () => {
    const html = read(file);
    expect(html).toContain(`<html lang="${lang}" data-page="privacy">`);
    expect(html).toContain(`<title>${esc(t.metaTitle)}</title>`);
    expect(html).toContain(`<link rel="canonical" href="${facts.siteUrl}${pagePaths.privacy[lang]}" />`);
    expect(html).toContain(`<link rel="alternate" hreflang="en" href="https://hoprs.online/privacy/" />`);
    expect(html).toContain(`<link rel="alternate" hreflang="pt-BR" href="https://hoprs.online/pt-br/privacy/" />`);
    expect(html).toContain(`<link rel="alternate" hreflang="x-default" href="https://hoprs.online/privacy/" />`);
    expect(html).not.toContain("app-head");
    expect(html.match(/<title>/g)?.length).toBe(1);
    expect(html.match(/<meta name="description"/g)?.length).toBe(1);
  });

  it("has the content in the HTML", () => {
    const html = read(file);
    expect(html).toContain(`<h1>${esc(t.title)}</h1>`);
    expect(html).toContain("mailto:privacy@hoprs.online");
    expect(html).toContain("drive.file");
  });

  it("has JSON-LD of a WebSite and a WebPage only", () => {
    const graph = ldGraph(read(file));
    expect(graph.map((n) => n["@type"]).sort()).toEqual(["WebPage", "WebSite"]);
    const page = graph.find((n) => n["@type"] === "WebPage")!;
    expect(page.url).toBe(pageUrl(lang, "privacy"));
    expect(page.inLanguage).toBe(lang);
  });
});
