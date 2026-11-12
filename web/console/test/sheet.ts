import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// src/styles.css is an index of @import lines, one module per part of the
// console. A test about the stylesheet reads it the way the bundler does:
// every module inlined where it is imported, in cascade order.

const entry = fileURLToPath(new URL("../src/styles.css", import.meta.url));

export function inline(path: string): string {
  return readFileSync(path, "utf8").replace(/^@import "([^"]+)";\n?/gm, (_, rel: string) => inline(join(dirname(path), rel)));
}

/** The console's main stylesheet with every import inlined. */
export const sheet = (): string => inline(entry);
