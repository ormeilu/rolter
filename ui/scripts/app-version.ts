import { readFileSync } from "node:fs";

/**
 * The product version the dashboard reports, read from the Cargo workspace.
 *
 * `ui/package.json` used to carry its own `"version"`, which nothing ever
 * bumped — the sidebar showed v0.0.1 while the product was at 0.0.10, and every
 * UX event ever emitted was labelled 0.0.1, collapsing cross-release analysis
 * into a single bucket (#953).
 *
 * The workspace manifest is the one version release-plz already maintains, so
 * reading it leaves exactly one source of truth rather than a second thing to
 * keep in sync.
 */
export function parseWorkspaceVersion(cargoToml: string): string {
  // scoped to [workspace.package] on purpose: the file also carries a
  // [workspace.dependencies] section full of `version = "..."` lines, and the
  // first match in the whole file is not necessarily ours
  const section = cargoToml.split(/^\[workspace\.package\]\s*$/m)[1];
  const version = section?.match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version) {
    throw new Error(
      "could not read version from [workspace.package] in the Cargo workspace manifest; " +
        "the dashboard version is derived from it (see ui/scripts/app-version.ts)",
    );
  }
  return version;
}

/** Read and parse the workspace version from `cargoTomlPath`. */
export function readAppVersion(cargoTomlPath: string): string {
  return parseWorkspaceVersion(readFileSync(cargoTomlPath, "utf8"));
}
