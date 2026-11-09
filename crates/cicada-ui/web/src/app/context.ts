// What every page gets from the console shell: the API, the registered types, and the refresh cycle.

import { createContext, useContext, useEffect, useRef } from 'react';
import type { Api } from '../api/client';
import type { TypeView } from '../api/wire';

export interface ConsoleContext {
  api: Api;
  /** The registered types, as of the last refresh. */
  types: TypeView[];
  /** The type a URI belongs to, with its parameters (templates never overlap). */
  typeOf(uri: string): { type: TypeView; params: Record<string, string> } | null;
  /** Reloads the types now. */
  reloadTypes(): Promise<void>;
  /** Refreshes the open page now (also while paused). */
  refresh(): void;
  /** Sets the refresh of the open page, which the shell calls on every poll. */
  setPageRefresh(refresh: (() => Promise<void>) | null): void;
}

export const Context = createContext<ConsoleContext | null>(null);

export function useConsole(): ConsoleContext {
  const ctx = useContext(Context);
  if (!ctx) throw new Error('useConsole outside the console');
  return ctx;
}

/** Registers [refresh] as the open page's refresh and loads the page once. The latest [refresh] is always called. */
export function usePageRefresh(refresh: () => Promise<void>): void {
  const { setPageRefresh, refresh: refreshNow } = useConsole();
  const latest = useRef(refresh);
  latest.current = refresh;
  useEffect(() => {
    setPageRefresh(() => latest.current());
    refreshNow();
    return () => setPageRefresh(null);
  }, [setPageRefresh, refreshNow]); // stable: runs once per page
}

/** Sets the browser tab's title for the open page. */
export function useTitle(title: string): void {
  useEffect(() => {
    document.title = `${title} — Cicada`;
  }, [title]);
}
