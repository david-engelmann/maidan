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

export type ParamRequirement = "required" | "one-of" | "optional";

export type Param = {
  name: string;
  type: string;
  /** True only when the name is in the schema's own `required` array. A conditional `anyOf`/`oneOf` requirement stays false. */
  required: boolean;
  requirement: ParamRequirement;
  description: string;
  choices: string[];
  /** Set when this name is one alternative of a conditional requirement. */
  requiredOneOf?: boolean;
  /** The `anyOf`/`oneOf` branch that names this parameter, when it is conditionally required. */
  oneOfGroup?: string[];
  /** `s.default`, rendered as a short literal. Absent when the schema sets no default. */
  defaultValue?: string;
  /** `format` on the schema, or on its non-null `anyOf`/`oneOf` branch (`uuid`, `date-time`, `uri`). */
  format?: string;
};

export type Badge = { id: "read-only" | "destructive" | "idempotent" | "open-world"; label: string };

export type CatalogFilter = "all" | "worker" | "reviewer" | "coordinator" | "read-only" | "destructive";

export type ToolProfileId = "worker" | "reviewer" | "coordinator";

export type ToolProfile = { id: ToolProfileId; label: string; path: string };

// The three fixed MCP profiles. The names match WORKER_TOOLS, REVIEWER_TOOLS
// and COORDINATOR_TOOLS in crates/maidan-mcp/src/profiles.rs; the catalog test
// reads that file and fails when the two copies drift.
export const WORKER_TOOLS: readonly string[] = [
  "acknowledge_claim",
  "amend_plan",
  "claim_next_thread",
  "claim_next_workspace_thread",
  "declare_status",
  "get_approval_gate",
  "get_thread_context",
  "get_thread_status",
  "link_thread_artifact",
  "list_active_claims",
  "post_message",
  "release_claim",
  "release_own_claims",
  "renew_claim",
  "report_usage",
  "request_approval",
  "set_thread_attributes",
  "set_thread_footprint",
  "set_thread_result",
  "submit_plan",
  "transition_thread",
  "wait_for_ready",
  "whoami",
];

export const REVIEWER_TOOLS: readonly string[] = [
  "get_artifact",
  "get_review_packet",
  "get_review_status",
  "get_thread_context",
  "get_thread_result",
  "get_waiting_inbox",
  "list_messages",
  "list_reviews",
  "list_thread_artifacts",
  "post_message",
  "submit_review",
  "transition_thread",
  "wait_for_notification",
  "whoami",
];

export const COORDINATOR_TOOLS: readonly string[] = [
  "add_reviewer",
  "approval_decide",
  "assign_thread",
  "cancel_thread",
  "create_thread",
  "find_thread",
  "find_threads",
  "get_approval_gate",
  "get_inbox",
  "get_review_status",
  "get_thread_context",
  "get_thread_result",
  "get_thread_status",
  "get_thread_steer",
  "get_waiting_inbox",
  "get_workspace_context",
  "list_channels",
  "list_child_threads",
  "list_messages",
  "list_notifications",
  "list_recently_active_threads",
  "list_result_deliveries",
  "list_reviews",
  "list_thread_results",
  "list_threads",
  "mark_inbox_read",
  "mark_notification_read",
  "post_message",
  "search_messages",
  "set_priority",
  "set_thread_attributes",
  "set_thread_steer",
  "transition_thread",
  "wait_for_notification",
  "wait_for_result",
  "whoami",
];

const PROFILES: readonly { id: ToolProfileId; label: string; path: string; tools: readonly string[] }[] = [
  { id: "worker", label: "Worker", path: "/mcp/worker", tools: WORKER_TOOLS },
  { id: "reviewer", label: "Reviewer", path: "/mcp/reviewer", tools: REVIEWER_TOOLS },
  { id: "coordinator", label: "Coordinator", path: "/mcp/coordinator", tools: COORDINATOR_TOOLS },
];

const PROFILE_TOOLS = new Map<ToolProfileId, ReadonlySet<string>>(PROFILES.map((p) => [p.id, new Set(p.tools)]));

/** Role and safety chips for the catalog. Role chips carry the profile endpoint. */
export const CATALOG_FILTERS: readonly { id: CatalogFilter; label: string; path?: string }[] = [
  { id: "all", label: "All" },
  { id: "worker", label: "Worker", path: "/mcp/worker" },
  { id: "reviewer", label: "Reviewer", path: "/mcp/reviewer" },
  { id: "coordinator", label: "Coordinator", path: "/mcp/coordinator" },
  { id: "read-only", label: "Read-only" },
  { id: "destructive", label: "Destructive" },
];

/** Profiles whose endpoint lists this tool, in worker, reviewer, coordinator order. */
export function toolProfiles(toolName: string): ToolProfile[] {
  return PROFILES.filter((p) => p.tools.includes(toolName)).map(({ id, label, path }) => ({ id, label, path }));
}

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

function choiceText(values: unknown[] | undefined): string[] {
  if (!Array.isArray(values) || !values.length) return [];
  return values.map((v) => (typeof v === "string" ? v : JSON.stringify(v)));
}

function pushChoices(out: string[], values: string[]) {
  for (const value of values) if (!out.includes(value)) out.push(value);
}

/** Enum values on the schema, on a non-null `anyOf`/`oneOf` branch, and on `items`. */
function schemaChoices(s: JsonSchema | null | undefined): string[] {
  if (!s || typeof s !== "object") return [];
  const out: string[] = [];
  pushChoices(out, choiceText(s.enum));
  const alts = s.anyOf ?? s.oneOf;
  if (Array.isArray(alts)) {
    for (const alt of alts) {
      if (!alt || alt.type === "null") continue;
      pushChoices(out, choiceText(alt.enum));
    }
  }
  if (s.items && typeof s.items === "object") pushChoices(out, choiceText(s.items.enum));
  return out;
}

function schemaFormat(s: JsonSchema | null | undefined): string | undefined {
  if (!s || typeof s !== "object") return undefined;
  if (typeof s.format === "string" && s.format) return s.format;
  const alts = s.anyOf ?? s.oneOf;
  if (!Array.isArray(alts)) return undefined;
  for (const alt of alts) {
    if (!alt || alt.type === "null") continue;
    if (typeof alt.format === "string" && alt.format) return alt.format;
  }
  return undefined;
}

/** A default rendered without extra quotes around ordinary strings. */
function formatDefault(value: unknown): string | undefined {
  if (value === undefined) return undefined;
  if (typeof value === "string") return value === "" ? '""' : value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (value === null) return "null";
  try {
    const text = JSON.stringify(value);
    return typeof text === "string" ? text : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Conditional requirements. Message tools omit `body` and `content` from
 * `required` and instead say `anyOf: [{ required: ["body"] }, { required: ["content"] }]`.
 * Each inner array is one alternative.
 */
export function schemaOneOfRequired(schema: JsonSchema | null | undefined): string[][] {
  if (!schema || typeof schema !== "object") return [];
  const branches = schema.anyOf ?? schema.oneOf;
  if (!Array.isArray(branches)) return [];
  const groups: string[][] = [];
  for (const branch of branches) {
    if (!branch || !Array.isArray(branch.required)) continue;
    const names = branch.required.filter((name): name is string => typeof name === "string" && name.length > 0);
    if (names.length) groups.push(names);
  }
  return groups;
}

/** The top-level parameters of an input schema, unconditional requirements first, then by name. */
export function schemaParams(schema: JsonSchema | null | undefined): Param[] {
  const props = schema?.properties;
  if (!props || typeof props !== "object") return [];
  const req = new Set(Array.isArray(schema?.required) ? schema.required : []);
  const groups = schemaOneOfRequired(schema);
  return Object.entries(props)
    .map(([name, s]) => {
      const unconditional = req.has(name);
      const oneOfGroup = unconditional ? undefined : groups.find((group) => group.includes(name));
      const requirement: ParamRequirement = unconditional ? "required" : oneOfGroup ? "one-of" : "optional";
      const format = schemaFormat(s);
      const defaultValue = formatDefault(s?.default);
      const param: Param = {
        name,
        type: typeWord(s),
        required: unconditional,
        requirement,
        description: typeof s?.description === "string" ? s.description.trim() : "",
        choices: schemaChoices(s),
      };
      if (oneOfGroup) {
        param.requiredOneOf = true;
        param.oneOfGroup = oneOfGroup;
      }
      if (defaultValue !== undefined) param.defaultValue = defaultValue;
      if (format) param.format = format;
      return param;
    })
    .sort((a, b) => Number(b.required) - Number(a.required) || Number(b.requirement === "one-of") - Number(a.requirement === "one-of") || a.name.localeCompare(b.name));
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

function matchesFilter(tool: CatalogTool, filter: CatalogFilter): boolean {
  if (filter === "all") return true;
  if (filter === "worker" || filter === "reviewer" || filter === "coordinator") return PROFILE_TOOLS.get(filter)?.has(tool.name) ?? false;
  if (filter === "read-only") return tool.annotations?.readOnlyHint === true;
  return tool.annotations?.destructiveHint === true && !tool.annotations?.readOnlyHint;
}

/** Tools whose name, title or description holds every word of the query, narrowed by a role or safety filter, sorted by name. */
export function filterTools(tools: CatalogTool[], query: string, filter: CatalogFilter = "all"): CatalogTool[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return tools
    .filter((t) => {
      if (!matchesFilter(t, filter)) return false;
      if (!words.length) return true;
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
