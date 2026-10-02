// Rebuild crates/maidan-server/static/ui/client.js from the board modules
// and a saved /openapi.json. cargo build does not run this.
//
//   node scripts/gen-ui-client.mjs path/to/openapi.json
//
// Paths are the templates the page already sends. A method is filled in when
// the OpenAPI document has exactly one method on the matching path. A path
// with several methods stays with an empty method so the page template is
// not guessed. Parameter segments in the document match ${...} in a template.

import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const uiDir = join(root, "crates/maidan-server/static/ui");
const openapiPath = process.argv[2];
if (!openapiPath) {
  console.error("usage: node scripts/gen-ui-client.mjs path/to/openapi.json");
  process.exit(1);
}

const spec = JSON.parse(readFileSync(openapiPath, "utf8"));
const methodsByNorm = new Map();
for (const [path, item] of Object.entries(spec.paths || {})) {
  const norm = path.replace(/\{[^}]+\}/g, "{}");
  const methods = ["get", "post", "put", "patch", "delete", "head", "options"]
    .filter((name) => item && item[name])
    .map((name) => name.toUpperCase());
  const prev = methodsByNorm.get(norm) || [];
  methodsByNorm.set(norm, [...new Set([...prev, ...methods])]);
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

const seen = new Set();
const templates = [];
for (const name of readdirSync(uiDir).filter((n) => n.endsWith(".js") && n !== "client.js").sort()) {
  const source = readFileSync(join(uiDir, name), "utf8");
  const re = /`([^`\\]|\\.)*`|"([^"\\]|\\.)*"|'([^'\\]|\\.)*'/g;
  let match;
  while ((match = re.exec(source))) {
    let raw = match[0].slice(1, -1);
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
  const method = methods.length === 1 ? methods[0] : "";
  return `  { method: ${JSON.stringify(method)}, path: ${JSON.stringify(path)} },`;
});

const out = `// @ts-check
/**
 * Typed catalog of the board calls. Paths are the templates the page already
 * sends. Regenerate from a live /openapi.json with scripts/gen-ui-client.mjs.
 * cargo build does not run that script.
 * @typedef {object} UiOperation
 * @property {string} method
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
  return OPERATIONS.find((op) => op.method === method && op.path === path);
}
`;

writeFileSync(join(uiDir, "client.js"), out);
console.log(`wrote ${templates.length} operations`);
