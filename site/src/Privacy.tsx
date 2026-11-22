import { Fragment } from "react";
import type { ReactNode } from "react";
import { facts } from "./facts";
import { pagePaths, useLang, useT } from "./i18n";

const permissionsUrl = "https://myaccount.google.com/permissions";

// The dictionaries hold the text; the contact and the Google link become links here.
function fill(text: string): ReactNode {
  const filled = text.replaceAll("{responsible}", facts.responsible).replaceAll("{email}", facts.privacyEmail);
  return filled.split(new RegExp(`(${facts.privacyEmail}|${permissionsUrl})`)).map((part, i) => {
    if (part === facts.privacyEmail) return <a key={i} href={`mailto:${part}`}>{part}</a>;
    if (part === permissionsUrl) return <a key={i} href={part}>{part}</a>;
    return <Fragment key={i}>{part}</Fragment>;
  });
}

export default function Privacy() {
  const lang = useLang();
  const t = useT().privacy;
  return (
    <main className="section container privacy">
      <p>
        <a href={pagePaths.home[lang]}>{t.back}</a>
      </p>
      <h1>{t.title}</h1>
      <p className="muted">{t.updated}</p>
      {t.sections.map(({ heading, paragraphs }) => (
        <section key={heading} className="privacy-section">
          <h2>{heading}</h2>
          {paragraphs.map((p) => (
            <p key={p}>{fill(p)}</p>
          ))}
        </section>
      ))}
    </main>
  );
}
