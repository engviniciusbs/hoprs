import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import App from "../App";
import { facts } from "../facts";
import { dicts, langs } from "../i18n";

afterEach(cleanup);

it.each(langs)("%s: the agents section has the command, the three levels and the link", (lang) => {
  const t = dicts[lang].agents;
  render(<App lang={lang} />);
  const section = screen.getByRole("heading", { level: 2, name: t.title }).closest("section")!;
  expect(section.getAttribute("aria-labelledby")).toBe("agents-title");
  expect(section.textContent).toContain(t.lead);
  expect(section.querySelector("code")!.textContent).toContain('claude mcp add hoprs -- "/usr/bin/hoprs" mcp');
  expect(section.querySelector("code")!.textContent).toContain(t.commandComment);
  const items = within(within(section).getByRole("list", { name: t.levelsLabel })).getAllByRole("listitem");
  expect(items).toHaveLength(3);
  t.levels.forEach((level, i) => {
    expect(items[i].textContent).toContain(level.name);
    expect(items[i].textContent).toContain(level.note);
  });
  expect(within(section).getByRole("link", { name: t.how }).getAttribute("href")).toBe(facts.agentsDocUrl);
});

it("sits between the connection and FileZilla sections", () => {
  render(<App />);
  const ids = screen.getAllByRole("heading", { level: 2 }).map((h) => h.id);
  expect(ids.indexOf("agents-title")).toBe(ids.indexOf("conn-title") + 1);
  expect(ids.indexOf("fz-title")).toBe(ids.indexOf("agents-title") + 1);
});
