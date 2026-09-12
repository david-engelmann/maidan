# Research dump — LangChain MCP (`langchain.mcp`, Sept 2026)

**Permalink dump, not a program.** Never paste into Open Work. Recast only. Wave 1 rank is [[GitHub Roadmap]] / the live [[Open Work]] forward program. Fold Ready is KEEP/SKIP identity. Do not mint H16 / G20 / NEW-langchain / NEW-interrupt. Do not become a LangChain adapter, Deep Agents harness, or LangGraph runtime.

**Splice pickup:** open as quotes for §1 homes. Live program is [[Open Work]] Wave 1 (as of 2026-09-09: #1–#12 SHIPPED through Cluster 361; #13 G2/G4/G3/G11 is NEXT). Do not reopen J3. Do not restore `request_client` / `Mcp-Session-Id`. Do not mint a second elicitation cluster.

**Fetched 2026-09-09 ET.** Primary: [MCP in LangChain: Stateless Protocol, Elicitation, and More](https://www.langchain.com/blog/mcp-in-langchain-stateless-protocol-elicitation-and-more) (Sydney Runkle). Also: LangChain docs `langchain.mcp` (beta, `langchain[mcp]>=1.4.0`), release notes 1.4.0a2/a3, PR [langchain#39786](https://github.com/langchain-ai/langchain/pull/39786) (elicitation → LangGraph interrupt), FastMCP 4 client (protocol negotiation + cache).

---

## 1. What LangChain shipped (facts)

Three product moves on top of MCP **2026-07-28**:

| Move | What it is | Why they care |
|------|------------|---------------|
| **First-class `langchain.mcp`** | MCP leaves `langchain-mcp-adapters`; lives in main package as `MCPAdapter` over FastMCP. `MultiServerMCPClient` → one adapter / `ClientGroup`. | Agents get tools without a side package. |
| **Stateless-era client** | Negotiates modern + legacy eras per connection (`mode="auto"` / `"legacy"`). Streamable HTTP, stdio, in-memory. Auth: bearer, OAuth 2.1, M2M, CIMD. | Sticky-session servers were the scale pain; 2026 erased sessions. |
| **Elicitation via interrupts** | Mid-tool ask arrives as modern **`InputRequiredResult`** (no server→client back-channel). Adapter surfaces it as a **LangGraph `interrupt()`**; resume with `Command(resume=…)` (`accept` / `decline` / `cancel`). Opt-in (`elicitation="interrupt"`) because declaring the capability is a promise. | HITL without holding a socket. |
| **Client-side tool-list cache** | Server may advertise how long `tools/list` stays fresh; FastMCP `cache=True` + `cache_mode="use"` respects TTL. | Every agent run used to re-fetch the catalog. |

Related but out of Maidan scope: Deep Agents / LangGraph checkpointer as the *caller* runtime; TypeScript MCP adapter "soon"; docs MCP server at `https://docs.langchain.com/mcp`.

---

## 2. Recast onto existing Maidan IDs (homes)

| Home | What to thicken | Not |
|------|-----------------|-----|
| **F3 Wave 1 #1 (SHIPPED Cluster 350)** | **Interop note, one paragraph:** LangChain's elicitation interrupt is the *caller*-side loop over modern `InputRequiredResult`. Maidan's held gate is the *room*-side durable answer: `request_approval` → `{status:"input_required", gate_id}` + REST/`/ui` resolve (HMAC `requestState`, CAS). Same family (stateless mid-call ask), different durability. Do **not** mint a LangGraph-interrupt cluster. Do **not** restore session `elicitation/create`. A LangChain agent talking to Maidan still needs a path: either poll `get_approval_gate` or a later reactive wait — document that in F3 leftovers / SDK notes, not a new ID. | Second elicitation product. Restore `request_client`. LangGraph-in-the-room. |
| **MCP 2026 headline (Clusters 300–303, DONE)** | LangChain validates the bet: Tier-1 SDKs + ChatGPT MCP traffic are climbing on the **stateless** core Maidan already shipped (`SUPPORTED` includes `2026-07-28`, cold POST, SEP-2243 headers). Keep `2024-11-05` on explicit request (Cluster 350 decision) — FastMCP also keeps a legacy era. | Reopen J3. Drop 2024. |
| **F3 deferred list-cache (`ttlMs` / `cacheScope`)** | **Re-weight, do not reopen as a program:** Splice Prompt says do not reopen. Honest update: LangChain clients will *prefer* servers that advertise list TTL. When a Wave 1 leftover or a tiny hardening cluster is free, land **server-side** `ttlMs`/`cacheScope` on `tools/list` (and resources/prompts if cheap) so FastMCP/LangChain caches work. Still niche vs G2 (#13). Park under F3 leftovers, not a NEW row. | Cache product. Client-side Redis in Maidan. |
| **N8 / ext-auth (Wave 1 #4 SHIPPED; F3 thickeners)** | FastMCP OAuth 2.1 / resource indicators / per-server `ClientGroup` credentials → same family as F3 **ext-auth step-up** (elicit, do not silently expand). Attenuation chrome stays N8. | Cedar engine. Maidan-as-OAuth-IdP SKU. |
| **G-dev-1 Wave 1 #11 (SHIPPED Cluster 360) deferred** | LangChain's "organizing context / Deep Agents" posts reinforce: pack framing + recency matter; pushback-on-framing is still the deferred G-dev-1 contract language (BullshitBench). Land that language when touching the pack next — do not invent a Deep Agents skill cluster. | Deep Agents SKU. ACE playbook. |
| **H12 Wave 1 #3 (SHIPPED Cluster 352)** | A2A `input-required` task list already surfaces pending gates. LangChain interrupt is MCP-caller shaped; H12 stays the external-agent discovery surface. | Duplicate HITL list on MCP. |

---

## 3. Maidan vs LangChain elicitation (honesty table)

| Axis | LangChain `langchain.mcp` | Maidan (Cluster 350+) |
|------|---------------------------|------------------------|
| Who am I? | **Client** calling MCP tools | **Server** (room) agents claim into |
| Pause shape | LangGraph `interrupt()` in the caller process | Durable `ApprovalGate` row + `input_required` tool result |
| Survives drop? | Needs checkpointer | Yes — gate is in the store |
| Answer path | `Command(resume=…)` same graph | REST `/approval-gates/:id/answer`, `/ui` Approvals tab, A2A `input-required` (H12) |
| Wire era | Modern `InputRequiredResult` retry; no S2C back-channel | S2C `request_client` **deleted** on purpose (350.5); poll/query instead |
| Capability promise | Opt-in `elicitation="interrupt"` | Gate tools always available under caps; N6 blocks `claim_next` while pending |

**Verdict:** Maidan is not behind on the protocol bet. The room model is *stricter* (durable, queryable, claim-gated). The gap to watch is **client interop**: a LangChain agent that expects to *resume the same `tools/call`* with attached answers may not map 1:1 onto Maidan's "open gate + poll" tool pair. That is an F3/SDK contract note, not a rewrite.

---

## 4. What NOT to build

- A LangChain / Deep Agents / LangGraph runtime inside Maidan.
- Restoring `request_client`, sampling, roots, or session sticky elicitation.
- A second elicitation cluster or `NEW-interrupt`.
- Reopening `ttlMs`/`cacheScope` as a ranked Wave row (park as F3 leftover only).
- Treating LangChain blog velocity as a reason to pause Wave 1 #13.

---

## 5. Suggested next-agent actions (tiny)

1. Keep shipping **Wave 1 #13** (G2/G4/G3/G11 wait-edges) — next ranked.
2. When next touching F3 leftovers or MCP catalog: add one sentence to Capabilities / Protocols on LangChain interrupt ↔ gate poll interop; optionally land `ttlMs` on `tools/list` if it stays <1 PR.
3. When next touching G-dev-1: land the deferred **pushback** contract language (challenge framing; uncertainty → ask/stuck).
4. Stale doc nit: `crates/maidan-server/src/openapi/mod.rs` MCP blurb — rewrite to 2026 cold POST + durable gate (this PR does that).

## Stamp

MaidanFadin, 2026-09-09. Evidence dump against public LangChain blog + docs + PR #39786 and Maidan HEAD Cluster 361. Not a rank. Not yet spliced.
