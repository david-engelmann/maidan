// Builds the try page's scene: try-src/gathering.ts, its types stripped, is
// try/gathering.js. The try page has no bundler and loads its scripts as
// they are, and the source uses only types that erase (no enums, no
// namespaces, no parameter properties), so Node's own type stripping is the
// whole build. Both files are committed: the Worker serves the built one.
//
//   node try-src/build.mjs          write try/gathering.js
//   node try-src/build.mjs --check  fail if it is not the source's build
//
// test/try-gathering.test.js runs the check, so an edit to one without the
// other fails CI.

import { readFileSync, writeFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { fileURLToPath } from "node:url";

const SOURCE = new URL("./gathering.ts", import.meta.url);
const OUTPUT = new URL("../try/gathering.js", import.meta.url);

const HEADER = `// Built from deploy/cloudflare/try-src/gathering.ts by try-src/build.mjs
// (npm run build:try). Do not edit: change the source and build again.

`;

/** The built module for a TypeScript source. */
export function build(source) {
  // Stripping keeps every position, comments included, and blanks the types
  // out with spaces. Those blanks are taken out again (with the space before
  // one that ended a statement or a list), so the file reads like the
  // source without its types.
  const js = stripTypeScriptTypes(source, { mode: "strip" });
  if (js.length !== source.length) throw new Error("type stripping moved the source");
  let out = "";
  for (let i = 0; i < js.length; ) {
    const blanked = (k) => js[k] === " " && !/\s/.test(source[k]);
    if (!blanked(i)) {
      out += js[i++];
      continue;
    }
    // One type is one run, its own spaces included (`: Handle | null`)
    for (;;) {
      while (i < js.length && blanked(i)) i++;
      let k = i;
      while (k < js.length && (source[k] === " " || source[k] === "\t")) k++;
      if (k > i && blanked(k)) i = k;
      else break;
    }
    if (out.endsWith(" ") && /[ ;,)\]]/.test(js[i] ?? "")) out = out.slice(0, -1);
  }
  const tidy = out
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  return `${HEADER}${tidy}\n`;
}

export function built() {
  return build(readFileSync(SOURCE, "utf8"));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const out = built();
  if (process.argv.includes("--check")) {
    if (readFileSync(OUTPUT, "utf8") !== out) {
      console.error("try/gathering.js is not the build of try-src/gathering.ts: run npm run build:try");
      process.exit(1);
    }
  } else {
    writeFileSync(OUTPUT, out);
  }
}
