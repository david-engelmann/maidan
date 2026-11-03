// The tool catalog: what an agent sees from MCP tools/list, read over REST
// from GET /tools. Plain data in, plain data out, so the schema rendering is
// testable without a browser.
import { ApiError, apiPath, failure, guarded, hasToken, headers, type CredentialMode, type Credentials } from "./api.ts";
import type { Fetch } from "./board.ts";

export type JsonSchema = {
  type?: string | string[];
  description?: string;
  properties?: Record<string, JsonSchema>;
  required?: string[];
  items?: JsonSchema;
  enum?: unknown[];
  format?: string;
  default?: unknown;
  anyOf?: JsonSchema[];
  oneOf?: JsonSchema[];
  $ref?: string;
};

export type ToolAnnotations = {
  title?: string;
  readOnlyHint?: boolean;
  destructiveHint?: boolean;
  idempotentHint?: boolean;
  openWorldHint?: boolean;
};

export type CatalogTool = {
  name: string;
  title?: string | null;
  description?: string | null;
  inputSchema?: JsonSchema | null;
  annotations?: ToolAnnotations | null;
  _meta?: Record<string, unknown>;
};

export type Param = { name: string; type: string; required: boolean; description: string; choices: string[] };
export type Badge = { id: "read-only" | "destructive" | "idempotent" | "open-world"; label: string };

/** A schema's type in words: `string`, `string[]`, `"a" | "b"`, `any`. */
export function typeWord(s: JsonSchema | null | undefined): string {
  if (!s || typeof s !== "object") return "any";
  if (Array.isArray(s.enum) && s.enum.length) return "enum";
  const alts = s.anyOf ?? s.oneOf;
  if (Array.isArray(alts) && alts.length) {
    const words = [...new Set(alts.map(typeWord).filter((w) => w !== "null"))];
    const nullable = alts.some((a) => a?.type === "null");
    return `${words.join(" | ") || "any"}${nullable ? "?" : ""}`;
  }
  if (s.$ref) return s.$ref.split("/").pop() || "object";
  const t = Array.isArray(s.type) ? s.type.filter((x) => x !== "null") : s.type ? [s.type] : [];
  const nullable = Array.isArray(s.type) && s.type.includes("null");
  if (!t.length) return s.properties ? "object" : "any";
  const word = t.map((x) => (x === "array" ? `${typeWord(s.items)}[]` : x)).join(" | ");
  return `${word}${nullable ? "?" : ""}`;
}

/** The top-level parameters of an input schema, required ones first, then by name. */
export function schemaParams(schema: JsonSchema | null | undefined): Param[] {
  const props = schema?.properties;
  if (!props || typeof props !== "object") return [];
  const req = new Set(Array.isArray(schema?.required) ? schema.required : []);
  return Object.entries(props)
    .map(([name, s]) => ({
      name,
      type: typeWord(s),
      required: req.has(name),
      description: typeof s?.description === "string" ? s.description.trim() : "",
      choices: Array.isArray(s?.enum) ? s.enum.map((v) => (typeof v === "string" ? v : JSON.stringify(v))) : [],
    }))
    .sort((a, b) => Number(b.required) - Number(a.required) || a.name.localeCompare(b.name));
}

/** The annotation hints worth a badge, in a fixed order. */
export function annotationBadges(a: ToolAnnotations | null | undefined): Badge[] {
  const out: Badge[] = [];
  if (!a) return out;
  if (a.readOnlyHint) out.push({ id: "read-only", label: "Read-only" });
  if (a.destructiveHint && !a.readOnlyHint) out.push({ id: "destructive", label: "Destructive" });
  if (a.idempotentHint) out.push({ id: "idempotent", label: "Idempotent" });
  if (a.openWorldHint) out.push({ id: "open-world", label: "Open world" });
  return out;
}

export function toolTitle(t: CatalogTool): string {
  return (t.title || t.annotations?.title || t.name).trim();
}

/** Tools whose name, title or description holds every word of the query, sorted by name. */
export function filterTools(tools: CatalogTool[], query: string): CatalogTool[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return tools
    .filter((t) => {
      const hay = `${t.name} ${toolTitle(t)} ${t.description ?? ""}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    })
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** The first sentence of a description, for the list row. */
export function summary(description: string | null | undefined): string {
  const d = (description ?? "").trim().replace(/\s+/g, " ");
  const m = /^(.+?[.!?])(\s|$)/.exec(d);
  return m ? m[1] : d;
}

export class CatalogError extends ApiError {}

export const CATALOG_FORBIDDEN = "Your access cannot read the tool catalog. It needs workspace:read.";
export const CATALOG_SESSION_MISSING = "This server does not list tools for a signed-in session yet. Connect this browser with a token to see them.";

export class CatalogApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = guarded(fetchFn, CatalogError);
    this.creds = creds;
    this.mode = mode;
  }

  /** GET /tools: the caller's catalog, as tools/list would answer it. */
  async list(): Promise<CatalogTool[]> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, "/tools"), { headers: headers(c), credentials: "include" });
    if (res.status === 403) throw new CatalogError(CATALOG_FORBIDDEN, 403);
    if (res.status === 404 && !hasToken(c)) throw new CatalogError(CATALOG_SESSION_MISSING, 404);
    if (!res.ok) return failure(res, this.mode(), CatalogError);
    const body = (await res.json()) as { tools?: unknown };
    return Array.isArray(body?.tools) ? (body.tools as CatalogTool[]).filter((t) => t && typeof t.name === "string") : [];
  }
}

/** The tool a `#/catalog/<name>` hash opens, or null. */
export function openTool(hash: string): string | null {
  const rest = hash.replace(/^#\/?catalog\/?/, "");
  if (rest === hash || !rest) return null;
  try {
    return decodeURIComponent(rest.split(/[?/]/)[0]) || null;
  } catch {
    return null;
  }
}
