import { describe, expect, test } from "bun:test";

import {
  findLiterals,
  newViolations,
  staleBaseline,
  toBaseline,
  type Baseline,
} from "./literals";

/** shorthand: the texts a scan found, in order */
function texts(source: string): string[] {
  return findLiterals(source, "f.tsx").map((l) => l.text);
}

describe("findLiterals", () => {
  test("catches the #871 regression it was written for", () => {
    const source = `
      const guard = () => window.confirm("Discard unsaved changes?");
      export function S({ cancelLabel = "Cancel" }) { return null; }
    `;
    expect(texts(source)).toEqual(["Discard unsaved changes?", "Cancel"]);
  });

  test("accepts the same code once it goes through t()", () => {
    const source = `
      const guard = () => window.confirm(t("common.discardChanges"));
      <Button>{cancelLabel ?? t("common.cancel")}</Button>
    `;
    expect(texts(source)).toEqual([]);
  });

  test("catches prose in a user-facing prop and in a JSX text node", () => {
    expect(texts('<Field label="Upstream model name" />')).toEqual(["Upstream model name"]);
    expect(texts("<p>No events in window.</p>")).toEqual(["No events in window."]);
  });

  test("ignores props nobody reads", () => {
    const source = `
      <div className="grid gap-3 md:grid-cols-2" id="model-form" data-testid="sheet" />
      <input type="text" name="apiBase" autoComplete="off" />
    `;
    expect(texts(source)).toEqual([]);
  });

  test("ignores codes, identifiers, urls and numbers", () => {
    const cases = [
      '<Field label="USD" />',
      '<Field label="gpt-4o" />',
      '<Field label="api_key_env" />',
      '<Field placeholder="0.00" />',
      '<Field placeholder="https://api.openai.com" />',
      "<span>RPM</span>",
    ];
    for (const source of cases) {
      expect(texts(source)).toEqual([]);
    }
  });

  test("ignores comments, which ship to nobody", () => {
    const source = `
      // <p>Not actually rendered</p>
      /* <Field label="Also not rendered" /> */
       * <span>Nor this one</span>
    `;
    expect(texts(source)).toEqual([]);
  });

  test("normalizes whitespace so reflowing a string is not a new violation", () => {
    expect(texts("<p>Two   words</p>")).toEqual(["Two words"]);
  });

  test("reports the line and the kind so the message points somewhere", () => {
    const found = findLiterals('\n\n<Field label="Model type" />', "src/x.tsx");
    expect(found).toEqual([{ file: "src/x.tsx", line: 3, text: "Model type", kind: "prop" }]);
  });
});

describe("baseline", () => {
  const found = [
    { file: "a.tsx", line: 1, text: "Old one", kind: "prop" as const },
    { file: "a.tsx", line: 9, text: "Brand new", kind: "text" as const },
  ];
  const baseline: Baseline = { "a.tsx": ["Old one"] };

  test("only the unrecorded literal fails the build", () => {
    expect(newViolations(found, baseline).map((l) => l.text)).toEqual(["Brand new"]);
  });

  test("a literal recorded under a different file still fails", () => {
    expect(newViolations(found, { "b.tsx": ["Old one", "Brand new"] })).toHaveLength(2);
  });

  test("a paid-off baseline entry is reported so it cannot come back unnoticed", () => {
    expect(staleBaseline(found, { "a.tsx": ["Old one", "Since translated"] })).toEqual([
      "a.tsx: Since translated",
    ]);
    expect(staleBaseline(found, baseline)).toEqual([]);
  });

  test("a recorded baseline is deduplicated and stably ordered", () => {
    const recorded = toBaseline([
      { file: "b.tsx", line: 2, text: "Zeta", kind: "prop" as const },
      { file: "a.tsx", line: 1, text: "Beta", kind: "prop" as const },
      { file: "a.tsx", line: 5, text: "Alpha", kind: "prop" as const },
      { file: "a.tsx", line: 7, text: "Alpha", kind: "text" as const },
    ]);
    expect(Object.keys(recorded)).toEqual(["a.tsx", "b.tsx"]);
    expect(recorded["a.tsx"]).toEqual(["Alpha", "Beta"]);
  });
});
