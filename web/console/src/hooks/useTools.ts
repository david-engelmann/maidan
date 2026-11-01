import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ToolsApi } from "../core/tools.ts";
import type { Loaded } from "./usePeople.ts";
import { useSession } from "./useSession.ts";

/** One ToolsApi per session store. */
export function useToolsApi(): ToolsApi {
  const { store } = useSession();
  return useMemo(() => new ToolsApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
}

/**
 * A read that runs when its key changes and again on reload. A null key
 * means there is nothing to read yet. A failed reload keeps the last value.
 */
export function useLoad<T>(key: string | null, read: () => Promise<T>): Loaded<T> & { reload: () => Promise<void> } {
  const [state, setState] = useState<{ key: string | null; value: Loaded<T> }>({ key: null, value: { status: "loading", value: null, error: null } });
  const gen = useRef(0);
  const readRef = useRef(read);
  useEffect(() => {
    readRef.current = read;
  });
  const load = useCallback(async () => {
    if (key === null) return;
    const mine = ++gen.current;
    try {
      const value = await readRef.current();
      if (mine === gen.current) setState({ key, value: { status: "ready", value, error: null } });
    } catch (e) {
      if (mine !== gen.current) return;
      const error = e instanceof Error ? e.message : "Could not load this";
      setState((s) => (s.key === key && s.value.value !== null ? { key, value: { ...s.value, error } } : { key, value: { status: "error", value: null, error } }));
    }
  }, [key]);
  useEffect(() => {
    const t = setTimeout(() => void load(), 0);
    return () => clearTimeout(t);
  }, [load]);
  const value: Loaded<T> = state.key === key ? state.value : { status: "loading", value: null, error: null };
  return { ...value, reload: load };
}
