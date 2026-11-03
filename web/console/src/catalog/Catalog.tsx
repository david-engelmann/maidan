// Catalog: the tools an agent can call here, exactly as MCP tools/list hands
// them out for this caller. A list with a filter; each tool opens to its
// description, its input parameters and its safety hints.
import { useMemo, useState, useSyncExternalStore } from "react";
import { CATALOG_FORBIDDEN, CatalogApi, annotationBadges, filterTools, openTool, schemaParams, summary, toolTitle, type CatalogTool } from "../core/catalog.ts";
import { useLoad } from "../hooks/useTools.ts";
import { useSession } from "../hooks/useSession.ts";
import { EmptyState } from "../shell/EmptyState.tsx";
import { LoadError } from "../shell/LoadError.tsx";

function subscribeHash(fn: () => void): () => void {
  window.addEventListener("hashchange", fn);
  return () => window.removeEventListener("hashchange", fn);
}

const toolHref = (name: string) => `#/catalog/${encodeURIComponent(name)}`;

function Badges({ tool }: { tool: CatalogTool }) {
  const badges = annotationBadges(tool.annotations);
  if (!badges.length) return null;
  return (
    <span className="hint-badges">
      {badges.map((b) => (
        <span key={b.id} className={`hint-badge ${b.id}`}>{b.label}</span>
      ))}
    </span>
  );
}

function ToolDetail({ tool }: { tool: CatalogTool }) {
  const params = schemaParams(tool.inputSchema);
  return (
    <article className="tool-detail" aria-labelledby="tool-detail-title">
      <header className="tool-detail-head">
        <h2 id="tool-detail-title">{toolTitle(tool)}</h2>
        <code className="mono-id">{tool.name}</code>
        <Badges tool={tool} />
      </header>
      {tool.description ? <p className="tool-description">{tool.description}</p> : <p className="muted">No description.</p>}
      <h3 className="tool-section">Parameters</h3>
      {params.length ? (
        <dl className="param-list">
          {params.map((p) => (
            <div key={p.name} className="param">
              <dt>
                <code>{p.name}</code>
                <span className="param-type">{p.type}</span>
                {p.required ? <span className="param-req">required</span> : <span className="param-opt">optional</span>}
              </dt>
              <dd>
                {p.description || <span className="muted">No description.</span>}
                {p.choices.length ? (
                  <span className="param-choices">
                    {p.choices.map((c) => (
                      <code key={c}>{c}</code>
                    ))}
                  </span>
                ) : null}
              </dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className="muted">This tool takes no parameters.</p>
      )}
    </article>
  );
}

export function Catalog() {
  const { store, session } = useSession();
  const api = useMemo(() => new CatalogApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const key = session.status === "signed-in" ? "catalog" : null;
  const loaded = useLoad(key, () => api.list());
  const hash = useSyncExternalStore(subscribeHash, () => window.location.hash);
  const open = openTool(hash);
  const [query, setQuery] = useState("");
  const tools = loaded.value ?? [];
  const shown = filterTools(tools, query);
  const selected = open ? (tools.find((t) => t.name === open) ?? null) : null;
  const forbidden = loaded.error === CATALOG_FORBIDDEN;

  return (
    <div className={`catalog-grid${open ? " detail-open" : ""}`}>
      <section className="catalog-list" aria-label="Tools">
        <div className="roster-tools">
          <input className="search" type="search" placeholder="Find a tool" aria-label="Find a tool by name or description" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <p className="mono roster-count" aria-live="polite">
          {loaded.status === "ready" ? (query ? `${shown.length} of ${tools.length} tools` : `${tools.length} tools`) : loaded.status === "loading" ? "Loading tools" : ""}
        </p>
        {loaded.status === "loading" ? (
          <ul className="tool-rows" aria-hidden="true">
            {[0, 1, 2, 3].map((i) => (
              <li key={i} className="tool-skeleton" />
            ))}
          </ul>
        ) : null}
        {forbidden && !loaded.value ? (
          <div className="empty-state" role="alert">
            <strong>No access to the catalog</strong>
            <p className="muted">{CATALOG_FORBIDDEN} Ask an admin for a token that holds it.</p>
          </div>
        ) : loaded.error && loaded.value ? (
          <div className="stale" role="alert">
            <p>{loaded.error}</p>
            <button type="button" className="button steel" onClick={() => void loaded.reload()}>Try again</button>
          </div>
        ) : loaded.error ? (
          <LoadError message={loaded.error} onRetry={() => void loaded.reload()} />
        ) : null}
        {loaded.status === "ready" && !shown.length ? (
          query ? (
            <EmptyState title="No matching tools" body={`No tool matches “${query}”.`} />
          ) : (
            <EmptyState title="No tools yet" body="This access lists no tools." />
          )
        ) : null}
        <ul className="tool-rows">
          {shown.map((t) => (
            <li key={t.name}>
              <a className={`tool-row${open === t.name ? " active" : ""}`} href={toolHref(t.name)} aria-current={open === t.name ? "true" : undefined}>
                <span className="who">
                  <b>{toolTitle(t)}</b>
                  <small>{summary(t.description) || t.name}</small>
                </span>
                <Badges tool={t} />
              </a>
            </li>
          ))}
        </ul>
      </section>
      {open ? (
        <section className="member-detail card" aria-label="Tool details">
          <a className="back" href="#/catalog">Catalog</a>
          {selected ? <ToolDetail key={selected.name} tool={selected} /> : loaded.status === "ready" ? <p className="muted">No tool called {open} is listed for you.</p> : loaded.error ? <LoadError message={loaded.error} onRetry={() => void loaded.reload()} /> : <p className="muted">Loading…</p>}
        </section>
      ) : (
        <section className="member-detail card wide-only" aria-hidden="true">
          <p className="muted">Pick a tool to see what it does and what it takes.</p>
        </section>
      )}
    </div>
  );
}
