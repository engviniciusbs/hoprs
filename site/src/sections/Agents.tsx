import { facts } from "../facts";
import { useT } from "../i18n";

export default function Agents() {
  const t = useT().agents;
  return (
    <section className="section container" aria-labelledby="agents-title">
      <h2 id="agents-title">{t.title}</h2>
      <p className="lead">{t.lead}</p>
      <pre className="well cmd install-pre">
        <span className="install-prompts" aria-hidden="true">
          <span>{" "}</span>
          <span>$</span>
        </span>
        <code>
          <span className="out-note"># {t.commandComment}</span>
          {"\n"}claude mcp add hoprs -- /usr/bin/hoprs mcp
        </code>
      </pre>
      <ul className="well out" aria-label={t.levelsLabel}>
        {t.levels.map((level) => (
          <li key={level.name}>
            <span className="out-value">{level.name}</span>
            <span className="out-note"># {level.note}</span>
          </li>
        ))}
      </ul>
      <p className="muted small">
        {t.note} <a href={facts.agentsDocUrl}>{t.how}</a>
      </p>
    </section>
  );
}
