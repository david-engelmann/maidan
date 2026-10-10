// @ts-check
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
  { method: "", path: "/approval-gates/${gateId}/answer" },
  { method: "", path: "/artifacts/${encodeURIComponent(sha)}" },
  { method: "", path: "/artifacts/${encodeURIComponent(sha)}/meta" },
  { method: "", path: "/artifacts/${sha}/meta" },
  { method: "", path: "/artifacts?${params}" },
  { method: "", path: "/channels/${channelId}/threads?${q}" },
  { method: "", path: "/channels/${cid}/occupancy" },
  { method: "", path: "/channels/${cid}/queue-depth" },
  { method: "", path: "/channels/${cid}/threads" },
  { method: "", path: "/channels/${selectedChannelId}/threads" },
  { method: "", path: "/dm/${selectedDm.id}/messages" },
  { method: "", path: "/group-dms/${selectedGdm.id}/messages" },
  { method: "", path: "/me" },
  { method: "", path: "/members/${id}/delivery-mode" },
  { method: "", path: "/members/${id}/email" },
  { method: "", path: "/members/${id}/notification-prefs" },
  { method: "", path: "/members/${id}/waiting?sla_secs=${encodeURIComponent(sla)}" },
  { method: "", path: "/members/${me}/waiting" },
  { method: "", path: "/members/${sessionMemberId}/${path}" },
  { method: "", path: "/members/${sessionMemberId}/${path}/${targetId}" },
  { method: "", path: "/members/${sessionMemberId}/delivery-mode" },
  { method: "", path: "/members/${sessionMemberId}/email" },
  { method: "", path: "/members/${sessionMemberId}/notification-prefs" },
  { method: "", path: "/members/${sessionMemberId}/notifications/${nid}/read" },
  { method: "", path: "/members/${sessionMemberId}/notifications/read-all" },
  { method: "", path: "/members/${sessionMemberId}/notifications/unread-count" },
  { method: "", path: "/members/${sessionMemberId}/notifications?unread_only=${unreadOnly}&limit=50" },
  { method: "", path: "/messages/${id}" },
  { method: "", path: "/messages/${messageId}/reactions" },
  { method: "", path: "/operator/reindex-embeddings/${encodeURIComponent(id)}" },
  { method: "", path: "/threads/${id}" },
  { method: "", path: "/threads/${selectedThreadId}/messages" },
  { method: "", path: "/threads/${selectedThreadId}/messages?limit=50" },
  { method: "", path: "/threads/${threadId}/messages?limit=50" },
  { method: "", path: "/threads/${threadId}/pins" },
  { method: "", path: "/threads/${tid}" },
  { method: "", path: "/threads/${tid}/dependencies" },
  { method: "", path: "/threads/${tid}/messages?limit=50" },
  { method: "", path: "/threads/${tid}/result" },
  { method: "", path: "/threads/${tid}/review-status" },
  { method: "", path: "/threads/${tid}/reviews" },
  { method: "", path: "/workspaces/${wid()}" },
  { method: "", path: "/workspaces/${wid()}/agents" },
  { method: "", path: "/workspaces/${wid()}/app-installations" },
  { method: "", path: "/workspaces/${wid()}/channels" },
  { method: "", path: "/workspaces/${wid()}/deliveries/${id}/replay?kind=${encodeURIComponent(kind)}" },
  { method: "", path: "/workspaces/${wid()}/deliveries?${query}" },
  { method: "", path: "/workspaces/${wid()}/dm" },
  { method: "", path: "/workspaces/${wid()}/dm?member_id=${encodeURIComponent(me)}" },
  { method: "", path: "/workspaces/${wid()}/events?${q}" },
  { method: "", path: "/workspaces/${wid()}/group-dms" },
  { method: "", path: "/workspaces/${wid()}/group-dms?member_id=${encodeURIComponent(me)}" },
  { method: "", path: "/workspaces/${wid()}/members" },
  { method: "", path: "/workspaces/${wid()}/peers" },
  { method: "", path: "/workspaces/${wid()}/slash-commands" },
  { method: "", path: "/workspaces/${wid()}/slash-commands/${id}" },
  { method: "", path: "/workspaces/${ws}/approval-gates" },
  { method: "", path: "/workspaces/${ws}/channels" },
  { method: "", path: "/workspaces/${ws}/events?limit=200" },
  { method: "", path: "/workspaces/${ws}/peers" },
  { method: "", path: "/workspaces/${ws}/task-schedules" },
  { method: "", path: "/ws/subscribe" },
];

/**
 * @param {string} method
 * @param {string} path
 * @returns {UiOperation | undefined}
 */
export function findOperation(method, path) {
  return OPERATIONS.find((op) => op.method === method && op.path === path);
}
