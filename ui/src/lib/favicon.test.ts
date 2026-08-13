import { describe, it, expect } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";

// #941: the dashboard shipped with no tab icon at all, so browsers requested
// /favicon.ico, got a 404 and drew their generic placeholder — leaving rolter
// the one unidentifiable tab in an operator's pinned strip.

const root = (path: string) => fileURLToPath(new URL(`../../${path}`, import.meta.url));
const indexHtml = readFileSync(root("index.html"), "utf8");

/** every `href` on a `<link rel="...icon...">` in index.html */
function iconHrefs(): string[] {
  return [...indexHtml.matchAll(/<link\s[^>]*rel="([^"]*icon[^"]*)"[^>]*>/g)].map((tag) => {
    const href = tag[0].match(/href="([^"]+)"/)?.[1];
    if (!href) throw new Error(`icon link with no href: ${tag[0]}`);
    return href;
  });
}

describe("tab icon", () => {
  it("declares an icon at all", () => {
    expect(iconHrefs().length).toBeGreaterThan(0);
  });

  it("ships every referenced icon in public/", () => {
    // public/ is copied verbatim into dist/, so a file here is a file served
    for (const href of iconHrefs()) {
      expect(existsSync(root(`public${href}`))).toBe(true);
    }
  });

  it("covers the .ico the browser asks for unprompted", () => {
    // without it the automatic /favicon.ico request 404s even though an SVG
    // icon is declared, which is the original bug
    expect(iconHrefs()).toContain("/favicon.ico");
  });

  it("offers an apple-touch-icon for a home-screen bookmark", () => {
    expect(indexHtml).toContain('rel="apple-touch-icon"');
    expect(existsSync(root("public/apple-touch-icon.png"))).toBe(true);
  });

  it("loads no icon from an external host", () => {
    // rolter must run air-gapped: a CDN-hosted icon is a runtime request to
    // the public internet, which is the constraint this whole change is under
    for (const href of iconHrefs()) {
      expect(href.startsWith("/")).toBe(true);
    }
    expect(indexHtml).not.toMatch(/<link[^>]*rel="[^"]*icon[^"]*"[^>]*href="https?:/);
  });

  it("uses a favicon distinct from the full logo mark", () => {
    // logo-mark.svg draws 28 separate squares and turns to mush at 16px; the
    // favicon is a simplified single-path version of the same silhouette
    const favicon = readFileSync(root("public/favicon.svg"), "utf8");
    const logo = readFileSync(root("public/logo-mark.svg"), "utf8");
    expect(favicon).not.toBe(logo);
    expect((favicon.match(/<rect/g) ?? []).length).toBeLessThan(
      (logo.match(/<rect/g) ?? []).length,
    );
  });
});
