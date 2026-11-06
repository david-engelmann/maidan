// Runtime shape gates for values the console reads from the API. Compile-time
// Fits live in api-contract.ts; these catch the bags JSON.parse still lets
// through (wrong types, blank ids, non-finite numbers, runaway lists).

export const MAX_API_STRING = 1_000_000;
export const MAX_API_LIST = 100_000;

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function requireNonEmptyString(value: unknown, label = "value"): string {
  if (typeof value !== "string") {
    throw new TypeError(`${label} must be a string`);
  }
  const trimmed = value.trim();
  if (!trimmed) {
    throw new TypeError(`${label} must be non-empty`);
  }
  if (value.length > MAX_API_STRING) {
    throw new TypeError(`${label} exceeds ${MAX_API_STRING} characters`);
  }
  return trimmed;
}

/** Maidan ids are opaque non-empty strings without whitespace or path separators. */
export function requireApiId(value: unknown, label = "id"): string {
  const id = requireNonEmptyString(value, label);
  if (/[\s/\\]/.test(id)) {
    throw new TypeError(`${label} must not contain whitespace or path separators`);
  }
  return id;
}

export function requireFiniteNumber(value: unknown, label = "value"): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new TypeError(`${label} must be a finite number`);
  }
  return value;
}

export function requireStringList(value: unknown, label = "list", max = MAX_API_LIST): string[] {
  if (!Array.isArray(value)) {
    throw new TypeError(`${label} must be an array`);
  }
  if (value.length > max) {
    throw new TypeError(`${label} exceeds ${max} items`);
  }
  return value.map((item, i) => requireNonEmptyString(item, `${label}[${i}]`));
}

/** Read an optional string field; blank becomes null. */
export function optionalString(value: unknown, label = "value"): string | null {
  if (value == null) return null;
  if (typeof value !== "string") {
    throw new TypeError(`${label} must be a string or null`);
  }
  if (value.length > MAX_API_STRING) {
    throw new TypeError(`${label} exceeds ${MAX_API_STRING} characters`);
  }
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}
