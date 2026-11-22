import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import { StrictMode } from "react";
import { hydrateRoot } from "react-dom/client";
import App from "./App";
import { isLang } from "./i18n";
import type { Page } from "./i18n";
import "./styles/app.css";

// The prerender set <html lang> per page (en at /, pt-BR at /pt-br/); hydrate in the same language.
const htmlLang = document.documentElement.lang;
const lang = isLang(htmlLang) ? htmlLang : "en";
// ...and the page (home or privacy) it wrote in data-page.
const page: Page = document.documentElement.dataset.page === "privacy" ? "privacy" : "home";

hydrateRoot(
  document.getElementById("root")!,
  <StrictMode>
    <App lang={lang} page={page} />
  </StrictMode>,
);
