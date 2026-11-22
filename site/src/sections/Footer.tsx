import { facts } from "../facts";
import { formatDate, pagePaths, useLang, useT } from "../i18n";

export default function Footer() {
  const lang = useLang();
  const dict = useT();
  const t = dict.footer;
  return (
    <footer className="footer">
      <div className="container footer-row">
        <a className="wordmark" href={pagePaths.home[lang]} aria-label={dict.nav.home}>
          <img src="/icon.svg" alt="" width="24" height="24" />
          <span>hoprs</span>
        </a>
        <nav aria-label={t.label} className="nav-links">
          <a href={facts.repoUrl}>GitHub</a>
          <a href={facts.releasesUrl}>{t.releases}</a>
          <a href={facts.changelogUrl}>{t.changelog}</a>
          <a href={facts.securityUrl}>{t.security}</a>
          <a href={facts.licenseUrl}>{t.license}</a>
          <a href={pagePaths.privacy[lang]}>{t.privacy}</a>
        </nav>
        <p className="muted small">
          {t.licensed} {t.madeBy} <a href={facts.maintainerUrl}>{facts.maintainer}</a>. {t.updated}{" "}
          <time dateTime={facts.releaseDate}>{formatDate(lang, facts.releaseDate)}</time>.
        </p>
      </div>
    </footer>
  );
}
