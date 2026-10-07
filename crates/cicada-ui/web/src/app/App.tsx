// The console shell: the sidebar and top bar, routing of the URL fragment to a page, polling, sign-in and the
// refresh notices.

import { useCallback, useEffect, useMemo, useReducer, useRef, useState, type ReactNode } from 'react';
import { ApiError, createApi } from '../api/client';
import type { TypeView } from '../api/generated';
import { isActorUri, resolveType } from '../lib/address';
import { capitalize, clock } from '../lib/format';
import { Poller } from '../lib/poller';
import { href, parseRoute, type Route } from '../lib/routes';
import { session } from '../lib/storage';
import { useTheme } from '../lib/theme';
import { Button, Callout, ProblemCallout, Sidebar } from '../halo';
import { SignInDialog } from '../dialogs/SignIn';
import { ActorPage } from '../pages/ActorPage';
import { ActorsPage } from '../pages/ActorsPage';
import { FailuresPage } from '../pages/FailuresPage';
import { MessagePage } from '../pages/MessagePage';
import { PendingPage } from '../pages/PendingPage';
import { TypesPage } from '../pages/TypesPage';
import { Context, type ConsoleContext } from './context';
import { TopBar, type Health } from './TopBar';

const TOKEN_KEY = 'cicada-console-token';
const TYPES_EVERY = 12; // ticks: once a minute at the 5 s interval
const FAILURE_COUNT_EVERY = 6; // ticks: every 30 s
const DAY_MS = 24 * 3600_000;

function useHashRoute(): Route {
  const [hash, setHash] = useState(() => location.hash);
  useEffect(() => {
    const onChange = () => {
      setHash(location.hash);
      window.scrollTo(0, 0);
    };
    window.addEventListener('hashchange', onChange);
    return () => window.removeEventListener('hashchange', onChange);
  }, []);
  return useMemo(() => parseRoute(hash), [hash]);
}

const NAV: Record<Route['name'], string> = { actors: 'actors', actor: 'actors', message: 'actors', pending: 'pending', failures: 'failures', types: 'types' };

export function App({ apiPath }: { apiPath: string }) {
  const theme = useTheme();
  const route = useHashRoute();
  const token = useRef<string | null>(session.get(TOKEN_KEY));
  const api = useMemo(() => createApi({ base: apiPath, token: () => token.current }), [apiPath]);
  const [types, setTypes] = useState<TypeView[]>([]);
  const typesRef = useRef<TypeView[] | null>(null);
  const pageRefresh = useRef<(() => Promise<void>) | null>(null);
  const tick = useRef(0);
  const [health, setHealth] = useState<Health | null>(null);
  const [failureCount, setFailureCount] = useState('');
  const [notice, setNotice] = useState<ReactNode>(null);
  const [signingIn, setSigningIn] = useState(false);
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  const searchRef = useRef<HTMLInputElement>(null);

  const reloadTypes = useCallback(async () => {
    const next = (await api.types()).items;
    typesRef.current = next;
    setTypes((current) => (JSON.stringify(current.map((t) => t.hash)) === JSON.stringify(next.map((t) => t.hash)) ? current : next));
  }, [api]);

  const refreshAll = useCallback(async () => {
    tick.current += 1;
    const n = tick.current;
    const jobs: Array<Promise<unknown>> = [checkHealth()];
    if (n % FAILURE_COUNT_EVERY === 1) jobs.push(countFailures());
    const current = pageRefresh.current;
    // Pages resolve URIs against the types, so the first load waits for them.
    const work = (async () => {
      if (!typesRef.current || n % TYPES_EVERY === 0) await reloadTypes();
      await current?.();
    })();
    const [result] = await Promise.allSettled([work, ...jobs]);
    if (!result || result.status === 'fulfilled') {
      setNotice(null);
      return;
    }
    const error: unknown = result.reason;
    if (error instanceof ApiError && error.status === 401) {
      setNotice(<Callout kind="user" title="Sign in required" detail="This node’s API needs credentials." action={<Button size="sm" onClick={() => setSigningIn(true)}>Sign In</Button>} />);
      setSigningIn(true);
    } else if (error instanceof ApiError && error.temporary) {
      const last = poller.lastAt ? `Showing data from ${clock(poller.lastAt)}; ` : '';
      setNotice(<Callout kind="temp" role="status" detail={error.status === 0
        ? `Couldn’t refresh — the node is unreachable. ${last}polling continues.`
        : `Couldn’t refresh — the store is temporarily unavailable. ${last}polling continues.`} />);
    } else {
      if (!(error instanceof ApiError)) console.error(error);
      const title = error instanceof ApiError ? error.title : error instanceof Error ? error.message : String(error);
      setNotice(<ProblemCallout error={error} title={'Couldn’t refresh — ' + title} />);
    }
    throw error;

    async function checkHealth() {
      try {
        const h = await api.health();
        setHealth({ up: true, node: h.node, state: capitalize(h.state) });
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) return;
        setHealth((prev) => ({ up: false, node: prev?.node, detail: e instanceof ApiError ? e.detail || e.title : String(e) }));
      }
    }

    async function countFailures() {
      try {
        const since = new Date(Date.now() - DAY_MS).toISOString();
        const result = await api.failures({ since, limit: 1000 });
        const count = result.items.length;
        setFailureCount(count === 0 ? '' : result.next ? `${count}+` : String(count));
      } catch {
        // The count is a hint; the page itself reports errors.
      }
    }
  }, [api, reloadTypes]);

  // The poller calls the latest refreshAll through this ref (refreshAll reads the poller only when a refresh failed).
  const refreshAllRef = useRef(refreshAll);
  refreshAllRef.current = refreshAll;
  const [poller] = useState(() => new Poller(() => refreshAllRef.current(), () => rerender()));
  useEffect(() => () => poller.dispose(), [poller]);

  const refresh = useCallback(() => void poller.refresh(), [poller]);
  const setPageRefresh = useCallback((fn: (() => Promise<void>) | null) => {
    pageRefresh.current = fn;
  }, []);

  const ctx = useMemo<ConsoleContext>(() => ({
    api,
    types,
    typeOf: (uri: string) => resolveType(types, uri),
    reloadTypes,
    refresh,
    setPageRefresh,
  }), [api, types, reloadTypes, refresh, setPageRefresh]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    document.addEventListener('keydown', onKey);
    const clockTimer = setInterval(rerender, 1000); // "2 s ago"
    return () => {
      document.removeEventListener('keydown', onKey);
      clearInterval(clockTimer);
    };
  }, []);

  useEffect(() => setNotice(null), [route]);

  const page = (() => {
    switch (route.name) {
      case 'actor': return <ActorPage key={'actor:' + route.uri} uri={route.uri} />;
      case 'message': return <MessagePage key={'message:' + route.id} id={route.id} />;
      case 'pending': return <PendingPage key="pending" query={route.query} />;
      case 'failures': return <FailuresPage key="failures" query={route.query} />;
      case 'types': return <TypesPage key="types" query={route.query} />;
      default: return <ActorsPage key="actors" query={route.query} />;
    }
  })();

  return (
    <Context.Provider value={ctx}>
      <div className="cc-app">
        <Sidebar
          brand={{ name: 'Cicada', subtitle: 'Actor console', letter: 'C' }}
          active={NAV[route.name]}
          sections={[
            { title: 'Runtime', items: [
              { key: 'actors', icon: 'grid', label: 'Actors', href: href.actors() },
              { key: 'pending', icon: 'clock', label: 'Pending', href: href.pending() },
              { key: 'failures', icon: 'alert', label: 'Failures', href: href.failures(), count: failureCount },
            ] },
            { title: 'Reference', items: [{ key: 'types', icon: 'tag', label: 'Types', href: href.types() }] },
          ]}
        />
        <main className="cc-main">
          <TopBar
            poller={poller} health={health} dark={theme.dark} onToggleTheme={theme.toggle} searchRef={searchRef}
            onSearch={(text) => {
              location.hash = isActorUri(text) ? href.actor(text) : href.message(text);
            }}
          />
          <div className="cc-page-notice">{notice}</div>
          <div className="cc-page">{page}</div>
        </main>
      </div>
      {signingIn ? (
        <SignInDialog
          onClose={() => setSigningIn(false)}
          onSubmit={(value) => {
            token.current = value || null;
            session.set(TOKEN_KEY, token.current);
            setSigningIn(false);
            refresh();
          }}
        />
      ) : null}
    </Context.Provider>
  );
}
