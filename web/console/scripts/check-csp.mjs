// Fails when the built console page carries anything the strict CSP blocks:
// an inline <script> (one without src), a <style> element, an inline event
// handler attribute, or a javascript: URL. Usage:
//   node scripts/check-csp.mjs [path/to/index.html]
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** @param {string} html @returns {string[]} */
export function cspViolations(html) {
  const found = [];
  for (const m of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi)) {
    const attrs = m[1] ?? "";
    const body = (m[2] ?? "").trim();
    if (!/\bsrc\s*=/i.test(attrs)) found.push("inline <script> without src");
    else if (body) found.push("<script src> with an inline body");
  }
  if (/<style\b/i.test(html)) found.push("<style> element");
  if (/\son[a-z]+\s*=/i.test(html)) found.push("inline event handler attribute");
  if (/javascript:/i.test(html)) found.push("javascript: URL");
  return found;
}

/**
 * Chunks the entry imports statically that index.html does not preload, so
 * the browser would find them one import level late.
 * @param {string} html @param {string} entry @param {string} base
 * @returns {string[]}
 */
export function missingPreloads(html, entry, base) {
  const chunks = new Set();
  for (const m of entry.matchAll(/(?:\bfrom|\bimport)\s*["']\.\/([^"']+\.js)["']/g)) chunks.add(m[1]);
  const preloaded = new Set();
  for (const m of html.matchAll(/<link\b[^>]*>/gi)) {
    if (!/\brel="modulepreload"/i.test(m[0])) continue;
    const href = /\bhref="([^"]+)"/i.exec(m[0])?.[1];
    if (href?.startsWith(base)) preloaded.add(href.slice(base.length));
  }
  return [...chunks].filter((c) => !preloaded.has(c)).sort();
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const path =
    process.argv[2] ??
    (process.env.CONSOLE_OUT ? `${process.env.CONSOLE_OUT}/index.html` : undefined) ??
    fileURLToPath(new URL("../../../crates/maidan-server/static/console/dist/index.html", import.meta.url));
  const html = readFileSync(path, "utf8");
  const bad = cspViolations(html);
  if (bad.length > 0) {
    console.error(`CSP check failed for ${path}:\n- ${bad.join("\n- ")}`);
    process.exit(1);
  }
  if (!/<script\b[^>]*\bsrc="\/ui\/app\.js"/.test(html)) {
    console.error(`CSP check: ${path} does not load /ui/app.js`);
    process.exit(1);
  }
  const base = /<script\b[^>]*\bsrc="([^"]*\/)app\.js"/.exec(html)?.[1] ?? "/";
  const entry = readFileSync(path.replace(/index\.html$/, "app.js"), "utf8");
  const late = missingPreloads(html, entry, base);
  if (late.length > 0) {
    console.error(`Preload check: ${path} does not modulepreload ${late.join(", ")}, which app.js imports`);
    process.exit(1);
  }
  console.log(`CSP check passed: ${path}`);
}
