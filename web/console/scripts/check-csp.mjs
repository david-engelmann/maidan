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
  console.log(`CSP check passed: ${path}`);
}
