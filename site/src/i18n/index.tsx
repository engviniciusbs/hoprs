import { createContext, useContext } from "react";
import type { ReactNode } from "react";
import { en } from "./en";
import type { Dict } from "./en";
import { ptBr } from "./pt-br";

export type { Dict };
export type Lang = "en" | "pt-BR";

export const langs: readonly Lang[] = ["en", "pt-BR"];
export const dicts: Record<Lang, Dict> = { en, "pt-BR": ptBr };
/** Where each language's page lives on the site (and in dist/). */
export const pagePath: Record<Lang, string> = { en: "/", "pt-BR": "/pt-br/" };
export type Page = "home" | "privacy";
/** Every page, per language: where it lives on the site (and in dist/). */
export const pagePaths: Record<Page, Record<Lang, string>> = {
  home: pagePath,
  privacy: { en: "/privacy/", "pt-BR": "/pt-br/privacy/" },
};
export const otherLang = (lang: Lang): Lang => (lang === "en" ? "pt-BR" : "en");

export function isLang(value: string): value is Lang {
  return (langs as readonly string[]).includes(value);
}

const LangContext = createContext<Lang>("en");

export function LangProvider({ lang, children }: { lang: Lang; children: ReactNode }) {
  return <LangContext.Provider value={lang}>{children}</LangContext.Provider>;
}

export function useLang(): Lang {
  return useContext(LangContext);
}

export function useT(): Dict {
  return dicts[useContext(LangContext)];
}

/**
 * Numbers per locale. pt-BR groups with "." and uses a decimal comma; English keeps the page's
 * existing look, a non-breaking space between thousands ("100 000").
 */
export function formatNumber(lang: Lang, n: number): string {
  const text = new Intl.NumberFormat(lang === "en" ? "en-US" : "pt-BR", { maximumFractionDigits: 2 }).format(n);
  return lang === "en" ? text.replaceAll(",", " ") : text;
}

/** A calendar date (YYYY-MM-DD) in the page's language, e.g. "November 14, 2026" / "14 de novembro de 2026". */
export function formatDate(lang: Lang, iso: string): string {
  const [y, m, d] = iso.split("-").map(Number);
  return new Intl.DateTimeFormat(lang, { dateStyle: "long", timeZone: "UTC" }).format(new Date(Date.UTC(y, m - 1, d)));
}
