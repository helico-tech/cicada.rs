// Actors: instances by type template or URI prefix, with cursor pagination and new-row highlights.

import { useEffect, useMemo, useRef, useState } from 'react';
import { fetchPages } from '../api/client';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { combinePrefixes, matchTemplate, normalizePrefix, templatePrefix, withoutScheme } from '../lib/address';
import { addedKeys, removedKeys } from '../lib/diff';
import { href, replaceHash, type Query } from '../lib/routes';
import { useDebounced } from '../lib/hooks';
import { Badge, Button, EmptyState, Field, LinkButton, SearchInput, Select, SrOnly } from '../sx';
import { CreateActorDialog } from '../dialogs/CreateActor';
import { ChangeNote, Loading, Mono, PageHead } from '../ui/layout';

const PAGE_SIZE = 100;

interface Loaded {
  rows: string[];
  loadedCount: number;
  lastLoaded: string | undefined;
  next: string | null;
  impossible: boolean;
  added: Set<string>;
  removed: number;
}

export function ActorsPage({ query }: { query: Query }) {
  const ctx = useConsole();
  useTitle('Actors');
  const [type, setType] = useState(query.type ?? '');
  const [prefixText, setPrefixText] = useState(query.prefix ?? '');
  const prefix = useDebounced(prefixText, 300);
  const [pages, setPages] = useState(1);
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [creating, setCreating] = useState(false);
  const known = useRef<Set<string> | null>(null);
  const filterKey = type + '\n' + normalizePrefix(prefix);
  const lastKey = useRef(filterKey);

  usePageRefresh(async () => {
    if (lastKey.current !== filterKey) {
      // Highlights compare like with like: a new filter starts without them.
      known.current = null;
      lastKey.current = filterKey;
    }
    const listPrefix = combinePrefixes(type ? templatePrefix(type) : '', normalizePrefix(prefix));
    let items: string[] = [];
    let next: string | null = null;
    if (listPrefix !== null) {
      ({ items, next } = await fetchPages((cursor) => ctx.api.listActors({ prefix: listPrefix, cursor, limit: PAGE_SIZE }), { maxPages: pages }));
    }
    // The prefix of a template can also cover other types (actor://jobs/ covers actor://jobs/{id}/steps/{n}).
    const rows = type ? items.filter((uri) => matchTemplate(type, uri)) : items;
    const added = addedKeys(known.current, rows);
    const removed = removedKeys(known.current, rows).length;
    known.current = new Set(rows);
    setLoaded({ rows, loadedCount: items.length, lastLoaded: items.at(-1), next, impossible: listPrefix === null, added, removed });
  });

  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    replaceHash(href.actors({ type, prefix }));
    ctx.refresh();
  }, [type, prefix, pages, ctx]);

  const typeItems = useMemo(
    () => [{ value: '', label: 'All types' }, ...ctx.types.map((t) => ({ value: t.template, label: withoutScheme(t.template) }))],
    [ctx.types],
  );
  const chooseType = (v: string) => {
    setType(v);
    setPages(1);
  };

  const loadMore = () => {
    // The rows of the next page are not new, only newly loaded.
    known.current = null;
    setPages((p) => p + 1);
  };

  const shownPrefix = normalizePrefix(prefix);
  return (
    <div className="cc-page">
      <PageHead
        title="Actors"
        description="Browse actor instances by type template or URI prefix. Select an actor to inspect its state and mailbox."
        actions={<Button variant="primary" icon="plus" disabled={ctx.types.length === 0} onClick={() => setCreating(true)}>Create actor…</Button>}
      />
      <div className="cc-filters">
        <Field id="actors-type" label="Type" className="cc-filter-type">
          <Select id="actors-type" value={type} options={typeItems} onChange={chooseType} />
        </Field>
        <Field id="actors-prefix" label="URI prefix" className="cc-filter-prefix">
          <SearchInput id="actors-prefix" icon="filter" mono placeholder="actor://sync/ac" defaultValue={prefixText}
            onInput={(v) => { setPrefixText(v); setPages(1); }} />
        </Field>
        <span className="sx-spacer" />
        {loaded ? <ChangeNote added={loaded.added.size} addedWord="new actor" removed={loaded.removed} removedWord="removed" /> : null}
      </div>
      <div className="cc-tablebox">
        {!loaded ? (
          <Loading>Loading actors…</Loading>
        ) : loaded.rows.length === 0 ? (
          loaded.impossible
            ? <EmptyState title="No actors match these filters" caption="The type and the URI prefix exclude each other." />
            : shownPrefix || type
              ? <EmptyState title="No actors match these filters" caption={<>Nothing starts with <Mono>{shownPrefix || templatePrefix(type)}</Mono>{type ? ' for the selected type' : ''}. Check the prefix, or create the actor.</>} />
              : <EmptyState title="No actors yet" caption="Create one here, over HTTP or from a handler." />
        ) : (
          <div className="sx-tablewrap">
            <table className="sx-table cc-table-actors">
              <colgroup><col /><col className="cc-w-240" /><col className="cc-w-200" /><col className="cc-w-40" /></colgroup>
              <thead>
                <tr><th>Actor URI</th><th>Type template</th><th>Parameters</th><th className="is-actions"><SrOnly>Open</SrOnly></th></tr>
              </thead>
              <tbody>
                {loaded.rows.map((uri) => {
                  const resolved = ctx.typeOf(uri);
                  const isNew = loaded.added.has(uri);
                  const params = resolved ? Object.entries(resolved.params) : [];
                  return (
                    <tr key={uri} className={isNew ? 'cc-new' : undefined}>
                      <td>
                        <div className="sx-cell">
                          <a className="mono" href={href.actor(uri)} title={uri}>{uri}</a>
                          {isNew ? <Badge tone="info" size="sm">New</Badge> : null}
                        </div>
                      </td>
                      <td className="mono-sm cc-muted">{resolved ? resolved.type.template : 'no registered type'}</td>
                      <td className="caption">{params.length ? params.map(([k, v]) => `${k} = ${v}`).join(', ') : <span className="cc-subtle">none</span>}</td>
                      <td className="is-actions"><LinkButton href={href.actor(uri)} variant="ghost" size="xs" icon="chevron-right" label={'Open ' + uri} /></td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
        {loaded ? (
          <div className="sx-table-foot">
            <span className="caption sx-num cc-trunc">
              {type && loaded.rows.length !== loaded.loadedCount ? `${loaded.rows.length} shown of ${loaded.loadedCount} loaded` : `${loaded.loadedCount} loaded`}
              {loaded.next ? ` · next page starts after ${loaded.lastLoaded}` : ' · no more pages'}
            </span>
            <span className="sx-spacer" />
            {loaded.next ? <Button size="sm" variant="ghost" icon="chevron-down" onClick={loadMore}>Load more</Button> : <span className="caption">End of list</span>}
          </div>
        ) : null}
      </div>
      {creating ? <CreateActorDialog template={type} onClose={() => setCreating(false)} /> : null}
    </div>
  );
}
