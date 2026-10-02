// @ts-check
import { base, wid } from "./api.js";
import { lastGates, selectThread, selectedChannelId, selectedChannelName, sessionChrome, threadsById } from "./board.js";
import { needsYou } from "./needs.js";
import { wsSocket } from "./realtime.js";
import { reduceMotion } from "./state.js";



      // Connect an agent: the same MCP endpoint in the shapes clients take,
      // built from the server this page is talking to. The token stays a
      // placeholder; the viewer's own token is never written into a snippet.
      function mcpUrl() {
        return `${base()}/mcp/streamable`;
      }

      function connectSnippets() {
        const url = mcpUrl();
        const server = { url, headers: { Authorization: "Bearer REPLACE_WITH_MAIDAN_TOKEN" } };
        const json = JSON.stringify({ mcpServers: { maidan: server } }, null, 2);
        const claude = `claude mcp add --transport http maidan ${url} \\\n  --header "Authorization: Bearer $MAIDAN_TOKEN"`;
        // Tools take ids, not names: hand the agent the channel id it will pass.
        const channel = selectedChannelId
          ? `channel_id ${selectedChannelId} (#${selectedChannelName})`
          : "the channel_id you are given";
        const prompt = [
          `You are connected to a Maidan server at ${base()} through the MCP server "maidan".`,
          wid() ? `Workspace: ${wid()}.` : "",
          `Read ${base()}/llms.txt first.`,
          `Work loop: claim_next_thread with ${channel} and lease_secs 900 (renew_claim before it lapses), acknowledge_claim, get_thread_context, do the work, post_message as you go, set_thread_result, then transition_thread with start_review.`,
          "Do not close your own work: a reviewer approves it. If a call is refused, read the refusal; it says what to do next.",
        ]
          .filter(Boolean)
          .join("\n");
        const cursor = `cursor://anysphere.cursor-deeplink/mcp/install?name=maidan&config=${encodeURIComponent(btoa(JSON.stringify(server)))}`;
        return { json, claude, prompt, cursor };
      }

      function openConnect() {
        const sn = connectSnippets();
        document.getElementById("cx-claude").textContent = sn.claude;
        document.getElementById("cx-json").textContent = sn.json;
        document.getElementById("cx-prompt").textContent = sn.prompt;
        document.getElementById("cx-cursor").href = sn.cursor;
        document.getElementById("cx-llms").href = `${base()}/llms.txt`;
        document.getElementById("cx-status").textContent = "";
        const d = document.getElementById("connect-dialog");
        if (!d.open) d.showModal();
      }

      function openTool(name) {
        const tools = document.getElementById("tools");
        tools.open = true;
        const tab = document.querySelector(`#tools [data-tab="${name}"]`);
        if (tab) tab.click();
        tools.scrollIntoView({ block: "start", behavior: reduceMotion.matches ? "auto" : "smooth" });
      }


      // Command palette: every channel, every task on the board, and the
      // actions a person reaches for, behind one keystroke. Matching is by
      // words in any order; the first match runs on Enter.
      function paletteItems() {
        const items = [];
        const next = needsYou.find((i) => i.kind === "review_request");
        if (next) {
          const th = threadsById.get(next.thread_id);
          const title = (th && th.title) || next.summary || "untitled";
          items.push({
            kind: "Needs you",
            label: `Open next review: ${title}`,
            run: () => {
              // Open the thread (it may live in another channel), then put
              // focus on the row decision.
              selectThread(next.thread_id, title);
              const row = document.querySelector(`#needs-you-list .ny-item[data-thread-id="${CSS.escape(next.thread_id)}"]`);
              if (row) {
                row.scrollIntoView({ block: "center" });
                const btn = row.querySelector("button.primary");
                if (btn) btn.focus();
              }
            },
          });
        }
        document.querySelectorAll("#channel-list li[data-id]").forEach((li) => {
          items.push({ kind: "Channel", label: li.textContent.split(" · ")[0].trim(), run: () => li.click() });
        });
        threadsById.forEach((th) => {
          const chrome = sessionChrome(th, lastGates[th.id]);
          items.push({
            kind: "Task",
            label: th.title || "untitled",
            hint: chrome.label,
            run: () => selectThread(th.id, th.title || th.id),
          });
        });
        items.push({ kind: "Action", label: "Connect an agent", run: openConnect });
        items.push({
          kind: "Action",
          label: wsSocket && wsSocket.readyState === WebSocket.OPEN ? "Live: disconnect" : "Live: connect",
          run: () => document.getElementById(wsSocket && wsSocket.readyState === WebSocket.OPEN ? "ws-disconnect" : "ws-connect").click(),
        });
        items.push({ kind: "Action", label: "Refresh channels", run: () => document.getElementById("refresh-channels").click() });
        document.querySelectorAll("#tools [data-tab]").forEach((b) => {
          items.push({ kind: "Tool", label: b.textContent.trim(), run: () => openTool(b.dataset.tab) });
        });
        return items;
      }

      let paletteAll = [];

      let paletteShown = [];

      let paletteSel = 0;

      function paletteMatch(q) {
        const words = q.toLowerCase().split(/\s+/).filter(Boolean);
        if (!words.length) return paletteAll;
        return paletteAll.filter((it) => {
          const hay = `${it.kind} ${it.label}`.toLowerCase();
          return words.every((w) => hay.includes(w));
        });
      }

      function renderPalette() {
        const list = document.getElementById("palette-list");
        list.replaceChildren();
        paletteShown = paletteMatch(document.getElementById("palette-input").value).slice(0, 40);
        if (paletteSel >= paletteShown.length) paletteSel = Math.max(0, paletteShown.length - 1);
        if (!paletteShown.length) {
          const li = document.createElement("li");
          li.className = "empty";
          li.textContent = "Nothing matches. Try a channel name, a task title, or \u201cconnect\u201d.";
          list.appendChild(li);
          document.getElementById("palette-input").removeAttribute("aria-activedescendant");
          return;
        }
        paletteShown.forEach((it, i) => {
          const li = document.createElement("li");
          li.setAttribute("role", "option");
          li.id = `palette-opt-${i}`;
          li.setAttribute("aria-selected", String(i === paletteSel));
          const k = document.createElement("span");
          k.className = "pk";
          k.textContent = it.kind;
          const l = document.createElement("span");
          l.className = "pl";
          l.textContent = it.label;
          li.append(k, l);
          if (it.hint) {
            const h = document.createElement("span");
            h.className = "kbd";
            h.textContent = it.hint;
            li.appendChild(h);
          }
          li.onmousemove = () => {
            if (paletteSel !== i) {
              paletteSel = i;
              renderPalette();
            }
          };
          li.onclick = () => runPalette(i);
          list.appendChild(li);
        });
        const sel = list.children[paletteSel];
        // Screen readers follow the highlighted option while focus stays in the input.
        document.getElementById("palette-input").setAttribute("aria-activedescendant", sel ? sel.id : "");
        if (sel && sel.scrollIntoView) sel.scrollIntoView({ block: "nearest" });
      }

      function runPalette(i) {
        const it = paletteShown[i];
        if (!it) return;
        document.getElementById("palette").close();
        it.run();
      }

      function openPalette() {
        const d = document.getElementById("palette");
        if (d.open) return;
        paletteAll = paletteItems();
        paletteSel = 0;
        const input = document.getElementById("palette-input");
        input.value = "";
        renderPalette();
        d.showModal();
        input.focus();
      }

      document.getElementById("palette-input").oninput = () => {
        paletteSel = 0;
        renderPalette();
      };

      document.getElementById("palette-input").onkeydown = (e) => {
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          const n = paletteShown.length || 1;
          paletteSel = (paletteSel + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
          renderPalette();
        } else if (e.key === "Enter") {
          e.preventDefault();
          runPalette(paletteSel);
        }
      };

export { connectSnippets, mcpUrl, openConnect, openPalette, openTool, paletteAll, paletteItems, paletteMatch, paletteSel, paletteShown, renderPalette, runPalette };
