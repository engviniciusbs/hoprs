import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, it } from "vitest";
import { facts } from "./facts";

const file = resolve(process.cwd(), "dist/sitemap.xml");
const XHTML = "http://www.w3.org/1999/xhtml";

// Runs after `npm run build` (jsdom's DOMParser does the XML parsing). Skipped without dist/.
it.skipIf(!existsSync(file))("dist/sitemap.xml parses and lists the home and privacy pages with hreflang alternates", () => {
  const doc = new DOMParser().parseFromString(readFileSync(file, "utf8"), "application/xml");
  expect(doc.getElementsByTagName("parsererror").length).toBe(0);
  expect(doc.documentElement.localName).toBe("urlset");
  const urls = [...doc.getElementsByTagName("url")];
  expect(urls.map((u) => u.getElementsByTagName("loc")[0]?.textContent)).toEqual([
    "https://hoprs.online/",
    "https://hoprs.online/pt-br/",
    "https://hoprs.online/privacy/",
    "https://hoprs.online/pt-br/privacy/",
  ]);
  const pairs = [
    ["https://hoprs.online/", "https://hoprs.online/pt-br/"],
    ["https://hoprs.online/privacy/", "https://hoprs.online/pt-br/privacy/"],
  ];
  for (const [i, u] of urls.entries()) {
    const [en, pt] = pairs[Math.floor(i / 2)];
    expect(u.getElementsByTagName("lastmod")[0]?.textContent).toBe(facts.releaseDate);
    const links = [...u.getElementsByTagNameNS(XHTML, "link")].map((l) => [l.getAttribute("hreflang"), l.getAttribute("href")]);
    expect(links).toEqual([
      ["en", en],
      ["pt-BR", pt],
      ["x-default", en],
    ]);
  }
});
