// Rebuild crates/maidan-server/static/ui/client.js from the board modules
// and a saved /openapi.json.
//
//   node scripts/gen-ui-client.mjs path/to/openapi.json
//   node scripts/gen-ui-client.mjs path/to/openapi.json --stdout
//
// The ui_fetch_openapi_contract test runs this with --stdout against the
// built ApiDoc and fails when client.js differs, so the catalog cannot drift
// from the routes; MAIDAN_WRITE_SNAPSHOTS=1 on that test rewrites the file.
//
// Paths are the templates the page already sends. Each lists every method the
// OpenAPI document has on the matching path, since a template alone cannot
// say which one a call site uses. Parameter segments in the document match
// ${...} in a template.

import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const uiDir = join(root, "crates/maidan-server/static/ui");
const args = process.argv.slice(2);
const toStdout = args.includes("--stdout");
const openapiPath = args.find((arg) => arg !== "--stdout");
if (!openapiPath) {
  console.error("usage: node scripts/gen-ui-client.mjs path/to/openapi.json [--stdout]");
  process.exit(1);
}

const spec = JSON.parse(readFileSync(openapiPath, "utf8"));
const METHODS = ["get", "post", "put", "patch", "delete", "head", "options"];
const methodsByNorm = new Map();
for (const [path, item] of Object.entries(spec.paths || {})) {
  const norm = path.replace(/\{[^}]+\}/g, "{}");
  const seenHere = new Set(methodsByNorm.get(norm) || []);
  for (const name of METHODS) if (item && item[name]) seenHere.add(name.toUpperCase());
  // A fixed order keeps the output stable however the document orders paths.
  methodsByNorm.set(norm, METHODS.map((m) => m.toUpperCase()).filter((m) => seenHere.has(m)));
}

function normalize(value) {
  if (!value.startsWith("/") || value.startsWith("//")) return null;
  const path = value.split(/[?#]/)[0];
  let out = "";
  let i = 0;
  while (i < path.length) {
    if (path.startsWith("${", i)) {
      let depth = 1;
      i += 2;
      while (i < path.length && depth > 0) {
        if (path[i] === "{") depth += 1;
        else if (path[i] === "}") depth -= 1;
        i += 1;
      }
      if (depth !== 0) return null;
      out += "{}";
    } else {
      out += path[i];
      i += 1;
    }
  }
  return out;
}

// Comments and regex literals are skipped: an apostrophe in a comment, or the
// quote in `/"/g`, would otherwise open a string that swallows the code after
// it and hides the route templates there.
function stringLiterals(source) {
  const out = [];
  let i = 0;
  let last = "";
  while (i < source.length) {
    if (source[i] === "/" && source[i + 1] !== "/" && source[i + 1] !== "*" && (last === "" || "(,=:[!&|?{};+".includes(last))) {
      i = regexEnd(source, i);
      last = "/";
      continue;
    }
    if (source.startsWith("//", i)) {
      while (i < source.length && source[i] !== "\n") i += 1;
      continue;
    }
    if (source.startsWith("/*", i)) {
      const end = source.indexOf("*/", i + 2);
      i = end < 0 ? source.length : end + 2;
      continue;
    }
    const quote = source[i];
    if (quote !== "'" && quote !== '"' && quote !== "`") {
      if (!/\s/.test(quote)) last = quote;
      i += 1;
      continue;
    }
    last = quote;
    const start = i + 1;
    i = start;
    let escaped = false;
    while (i < source.length) {
      if (escaped) escaped = false;
      else if (source[i] === "\\") escaped = true;
      else if (source[i] === quote) {
        out.push(source.slice(start, i));
        i += 1;
        break;
      }
      i += 1;
    }
  }
  return out;
}

// The index just past a regex literal that opens at `start`. A `/` inside a
// character class does not close it.
function regexEnd(source, start) {
  let i = start + 1;
  let inClass = false;
  while (i < source.length && source[i] !== "\n") {
    const c = source[i];
    if (c === "\\") i += 1;
    else if (c === "[") inClass = true;
    else if (c === "]") inClass = false;
    else if (c === "/" && !inClass) return i + 1;
    i += 1;
  }
  return i;
}

const seen = new Set();
const templates = [];
for (const name of readdirSync(uiDir).filter((n) => n.endsWith(".js") && n !== "client.js").sort()) {
  const source = readFileSync(join(uiDir, name), "utf8");
  for (const literal of stringLiterals(source)) {
    // Direct calls build `${base()}/path`; the path after the base is the route.
    const raw = literal.startsWith("${base()}/") ? literal.slice("${base()}".length) : literal;
    if (!raw.startsWith("/")) continue;
    const norm = normalize(raw);
    if (!norm || !methodsByNorm.has(norm)) continue;
    if (seen.has(raw)) continue;
    seen.add(raw);
    templates.push(raw);
  }
}
templates.sort();

const rows = templates.map((path) => {
  const methods = methodsByNorm.get(normalize(path)) || [];
  return `  { methods: ${JSON.stringify(methods).replaceAll(",", ", ")}, path: ${JSON.stringify(path)} },`;
});

const out = `// @ts-check
/**
 * Typed catalog of the board calls. Paths are the templates the page already
 * sends, each with every method OpenAPI has on that path. Generated by
 * scripts/gen-ui-client.mjs; the ui_fetch_openapi_contract test fails when
 * this file is stale (MAIDAN_WRITE_SNAPSHOTS=1 on that test rewrites it).
 * @typedef {object} UiOperation
 * @property {readonly string[]} methods
 * @property {string} path
 */

/** @type {readonly UiOperation[]} */
export const OPERATIONS = [
${rows.join("\n")}
];

/**
 * @param {string} method
 * @param {string} path
 * @returns {UiOperation | undefined}
 */
export function findOperation(method, path) {
  return OPERATIONS.find((op) => op.path === path && op.methods.includes(method));
}
`;

if (toStdout) {
  process.stdout.write(out);
} else {
  writeFileSync(join(uiDir, "client.js"), out);
  console.log(`wrote ${templates.length} operations`);
}
