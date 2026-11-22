import { renderToString } from "react-dom/server";
import App from "./App";
import type { Lang, Page } from "./i18n";

export function render(lang: Lang, page: Page = "home"): string {
  return renderToString(<App lang={lang} page={page} />);
}
