// Pending: the pending messages of every actor (GET /messages), instance by instance in URI order, each mailbox in
// delivery order, filtered by type template, URI prefix and message type, with cursor pagination.

import { useEffect, useMemo, useRef, useState } from 'react';
import { fetchPages } from '../api/client';
import type { MessageView } from '../api/wire';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { combinePrefixes, matchTemplate, normalizePrefix, templatePrefix, withoutScheme } from '../lib/address';
import { addedKeys, removedKeys } from '../lib/diff';
import { relative, shortDateTime } from '../lib/format';
import { useDebounced } from '../lib/hooks';
import { href, replaceHash, type Query } from '../lib/routes';
import { Badge, Button, cx, EmptyState, Failures, Field, LinkButton, SearchInput, Select, SrOnly } from '../sx';
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
    <div className="cc-page">
      <PageHead title="Pending" description="Messages waiting in any mailbox, actor by actor, each in delivery order. Open a message to see whether it is running, or to cancel it." />
      <div className="cc-filters">
        <Field id="pending-type" label="Type" className="cc-filter-type">
          <Select id="pending-type" value={type} options={typeOptions} onChange={(v) => { setType(v); setMessageType('all'); setPages(1); }} />
        </Field>
        <Field id="pending-prefix" label="Actor URI prefix" className="cc-filter-prefix">
          <SearchInput id="pending-prefix" icon="filter" mono placeholder="actor://sync/" defaultValue={prefixText} onInput={(v) => { setPrefixText(v); setPages(1); }} />
        </Field>
        <Field id="pending-message-type" label="Message type" className="cc-filter-type">
          <Select id="pending-message-type" value={messageType} options={messageTypeOptions} onChange={setMessageType} />
        </Field>
        <span className="sx-spacer" />
        {loaded ? <ChangeNote added={loaded.added.size} addedWord="new message" removed={loaded.removed} removedWord="left" /> : null}
      </div>
      <div className="cc-tablebox">
        {!loaded ? (
          <Loading>Loading pending messages…</Loading>
        ) : rows.length === 0 ? (
          loaded.impossible
            ? <EmptyState title="No messages match these filters" caption="The type and the URI prefix exclude each other." />
            : filtered
              ? <EmptyState title="No pending messages match these filters" caption={loaded.next ? 'Load more to look further.' : 'Widen the filters to look further.'} />
              : <EmptyState title="No pending messages" caption="Every mailbox is empty. Finished messages are found by id or under Failures." />
        ) : (
          <div className="sx-tablewrap">
            <table className="sx-table cc-table-pending">
              <colgroup>
                <col /><col className="cc-w-160" /><col className="cc-w-150" /><col className="cc-w-72" /><col className="cc-w-80" />
                <col className="cc-w-200" /><col className="cc-w-140" /><col className="cc-w-40" />
              </colgroup>
              <thead>
                <tr>
                  <th>Actor URI</th><th>Type</th><th>Deliver at</th><th className="is-num">Attempt</th>
                  <th className="is-num">Failures</th><th>Last error</th><th>Correlation</th>
                  <th className="is-actions"><SrOnly>Open</SrOnly></th>
                </tr>
              </thead>
              <tbody>
                {rows.map((m) => {
                  const isNew = loaded.added.has(m.id);
                  const due = Date.parse(m.deliverAt) <= Date.now();
                  return (
                    <tr key={m.id} className={cx('cc-tall', isNew && 'cc-new')}>
                      <td>
                        <div className="sx-cell">
                          <a className="mono" href={href.actor(m.to)} title={m.to}>{m.to}</a>
                          {isNew ? <Badge tone="info" size="sm">New</Badge> : null}
                        </div>
                      </td>
                      <td><a className="label" href={href.message(m.id)} title={m.type}>{m.type}</a></td>
                      <td>
                        <div className="cc-stack">
                          <span className="sx-num">{shortDateTime(m.deliverAt)}</span>
                          <span className={cx('caption', due ? 'cc-subtle' : 'cc-ink')}>{due ? `due ${relative(m.deliverAt)}` : relative(m.deliverAt)}</span>
                        </div>
                      </td>
                      <td className="is-num sx-num">{String(m.attempt)}</td>
                      <td className="is-num"><Failures n={m.failures} /></td>
                      <td>{m.lastError ? <span className="mono-sm cc-danger-text" title={m.lastError}>{m.lastError}</span> : <span className="cc-subtle">—</span>}</td>
                      <td className="mono-sm cc-muted">{m.correlationId ?? '—'}</td>
                      <td className="is-actions"><LinkButton href={href.message(m.id)} variant="ghost" size="xs" icon="chevron-right" label={`Open ${m.type} message`} /></td>
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
              {rows.length !== loaded.messages.length ? `${rows.length} shown of ${loaded.messages.length} loaded` : `${loaded.messages.length} loaded`}
              {loaded.next ? ` · next page starts after ${loaded.messages.at(-1)?.to ?? ''}` : ' · no more pages'}
            </span>
            <span className="sx-spacer" />
            {loaded.next
              ? <Button size="sm" variant="ghost" icon="chevron-down" onClick={() => { known.current = null; setPages((p) => p + 1); }}>Load more</Button>
              : <span className="caption">End of list</span>}
          </div>
        ) : null}
      </div>
    </div>
  );
}
