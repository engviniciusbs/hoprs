import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import App from "./App";
import type { Lang } from "./i18n";
import { pagePaths } from "./i18n";

afterEach(cleanup);

const langs: Lang[] = ["en", "pt-BR"];

it.each(langs)("%s privacy page has one h1, six h2, the contact and the key facts", (lang) => {
  const { container } = render(<App lang={lang} page="privacy" />);
  expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
  expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(6);
  const mail = screen.getAllByRole("link").filter((a) => a.getAttribute("href") === "mailto:privacy@hoprs.online");
  expect(mail.length).toBeGreaterThan(0);
  const text = container.textContent ?? "";
  expect(text).toContain("Vinícius Sarmento");
  expect(text).toContain("drive.file");
  expect(text).toContain("Cloudflare Web Analytics");
  expect(text).toContain("https://myaccount.google.com/permissions");
  expect(text).toContain("Google API Services User Data Policy");
  expect(text).not.toContain("{");
});

it("the pt-BR h1 differs from the English one", () => {
  const h1 = (lang: Lang) => {
    const { unmount } = render(<App lang={lang} page="privacy" />);
    const text = screen.getByRole("heading", { level: 1 }).textContent;
    unmount();
    return text;
  };
  expect(h1("pt-BR")).not.toBe(h1("en"));
});

it.each([
  ["en", "/privacy/"],
  ["pt-BR", "/pt-br/privacy/"],
] as const)("the %s home footer links %s", (lang, href) => {
  render(<App lang={lang} />);
  const footer = screen.getByRole("contentinfo");
  expect(within(footer).getAllByRole("link").some((a) => a.getAttribute("href") === href)).toBe(true);
  expect(pagePaths.privacy[lang]).toBe(href);
});

it.each([
  ["en", "/pt-br/privacy/", "/#install"],
  ["pt-BR", "/privacy/", "/pt-br/#install"],
] as const)("the %s privacy nav links the other privacy page and the install section", (lang, other, install) => {
  render(<App lang={lang} page="privacy" />);
  const nav = screen.getByRole("banner");
  const hrefs = within(nav).getAllByRole("link").map((a) => a.getAttribute("href"));
  expect(hrefs).toContain(other);
  expect(hrefs).toContain(install);
});
