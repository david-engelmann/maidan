// Fails when the console drifts from the server's committed OpenAPI spec.
//  1. src/core/api.gen.ts must match a fresh generation from the spec.
//  2. Every API path the console calls (string and template literals in src,
//     with the method read off the same line) must be a documented route.
// Shapes are pinned by src/core/api-contract.ts, which `npm run typecheck` checks.
// Run: npm run check:spec
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { SPEC, OUT, generate, routes } from "./gen-api-types.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(here, "../src");

/** Routes the console calls before the spec documents them, each with the PR that adds it. */
export const PENDING = new Map([["GET /threads/{}/votes", "#1838: the console falls back to per-message votes on 404"]]);

const API_ROOTS = /^\/(threads|messages|members|workspaces|channels|artifacts|dm|group-dms|approval-gates|operator|me)(\/|$|\?)/;
const METHOD = /["'](GET|POST|PUT|PATCH|DELETE)["']/;

export function callsIn(text) {
  const out = [];
  const lines = text.split("\n");
  lines.forEach((line, i) => {
    // A path built into a variable is sent on the next line: `send("POST", path, ...)`.
    const v = line.match(/^\s*(?:const|let) (\w+) = /)?.[1];
    const next = v && new RegExp(`, ${v}[,)]`).test(lines[i + 1] ?? "") ? lines[i + 1] : "";
    const method = (line.match(METHOD) ?? next.match(METHOD) ?? [])[1] ?? "GET";
    for (const m of line.matchAll(/`([^`]*)`|"([^"]*)"/g)) {
      const raw = m[1] ?? m[2];
      if (!API_ROOTS.test(raw)) continue;
      const path = raw.split("?")[0].replace(/\$\{[^}]*\}/g, "{}");
      out.push({ line: i + 1, method, path });
    }
  });
  return out;
}

export function matches(spec, method, path) {
  const re = new RegExp(`^${method} ${path.replace(/[.*+?^$()|[\]\\]/g, "\\$&").replace(/\\?\{\\?\}|\{\}/g, "[^/]+")}$`);
  return routes(spec).some((r) => re.test(r));
}

function files(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    // src/pwa holds cache-policy prefixes, not API calls.
    if (statSync(p).isDirectory()) return n === "pwa" ? [] : files(p);
    return /\.(ts|tsx)$/.test(n) && !/\.test\.|api\.gen\.ts$/.test(n) ? [p] : [];
  });
}

export function check(spec, srcFiles, read) {
  const errors = [];
  for (const f of srcFiles) for (const c of callsIn(read(f))) {
    const id = `${c.method} ${c.path}`;
    if (!matches(spec, c.method, c.path) && !PENDING.has(id)) errors.push(`${f}:${c.line}: ${id} is not in the spec`);
  }
  return errors;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const spec = JSON.parse(readFileSync(SPEC, "utf8"));
  const errors = [];
  if (readFileSync(OUT, "utf8") !== generate(spec)) errors.push("src/core/api.gen.ts is stale: run `npm run gen:api`");
  const list = files(SRC);
  errors.push(...check(spec, list.map((f) => relative(process.cwd(), f)), (f) => readFileSync(f, "utf8")));
  for (const [id, why] of PENDING) if (matches(spec, ...id.split(" "))) errors.push(`${id} is now in the spec: drop it from PENDING (${why})`);
  if (errors.length) { console.error(errors.join("\n")); process.exit(1); }
  console.log(`check:spec ok: ${list.length} files against ${routes(spec).length} routes`);
}
