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

  // the `>` of an arrow is not a closing tag. without this, every `.tsx` file
  // returning a generic from an arrow function reports the type name as copy
  test("ignores a generic return type on an arrow function", () => {
    const source = `
      export type FetchStub = (input: RequestInfo) => Promise<Response>;
      export const pending: FetchStub = scoped(() => new Promise<Response>(() => {}));
      const pick = <T,>(xs: T[]): Array<T> => xs;
    `;
    expect(texts(source)).toEqual([]);
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

// the blind spots the first scanner had (#1200): one finding per line, prose
// that wraps, strings inside expressions, a confirm split across lines
describe("findLiterals sees what the line-at-a-time scan missed", () => {
  test("reports every literal on a dense line, not just the first", () => {
    const source = '<Button>Cancel</Button><Button>Delete</Button>';
    expect(texts(source)).toEqual(["Cancel", "Delete"]);
  });

  test("reads a paragraph that wraps across lines", () => {
    const source = `
      <p className="text-sm">
        This invitation link is not valid. It may have been used,
        revoked, or expired.
      </p>`;
    expect(texts(source)).toEqual([
      "This invitation link is not valid. It may have been used, revoked, or expired.",
    ]);
  });

  test("reads the strings inside an expression in text position", () => {
    const source = '<Button>{pending ? "Saving…" : "Save group"}</Button>';
    expect(texts(source)).toEqual(["Saving…", "Save group"]);
  });

  test("reads the strings inside an expression-valued user-facing prop", () => {
    const source = '<Sheet title={initial ? "Configure tool group" : "Create tool group"} />';
    expect(texts(source)).toEqual(["Configure tool group", "Create tool group"]);
  });

  test("reads a window.confirm whose literal sits on the next line", () => {
    const source = `
      if (
        window.confirm(
          "Delete this rule? Traffic that matched it will pass through unchecked.",
        )
      ) remove.mutate(id);`;
    expect(texts(source)).toEqual([
      "Delete this rule? Traffic that matched it will pass through unchecked.",
    ]);
  });

  test("still ignores keyboard keys, header names and class lists in expressions", () => {
    const source = `
      <div onKeyDown={(e) => e.key === "Escape" && close()} className={cn("flex items-center", open && "bg-muted")}>
        {label}
      </div>`;
    expect(texts(source)).toEqual([]);
  });

  test("blanks a t() call that spans lines", () => {
    const source = `
      <p>
        {t("pages.acceptInvite.intro", {
          email,
        })}
      </p>`;
    expect(texts(source)).toEqual([]);
  });
});

// an error's message is what LoadError prints under its heading (#1200)
test("reads the message of a thrown error", () => {
  expect(texts('throw new ApiError("gateway request failed: ${res.status}", 502);')).toEqual([
    "gateway request failed: ${res.status}",
  ]);
  expect(texts('throw new Error("scope-1");')).toEqual([]);
});
