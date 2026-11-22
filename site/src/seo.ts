// Head tags, JSON-LD, sitemap.xml and llms.txt, built at prerender time from the dictionaries and
// facts.ts, so they never drift from the page. Pure functions: scripts/prerender.tsx writes them.
import { downloads, systems } from "./downloads";
import { facts } from "./facts";
import { dicts, formatDate, formatNumber, langs, otherLang, pagePaths } from "./i18n";
import type { Lang, Page } from "./i18n";

export const pageUrl = (lang: Lang, page: Page = "home") => `${facts.siteUrl}${pagePaths[page][lang]}`;
// One WebSite and one SoftwareApplication, whichever page describes them: both pages share these @ids.
export const siteId = `${facts.siteUrl}/#website`;
export const appId = `${facts.siteUrl}/#app`;
const abs = (path: string) => `${facts.siteUrl}${path}`;
const ogImage = { url: abs("/og.png"), width: 1200, height: 630 };
const shots = ["/shots/main.png", "/shots/split.png"];

const escapeHtml = (s: string) =>
  s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");

/** The four downloadable assets of the current release. */
export const downloadUrls = [facts.debUrl, facts.dmgUrl, facts.msiUrl, facts.zipUrl];

export function jsonLd(lang: Lang, page: Page = "home"): object {
  const t = dicts[lang];
  const url = pageUrl(lang, page);
  if (page === "privacy") {
    return {
      "@context": "https://schema.org",
      "@graph": [
        {
          "@type": "WebSite",
          "@id": siteId,
          name: "hoprs",
          url: pageUrl("en"),
          inLanguage: [...langs],
          description: t.meta.description,
        },
        {
          "@type": "WebPage",
          "@id": `${url}#webpage`,
          url,
          name: t.privacy.title,
          description: t.privacy.metaDescription,
          inLanguage: lang,
          isPartOf: { "@id": siteId },
        },
      ],
    };
  }
  const author = { "@type": "Person", name: facts.maintainer, url: facts.maintainerUrl, sameAs: [facts.maintainerUrl] };
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "WebSite",
        "@id": siteId,
        name: "hoprs",
        url: pageUrl("en"),
        inLanguage: [...langs],
        description: t.meta.description,
      },
      {
        "@type": "SoftwareApplication",
        "@id": appId,
        name: "hoprs",
        description: t.meta.description,
        url: pageUrl("en"),
        applicationCategory: "DeveloperApplication",
        operatingSystem: "Linux, macOS, Windows",
        softwareVersion: facts.version,
        datePublished: facts.releaseDate,
        downloadUrl: downloadUrls,
        installUrl: `${pageUrl("en")}#install`,
        releaseNotes: facts.changelogUrl,
        license: [facts.licenseUrl, facts.apacheLicenseUrl],
        isAccessibleForFree: true,
        offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
        image: ogImage.url,
        screenshot: [
          { "@type": "ImageObject", url: abs(shots[0]), caption: t.hero.shotAlt },
          { "@type": "ImageObject", url: abs(shots[1]), caption: t.connection.shotAlt },
        ],
        // The app's own interface is in English on every page.
        inLanguage: "en",
        author,
        publisher: author,
        sameAs: [facts.repoUrl],
      },
      {
        "@type": "FAQPage",
        "@id": `${url}#faq`,
        url,
        inLanguage: lang,
        isPartOf: { "@id": siteId },
        about: { "@id": appId },
        mainEntity: t.faq.items.map(({ q, a }) => ({
          "@type": "Question",
          name: q,
          acceptedAnswer: { "@type": "Answer", text: a },
        })),
      },
    ],
  };
}

/** Everything that goes in <head> after the static tags of index.html. */
export function headHtml(lang: Lang, page: Page = "home"): string {
  const t = dicts[lang];
  const other = otherLang(lang);
  const url = pageUrl(lang, page);
  const title = page === "privacy" ? t.privacy.metaTitle : t.meta.title;
  const description = page === "privacy" ? t.privacy.metaDescription : t.meta.description;
  const e = escapeHtml;
  // "<" inside a <script> data block could end it early; < is the same character to a JSON parser.
  const ld = JSON.stringify(jsonLd(lang, page)).replaceAll("<", "\\u003c");
  return [
    `<title>${e(title)}</title>`,
    `<meta name="description" content="${e(description)}" />`,
    `<link rel="canonical" href="${url}" />`,
    ...langs.map((l) => `<link rel="alternate" hreflang="${l}" href="${pageUrl(l, page)}" />`),
    `<link rel="alternate" hreflang="x-default" href="${pageUrl("en", page)}" />`,
    `<link rel="icon" href="/favicon.ico" sizes="32x32" />`,
    `<link rel="icon" type="image/svg+xml" href="/icon.svg" />`,
    `<link rel="apple-touch-icon" href="/apple-touch-icon.png" />`,
    `<meta property="og:type" content="website" />`,
    `<meta property="og:site_name" content="hoprs" />`,
    `<meta property="og:locale" content="${t.meta.ogLocale}" />`,
    `<meta property="og:locale:alternate" content="${dicts[other].meta.ogLocale}" />`,
    `<meta property="og:title" content="${e(title)}" />`,
    `<meta property="og:description" content="${e(description)}" />`,
    `<meta property="og:url" content="${url}" />`,
    `<meta property="og:image" content="${ogImage.url}" />`,
    `<meta property="og:image:width" content="${ogImage.width}" />`,
    `<meta property="og:image:height" content="${ogImage.height}" />`,
    `<meta property="og:image:alt" content="${e(t.meta.ogImageAlt)}" />`,
    `<meta name="twitter:card" content="summary_large_image" />`,
    `<meta name="twitter:title" content="${e(title)}" />`,
    `<meta name="twitter:description" content="${e(description)}" />`,
    `<meta name="twitter:image" content="${ogImage.url}" />`,
    `<meta name="twitter:image:alt" content="${e(t.meta.ogImageAlt)}" />`,
    `<script type="application/ld+json">${ld}</script>`,
  ].join("\n    ");
}

/** dist/sitemap.xml: every page (home, then privacy), each with its own hreflang alternates. */
export function sitemapXml(lastmod: string): string {
  const pages: Page[] = ["home", "privacy"];
  const urls = pages
    .flatMap((page) => {
      const alternates = [
        ...langs.map((l) => `    <xhtml:link rel="alternate" hreflang="${l}" href="${pageUrl(l, page)}"/>`),
        `    <xhtml:link rel="alternate" hreflang="x-default" href="${pageUrl("en", page)}"/>`,
      ].join("\n");
      return langs.map(
        (l) => `  <url>\n    <loc>${pageUrl(l, page)}</loc>\n    <lastmod>${lastmod}</lastmod>\n${alternates}\n  </url>`,
      );
    })
    .join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
${urls}
</urlset>
`;
}

/** dist/llms.txt (https://llmstxt.org): the page's facts in plain Markdown, English. */
export function llmsTxt(): string {
  const t = dicts.en;
  const { perf } = facts;
  const n = (v: number) => formatNumber("en", v).replaceAll(" ", " ");
  const leads: Record<string, string> = { linux: t.install.linuxLead, macos: t.install.macLead, windows: t.install.windowsLead };
  return `# hoprs

> ${t.meta.description}

${t.hero.headline} ${t.hero.sentence} Version ${facts.version}, released ${facts.releaseDate}. ${t.openSource.lead}

## Downloads (version ${facts.version})

${systems.map((os) => `- [${downloads[os].name} ${downloads[os].ext}](${downloads[os].href}): ${leads[os]}`).join("\n")}
- [Windows portable .zip](${facts.zipUrl}): unzip anywhere, no installer.
- [SHA256SUMS](${facts.sumsUrl}): checksums for every file above.
- Linux install: \`sudo apt install ./${facts.debName}\`. macOS and Windows builds are not notarized or signed with a paid certificate; the page explains how to open them.

## Features

- ${t.connection.lead}
- SSH with a terminal; files over SFTP, FTP and FTPS (explicit and implicit). Plain FTP is allowed and labelled ftp (unencrypted).
- Passwords are kept in the system keyring: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows. Site files hold no secrets.
- No accounts and no data collection: it talks only to the servers you add.
- Measured on a GitHub Actions runner (${formatDate("en", perf.measuredOn)}): ${n(perf.startupMs)} ms to the first frame, ${n(perf.idleMb)} MB memory at idle, ${n(perf.rowsP95Ms)} ms p95 view and layout for ${n(perf.rows)} rows, SFTP 1 GiB at 50 ms RTT at ${n(perf.sftpVsScp)} x scp. Method: ${facts.perfDocUrl}

## Compared with FileZilla

- Kept: ${t.filezilla.kept}.
${t.filezilla.added.map((line) => `- Added: ${line}.`).join("\n")}

## Questions

${t.faq.items.map(({ q, a }) => `- ${q} ${a}`).join("\n")}

## Licence

MIT OR Apache-2.0: ${facts.licenseUrl} and ${facts.apacheLicenseUrl}

## Links

- [Source code on GitHub](${facts.repoUrl})
- [Releases](${facts.releasesUrl})
- [Changelog](${facts.changelogUrl})
- [Security](${facts.securityUrl})
- [Privacy policy](${pageUrl("en", "privacy")})
- [Página em português](${pageUrl("pt-BR")})
`;
}
