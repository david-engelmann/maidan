// @ts-check


      const tokenKey = "maidan_token";

      const wsKey = "maidan_workspace";

      const wsResumeKey = "maidan_ws_resume";

      const baseInput = document.getElementById("base");

      baseInput.value = window.location.origin;

      document.getElementById("workspace").value = localStorage.getItem(wsKey) || "";

      const channelKey = "maidan_channel";

      // Workspace member directory: id -> { name, handle, kind }. Everything a
      // person reads shows a display name; the raw id stays in a tooltip.
      const memberDirectory = new Map();


      const BOARD_COLUMNS = [
        { key: "open", title: "Open" },
        { key: "working", title: "In progress" },
        { key: "review", title: "Needs review" },
        { key: "done", title: "Done" },
      ];


      // The channel as a board: one card per task, in the column its real
      // state puts it, with the one member holding it. Cards that changed since
      // the last render flash once, so a live update is visible.
      // A refused close, in the words the server already returned. The agent already got
      // this sentence. It stays on the card: the line is "Close refused", and the
      // sentence is the card title. The strip above the lanes is not painted.
      const refusals = new Map();


      // Cards move between lanes instead of blinking: record where each card
      // was, re-render, then play each one from its old spot to its new one
      // (FLIP). A card that is new to the board fades in. Nothing moves under
      // prefers-reduced-motion, and a channel switch is a fresh paint.
      const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");


      // ---- Team -------------------------------------------------------------
      // Every member on the board, agents first: what each one holds right
      // now, and whether they are live. Live means holding running work, or
      // seen on the socket (an event they caused, or presence) in the last two
      // minutes. Nothing here is inferred beyond that.
      const lastSeen = new Map();

      const LIVE_MS = 120000;

      // Event frames are flat: the event fields sit at the top level beside
      // log_id and kind, the message of a message event is nested, and who
      // acted is in attribution. A first subscribe starts at the head and a
      // reconnect replays only what it missed, so arrival time is close to
      // when an event happened.
      const ACTOR_KEYS = ["actor_id", "author_id", "assignee_id", "member_id", "produced_by", "reviewer_id", "requested_by", "resolved_by", "editor_id", "updated_by"];

      // Some kinds name a subject, not an actor: a lapsed or failed claim names
      // the holder that went quiet, and a reassignment names who received the
      // work. Those members did not act, so they do not light up.
      const SUBJECT_KEYS = {
        claim_expired: ["member_id", "assignee_id"],
        claim_failed: ["member_id", "assignee_id"],
        thread_assignment_changed: ["assignee_id", "member_id"],
      };


      // Board refreshes are driven by thread/claim/review events on the socket,
      // coalesced so a burst of events is one reload.
      const THREAD_BOARD_KINDS = new Set([
        "thread_created",
        "thread_state_changed",
        "thread_assignment_changed",
        "thread_result_set",
        "review_submitted",
        "thread_landed",
        "thread_ready",
        "approval_requested",
        "blocked_resolved",
        "claim_expired",
      ]);

      const NY_KINDS = new Set(["review_request", "open_gate"]);

      // A Needs you load that could not reach the server retries on its own,
      // backing off to once a minute.
      const NY_RETRY_MIN_MS = 5000;

      const NY_RETRY_MAX_MS = 60000;


      // Live thread view. WS event frames whose thread matches the
      // open thread refresh the message list (debounced) instead of only
      // landing as raw lines under the Live toolbar. Requires the WebSocket
      // connected with a filter that includes this thread (workspace-wide
      // subscribe covers it).
      const THREAD_CONTENT_KINDS = new Set([
        "message_posted",
        "message_edited",
        "message_tombstoned",
        "reaction_added",
        "reaction_removed",
        "message_pinned",
        "message_unpinned",
      ]);


      const QUICK_REACTIONS = ["👍", "❤️", "✅", "🎉", "👀"];

      // The worker preset is the named set on the server (maidan.agent.worker):
      // claim, post, and transition. The browser token is never written into
      // a snippet or replaced by the agent secret.
      const WORKER_PRESET = "maidan.agent.worker";

      // The shortcut reads the way this keyboard labels it.
      const onMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);


      // Raster types the server serves inline. SVG is not one: it can carry
      // script, and a blob: URL made from it would run in this page's origin.
      const INLINE_IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

      const artifactFetches = new Map();


      // A live thread redraws on every message; an image is fetched once.
      const artifactImages = new Map();

      const LIVE_POLL_MS = 15000;

export { ACTOR_KEYS, BOARD_COLUMNS, INLINE_IMAGE_TYPES, LIVE_MS, LIVE_POLL_MS, NY_KINDS, NY_RETRY_MAX_MS, NY_RETRY_MIN_MS, QUICK_REACTIONS, SUBJECT_KEYS, THREAD_BOARD_KINDS, THREAD_CONTENT_KINDS, WORKER_PRESET, artifactFetches, artifactImages, baseInput, channelKey, lastSeen, memberDirectory, onMac, reduceMotion, refusals, tokenKey, wsKey, wsResumeKey };
