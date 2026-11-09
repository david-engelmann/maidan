// The name chosen before the provider round trip. The server applies the
// same limits and, when the field is left blank, names the workspace after
// the new member.

/** The server's `WORKSPACE_NAME_MAX_CHARS`, in Unicode scalars. */
export const WORKSPACE_NAME_MAX = 200;

const WORKSPACE_INVITE_PREFIX = "maid_inv_";

/** A workspace invite joins an existing workspace, so it has no name to set. */
export function isWorkspaceInvite(invite: string | null): boolean {
  return invite !== null && invite.startsWith(WORKSPACE_INVITE_PREFIX);
}

/** Why `raw` cannot be a workspace name, or null when the server would keep it. Blank is kept: the server fills it in. */
export function workspaceNameRefusal(raw: string): string | null {
  const name = raw.trim();
  if (!name) return null;
  if ([...name].length > WORKSPACE_NAME_MAX) return `A workspace name can be at most ${WORKSPACE_NAME_MAX} characters.`;
  if ([...name].some(isControl)) return "A workspace name can't include control characters.";
  return null;
}

/** Turnstile's normal widget is 300px wide; in a narrower column, as on a 360px phone, it ran past the form. */
export const TURNSTILE_NORMAL_WIDTH = 300;

/** The Turnstile size that fits a slot `width` pixels wide. */
export function turnstileSize(width: number): "normal" | "compact" {
  return width < TURNSTILE_NORMAL_WIDTH ? "compact" : "normal";
}

/** Unicode category Cc: C0 controls, DEL, and the C1 controls. */
function isControl(char: string): boolean {
  const code = char.codePointAt(0) ?? 0;
  return code <= 0x1f || (code >= 0x7f && code <= 0x9f);
}

/**
 * Where the provider sign-in may send the browser afterwards. Anything that
 * is not a path on this origin becomes the console, matching the server.
 */
export function sameOriginReturn(raw: string | null): string {
  if (
    raw &&
    raw.startsWith("/") &&
    !raw.startsWith("//") &&
    !raw.includes("\\") &&
    ![...raw].some(isControl)
  ) {
    return raw;
  }
  return "/ui/";
}

/**
 * The path Sign in puts in `return_to`. The server keeps a same-origin path
 * (`safe_return_to`): `#` is not a control character, so `/ui/#/needs` is
 * accepted once the query value is percent-encoded. A bare board is the
 * console itself. A confirmation link's one-time token never goes in the query.
 */
export function signInReturnTo(hash: string): string {
  if (/^#\/?confirm-approval=/.test(hash)) return "/ui/#/needs";
  if (hash === "" || hash === "#" || hash === "#/" || hash === "#/board") return "/ui/";
  const route = hash.startsWith("#") ? hash : `#${hash}`;
  return sameOriginReturn(`/ui/${route}`);
}

export type SignupCheck = {
  /** The server sent the browser here to name the workspace (and pass Turnstile). */
  active: boolean;
  name: string;
  invite: string | null;
  returnTo: string;
};

/** The query the sign-up start leaves on `/ui/`. */
export function readSignupCheck(search: string): SignupCheck {
  const q = new URLSearchParams(search.replace(/^\?/, ""));
  const invite = q.get("invite");
  return {
    active: q.get("signup_check") === "1",
    name: q.get("workspace_name") ?? "",
    invite: invite && invite.length > 0 ? invite : null,
    returnTo: sameOriginReturn(q.get("return_to")),
  };
}
