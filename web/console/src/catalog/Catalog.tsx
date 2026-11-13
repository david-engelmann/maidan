// Catalog: the tools an agent can call here, exactly as MCP tools/list hands
// them out for this caller. A list with a filter; each tool opens to its
// description, its input parameters and its safety hints.
import { useMemo, useState, useSyncExternalStore } from "react";
import { CATALOG_FILTERS, CATALOG_FORBIDDEN, CatalogApi, annotationBadges, filterTools, openTool, schemaOneOfRequired, schemaParams, summary, toolProfiles, toolTitle, type CatalogFilter, type CatalogTool, type Param } from "../core/catalog.ts";
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

function oneOfNote(schema: CatalogTool["inputSchema"]): string {
  const names: string[] = [];
  for (const group of schemaOneOfRequired(schema)) {
    for (const name of group) if (!names.includes(name)) names.push(name);
  }
  return names.length ? `Requires one of: ${names.join(", ")}` : "";
}

function requirementLabel(param: Param) {
  if (param.requirement === "required") return <span className="param-req">required</span>;
  if (param.requirement === "one-of") return <span className="param-opt">one of required</span>;
  return <span className="param-opt">optional</span>;
}

function CopyToolName({ name }: { name: string }) {
  const [copied, setCopied] = useState<"idle" | "copied" | "failed">("idle");
  const copy = () => {
    let writing: Promise<void> | undefined;
    try {
      writing = navigator.clipboard?.writeText(name);
    } catch {
      setCopied("failed");
      return;
    }
    if (!writing) {
      setCopied("failed");
      return;
    }
    void writing.then(() => setCopied("copied"), () => setCopied("failed"));
  };
  return (
    <>
      {/* The whole visible phrase first, hidden context after it: an aria-label
          of "Copy tool name" over "Copy name" or "Copied" does not contain what
          is shown, and a hidden word between visible ones splits the phrase. */}
      <button type="button" className="chip-btn" onClick={copy}>
        {copied === "copied" ? (
          <>
            Copied<span className="visually-hidden"> tool name</span>
          </>
        ) : (
          <>
            Copy name<span className="visually-hidden"> of this tool</span>
          </>
        )}
      </button>
      {copied === "failed" ? <p className="err small" role="alert">This browser would not copy. Select the name and copy it by hand.</p> : null}
    </>
  );
}

function ToolDetail({ tool }: { tool: CatalogTool }) {
  const params = schemaParams(tool.inputSchema);
  const profiles = toolProfiles(tool.name);
  const note = oneOfNote(tool.inputSchema);
  return (
    <article className="tool-detail" aria-labelledby="tool-detail-title">
      <header className="tool-detail-head">
        <h2 id="tool-detail-title">{toolTitle(tool)}</h2>
        <div className="row-form">
          <code className="mono-id">{tool.name}</code>
          <CopyToolName name={tool.name} />
        </div>
        {profiles.length ? <p className="muted small">{profiles.map((p) => p.path).join(" · ")}</p> : null}
        <Badges tool={tool} />
      </header>
      {tool.description ? <p className="tool-description">{tool.description}</p> : <p className="muted">No description.</p>}
      <h3 className="tool-section">Parameters</h3>
      {note ? <p className="muted small">{note}</p> : null}
      {params.length ? (
        <dl className="param-list">
          {params.map((p) => (
            <div key={p.name} className="param">
              <dt>
                <code>{p.name}</code>
                <span className="param-type">{p.type}</span>
                {p.format ? <span className="param-type">{p.format}</span> : null}
                {requirementLabel(p)}
              </dt>
              <dd>
                {p.description || <span className="muted">No description.</span>}
                {p.defaultValue !== undefined ? <span className="muted small"> default: {p.defaultValue}</span> : null}
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

function rosterCount(status: string, total: number, shown: number, query: string, filter: CatalogFilter): string {
  if (status === "loading") return "Loading tools";
  if (status !== "ready") return "";
  const narrowed = query !== "" || filter !== "all";
  const base = narrowed ? `${shown} of ${total} tools` : `${total} tools`;
  const path = CATALOG_FILTERS.find((f) => f.id === filter)?.path;
  return path ? `${base} · ${path}` : base;
}

export function Catalog() {
  const { store, session } = useSession();
  const api = useMemo(() => new CatalogApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const key = session.status === "signed-in" ? "catalog" : null;
  const loaded = useLoad(key, () => api.list());
  const hash = useSyncExternalStore(subscribeHash, () => window.location.hash);
  const open = openTool(hash);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<CatalogFilter>("all");
  const tools = loaded.value ?? [];
  const shown = filterTools(tools, query, filter);
  const selected = open ? (tools.find((t) => t.name === open) ?? null) : null;
  const forbidden = loaded.error === CATALOG_FORBIDDEN;
  const filtering = query !== "" || filter !== "all";
  const reset = () => {
    setQuery("");
    setFilter("all");
  };

  return (
    <div className={`catalog-grid${open ? " detail-open" : ""}`}>
      <section className="catalog-list" aria-label="Tools">
        <div className="roster-tools">
          <input className="search" type="search" placeholder="Find a tool" aria-label="Find a tool by name or description" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <div className="view-pick client-pick" role="group" aria-label="Filter tools">
          {CATALOG_FILTERS.map((f) => (
            <button key={f.id} type="button" className="chip-btn" aria-pressed={filter === f.id} onClick={() => setFilter(f.id)}>{f.label}</button>
          ))}
        </div>
        <p className="mono roster-count" aria-live="polite">
          {rosterCount(loaded.status, tools.length, shown.length, query, filter)}
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
          filtering ? (
            <EmptyState title="No matching tools" body={query ? `No tool matches “${query}”.` : "No tool matches this filter."} action={<button type="button" className="button ghost" onClick={reset}>Show all tools</button>} />
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
          <EmptyState icon="catalog" title="No tool selected" body="Pick a tool to see what it does and what it takes." />
        </section>
      )}
    </div>
  );
}
