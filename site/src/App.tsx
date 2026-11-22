import { LangProvider } from "./i18n";
import type { Lang, Page } from "./i18n";
import Connection from "./sections/Connection";
import Faq from "./sections/Faq";
import Fast from "./sections/Fast";
import Filezilla from "./sections/Filezilla";
import Footer from "./sections/Footer";
import Hero from "./sections/Hero";
import Install from "./sections/Install";
import Nav from "./sections/Nav";
import OpenSource from "./sections/OpenSource";
import Privacy from "./Privacy";

export default function App({ lang = "en", page = "home" }: { lang?: Lang; page?: Page }) {
  return (
    <LangProvider lang={lang}>
      <Nav page={page} />
      {page === "privacy" ? (
        <Privacy />
      ) : (
        <main>
          <Hero />
          <Fast />
          <Connection />
          <Filezilla />
          <OpenSource />
          <Install />
          <Faq />
        </main>
      )}
      <Footer />
    </LangProvider>
  );
}
