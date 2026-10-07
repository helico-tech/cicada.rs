// Pending: the pending messages of every actor (GET /messages), instance by instance in URI order, each mailbox in
// delivery order, filtered by type template, URI prefix and message type, with cursor pagination.

import { useEffect, useMemo, useRef, useState } from 'react';
import { fetchPages } from '../api/client';
import type { MessageView } from '../api/generated';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { combinePrefixes, matchTemplate, normalizePrefix, templatePrefix, withoutScheme } from '../lib/address';
import { addedKeys, removedKeys } from '../lib/diff';
import { relative, shortDateTime } from '../lib/format';
import { useDebounced } from '../lib/hooks';
import { href, replaceHash, type Query } from '../lib/routes';
import { Badge, Button, Card, cx, EmptyState, Icon, SearchInput, Select, SrOnly } from '../halo';
import { ChangeNote, Loading, PageHead } from '../ui/layout';

const PAGE_SIZE = 100;

interface Loaded {
  messages: MessageView[];
  next: string | null;
  impossible: boolean;
  added: Set<string>;
  removed: number;
}

export function PendingPage({ query }: { query: Query }) {
  const ctx = useConsole();
  useTitle('Pending');
  const [type, setType] = useState(query.type ?? '');
  const [prefixText, setPrefixText] = useState(query.prefix ?? '');
  const prefix = useDebounced(prefixText, 300);
  const [messageType, setMessageType] = useState('all');
  const [pages, setPages] = useState(1);
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const known = useRef<Set<string> | null>(null);
  const filterKey = type + '\n' + normalizePrefix(prefix);
  const lastKey = useRef(filterKey);

  usePageRefresh(async () => {
    if (lastKey.current !== filterKey) {
      known.current = null;
      lastKey.current = filterKey;
    }
    const listPrefix = combinePrefixes(type ? templatePrefix(type) : '', normalizePrefix(prefix));
    let items: MessageView[] = [];
    let next: string | null = null;
    if (listPrefix !== null) {
      ({ items, next } = await fetchPages((cursor) => ctx.api.listPending({ prefix: listPrefix, cursor, limit: PAGE_SIZE }), { maxPages: pages }));
    }
    // A template's prefix can also cover other types (actor://jobs/ covers actor://jobs/{id}/steps/{n}).
    const messages = type ? items.filter((m) => matchTemplate(type, m.to)) : items;
    const ids = messages.map((m) => m.id);
    const added = addedKeys(known.current, ids);
    const removed = removedKeys(known.current, ids).length;
    known.current = new Set(ids);
    setLoaded({ messages, next, impossible: listPrefix === null, added, removed });
  });

  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    replaceHash(href.pending({ type, prefix }));
    ctx.refresh();
  }, [type, prefix, pages, ctx]);

  const typeOptions = useMemo(
    () => [{ value: '', label: 'All types' }, ...ctx.types.map((t) => ({ value: t.template, label: withoutScheme(t.template) }))],
    [ctx.types],
  );
  const messageTypeOptions = useMemo(() => {
    const names = new Set<string>();
    for (const t of ctx.types) if (!type || t.template === type) for (const name of Object.keys(t.messages)) names.add(name);
    return [{ value: 'all', label: 'All message types' }, ...[...names].sort().map((n) => ({ value: n, label: n }))];
  }, [ctx.types, type]);

  const rows = loaded ? (messageType === 'all' ? loaded.messages : loaded.messages.filter((m) => m.type === messageType)) : [];
  const filtered = Boolean(loaded?.impossible || type || normalizePrefix(prefix) || messageType !== 'all');
  return (
    <div>
      <PageHead title="Pending" description="Messages waiting in any mailbox, actor by actor, each in delivery order. Open a message to see whether it is running, or to cancel it." />
      <Card title="Pending Messages" subtitle="Sorted by actor URI, then deliver at · cursor pagination" flush>
        <div className="cc-filterbar cc-filterbar-fields">
          <div className="cc-field">
            <span className="cc-label cc-muted">Type</span>
            <Select label="Filter by type template" value={type} options={typeOptions} onChange={(v) => { setType(v); setMessageType('all'); setPages(1); }} />
          </div>
          <div className="cc-field cc-filter-prefix">
            <span className="cc-label cc-muted">Actor URI prefix</span>
            <SearchInput size="sm" placeholder="actor://sync/" label="Actor URI prefix" defaultValue={prefixText} onInput={(v) => { setPrefixText(v); setPages(1); }} />
          </div>
          <div className="cc-field">
            <span className="cc-label cc-muted">Message type</span>
            <Select label="Message type" value={messageType} options={messageTypeOptions} onChange={setMessageType} />
          </div>
        </div>
        {loaded ? <ChangeNote added={loaded.added.size} addedWord="new message" removed={loaded.removed} removedWord="left" /> : null}
        {!loaded ? (
          <Loading>Loading pending messages…</Loading>
        ) : rows.length === 0 ? (
          loaded.impossible
            ? <EmptyState title="No messages match these filters" caption="The type and the URI prefix exclude each other." />
            : filtered
              ? <EmptyState title="No pending messages match these filters" caption={loaded.next ? 'Load more to look further.' : 'Widen the filters to look further.'} />
              : <EmptyState title="No pending messages" caption="Every mailbox is empty. Finished messages are found by id or under Failures." />
        ) : (
          <div className="cc-scroll-x">
            <table className="ha-table cc-table cc-table-pending">
              <thead>
                <tr>
                  <th className="cc-col-actor">Actor URI</th><th>Type</th><th>Deliver at</th><th className="cc-num">Attempt</th>
                  <th className="cc-num">Failures</th><th className="cc-col-error">Last error</th><th>Correlation</th>
                  <th className="cc-col-icon"><SrOnly>Open</SrOnly></th>
                </tr>
              </thead>
              <tbody>
                {rows.map((m) => {
                  const isNew = loaded.added.has(m.id);
                  const due = Date.parse(m.deliverAt) <= Date.now();
                  return (
                    <tr key={m.id} className={isNew ? 'cc-new' : undefined}>
                      <td>
                        <div className="cc-row cc-nowrap-cell">
                          <a className="cc-link mono cc-small" href={href.actor(m.to)}>{m.to}</a>
                          {isNew ? <Badge tone="primary">New</Badge> : null}
                        </div>
                      </td>
                      <td><a className="cc-link mono cc-strong" href={href.message(m.id)}>{m.type}</a></td>
                      <td>
                        <div className="ha-num cc-nowrap">{shortDateTime(m.deliverAt)}</div>
                        <div className={cx('cc-caption cc-nowrap', !due && 'cc-ink')}>{due ? `due ${relative(m.deliverAt)}` : relative(m.deliverAt)}</div>
                      </td>
                      <td className="ha-num cc-num">{String(m.attempt)}</td>
                      <td className={cx('ha-num cc-num', m.failures > 0 && 'cc-critical cc-strong')}>{String(m.failures)}</td>
                      <td className="cc-col-error">{m.lastError ? <span className="cc-trunc mono" title={m.lastError}>{m.lastError}</span> : <span className="cc-faint">—</span>}</td>
                      <td><span className="mono cc-muted">{m.correlationId ?? '—'}</span></td>
                      <td className="cc-col-icon"><a href={href.message(m.id)} className="cc-icon-link" aria-label={`Open ${m.type} message`}><Icon name="chevron-right" size={16} /></a></td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
        {loaded ? (
          <div className="cc-card-foot">
            <span className="cc-caption">
              {rows.length !== loaded.messages.length ? `${rows.length} shown of ${loaded.messages.length} loaded` : `${loaded.messages.length} loaded`}
              {loaded.next ? ` · next page starts after ${loaded.messages.at(-1)?.to ?? ''}` : ' · no more pages'}
            </span>
            {loaded.next
              ? <Button size="sm" icon="chevron-down" onClick={() => { known.current = null; setPages((p) => p + 1); }}>Load More</Button>
              : <span className="cc-caption">End of list</span>}
          </div>
        ) : null}
      </Card>
    </div>
  );
}
