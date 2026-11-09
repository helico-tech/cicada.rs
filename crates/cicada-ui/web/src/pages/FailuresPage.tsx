// Failures: messages that ended as failed, newest first. GET /failures lists oldest first from `since`, so the page
// loads the whole window once, then only what finished after the newest it has, and sorts on the client.

import { useEffect, useMemo, useRef, useState } from 'react';
import { fetchPages } from '../api/client';
import type { OutcomeView } from '../api/wire';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { normalizePrefix } from '../lib/address';
import { count, dateTime, localInputValue, relative, shortDateTime } from '../lib/format';
import { useDebounced } from '../lib/hooks';
import { href, replaceHash, type Query } from '../lib/routes';
import { Badge, Button, Callout, cx, EmptyState, Field, Icon, LinkButton, OkMark, SearchInput, Segmented, Select, SrOnly, StatusBadge } from '../sx';
import { ChangeNote, Loading, PageHead } from '../ui/layout';

type Since = '1h' | '24h' | '7d' | 'custom';
const SINCE: Record<Exclude<Since, 'custom'>, number> = { '1h': 3600_000, '24h': 24 * 3600_000, '7d': 7 * 24 * 3600_000 };
const SINCE_LABEL: Record<Exclude<Since, 'custom'>, string> = { '1h': 'the last hour', '24h': 'the last 24 h', '7d': 'the last 7 days' };
const PAGE_LIMIT = 1000;
const MAX_PAGES = 10;
const SHOW_STEP = 50;

export function FailuresPage({ query }: { query: Query }) {
  const ctx = useConsole();
  useTitle('Failures');
  const [prefixText, setPrefixText] = useState(query.prefix ?? '');
  const prefix = normalizePrefix(useDebounced(prefixText, 300));
  const [type, setType] = useState('all');
  const [since, setSince] = useState<Since>('24h');
  const [from, setFrom] = useState(() => localInputValue(Date.now() - 2 * 24 * 3600_000));
  const [to, setTo] = useState('');
  const [byId, setById] = useState<ReadonlyMap<string, OutcomeView> | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [added, setAdded] = useState<ReadonlySet<string>>(new Set());
  const [shown, setShown] = useState(SHOW_STEP);
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const loadedKey = useRef<string | null>(null);
  const current = useRef<ReadonlyMap<string, OutcomeView>>(new Map());

  const windowStart = (): Date => {
    if (since !== 'custom') return new Date(Date.now() - SINCE[since]);
    const d = new Date(from);
    return Number.isNaN(d.getTime()) ? new Date(0) : d;
  };

  usePageRefresh(async () => {
    const start = windowStart();
    const key = `${prefix}|${since}|${since === 'custom' ? from : ''}`;
    const list = (after: Date) => fetchPages((cursor) => ctx.api.failures({ prefix, since: after.toISOString(), cursor, limit: PAGE_LIMIT }), { maxPages: MAX_PAGES });
    if (key !== loadedKey.current) {
      const { items, truncated: cut } = await list(start);
      current.current = new Map(items.map((f) => [f.id, f]));
      loadedKey.current = key;
      setTruncated(cut);
      setAdded(new Set());
      setShown(SHOW_STEP);
      setOpen(new Set());
    } else {
      // `since` is inclusive, so the newest known failure comes back too; ids deduplicate it.
      let newest = start;
      for (const f of current.current.values()) if (new Date(f.finishedAt) > newest) newest = new Date(f.finishedAt);
      const { items } = await list(newest);
      const next = new Map(current.current);
      setAdded(new Set(items.filter((f) => !next.has(f.id)).map((f) => f.id)));
      for (const f of items) next.set(f.id, f);
      // A sliding window drops what fell out of it.
      for (const [id, f] of next) if (new Date(f.finishedAt) < start) next.delete(id);
      current.current = next;
    }
    setById(current.current);
  });

  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    replaceHash(href.failures({ prefix }));
    ctx.refresh();
  }, [prefix, since, from, ctx]);

  const typeOptions = useMemo(() => {
    const names = new Set<string>();
    for (const t of ctx.types) for (const name of Object.keys(t.messages)) names.add(name);
    for (const f of byId?.values() ?? []) names.add(f.message.type);
    return [{ value: 'all', label: 'All message types' }, ...[...names].sort().map((n) => ({ value: n, label: n }))];
  }, [ctx.types, byId]);

  const rangeLabel = since === 'custom' ? `${shortDateTime(windowStart())}${to ? ' – ' + shortDateTime(to) : ''}` : SINCE_LABEL[since];
  const toDate = since === 'custom' && to ? new Date(to) : null;
  const rows = [...(byId?.values() ?? [])]
    .filter((f) => type === 'all' || f.message.type === type)
    .filter((f) => !toDate || new Date(f.finishedAt) <= toDate)
    .sort((a, b) => b.finishedAt.localeCompare(a.finishedAt) || b.id.localeCompare(a.id));
  const visible = rows.slice(0, shown);

  return (
    <div className="cc-page">
      <PageHead title="Failures" description="Messages that finished as failed, newest first. Open a row for the full envelope and retry history." />
      <div className="cc-filters">
        <Field id="failures-prefix" label="Actor URI prefix" className="cc-filter-prefix">
          <SearchInput id="failures-prefix" icon="filter" mono placeholder="actor://sync/" defaultValue={prefixText} onInput={setPrefixText} />
        </Field>
        <Field id="failures-type" label="Message type" className="cc-filter-type">
          <Select id="failures-type" value={type} options={typeOptions} onChange={(v) => { setType(v); setShown(SHOW_STEP); }} />
        </Field>
        <Field label="Since">
          <Segmented label="Since" value={since} onChange={setSince}
            items={[{ value: '1h', label: '1 hour' }, { value: '24h', label: '24 hours' }, { value: '7d', label: '7 days' }, { value: 'custom', label: 'Custom' }]} />
        </Field>
        {since === 'custom' ? (
          <>
            <Field id="failures-from" label="From">
              <input id="failures-from" className="sx-input sx-num cc-at" type="datetime-local" value={from} onChange={(e) => setFrom(e.target.value)} />
            </Field>
            <Field id="failures-to" label="To">
              <input id="failures-to" className="sx-input sx-num cc-at" type="datetime-local" value={to} onChange={(e) => setTo(e.target.value)} />
            </Field>
          </>
        ) : null}
        <span className="sx-spacer" />
        <div className="cc-filters-end">
          <ChangeNote added={added.size} addedWord="new failure" />
          {byId ? (
            rows.length
              ? <Badge tone="danger" solid icon="error">{count(rows.length, 'failure')}</Badge>
              : <Badge tone="success" icon="success">No failures</Badge>
          ) : null}
        </div>
      </div>
      <p className="caption cc-muted cc-flat">{`Since ${rangeLabel} · ${prefix || 'all actors'} · ${type === 'all' ? 'all types' : type}`}</p>
      {truncated ? (
        <Callout kind="info" title={`More than ${(PAGE_LIMIT * MAX_PAGES).toLocaleString('en')} failures in this range`}
          detail="Only the oldest ones are loaded, so the newest may be missing. Narrow the range or the prefix." />
      ) : null}
      <div className="cc-tablebox">
        {!byId ? (
          <Loading>Loading failures…</Loading>
        ) : rows.length === 0 ? (
          <EmptyState mark={<OkMark />} title={`No failures since ${rangeLabel}`}
            caption={prefix || type !== 'all' ? 'For these filters. Widen them to look further.' : 'Widen the range to look further back.'} />
        ) : (
          <>
            <div className="sx-tablewrap">
              <table className="sx-table cc-table-failures">
                <colgroup>
                  <col className="cc-w-150" /><col className="cc-w-240" /><col className="cc-w-150" /><col className="cc-w-100" /><col /><col className="cc-w-80" />
                </colgroup>
                <thead>
                  <tr>
                    <th>Finished at</th><th>Actor URI</th><th>Message type</th><th>Status</th><th>Error</th>
                    <th className="is-actions"><SrOnly>Open</SrOnly></th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((f) => {
                    const isOpen = open.has(f.id);
                    const error = f.error ?? '—';
                    const toggle = () => setOpen((s) => {
                      const next = new Set(s);
                      if (isOpen) next.delete(f.id);
                      else next.add(f.id);
                      return next;
                    });
                    return (
                      <tr key={f.id} className={cx('cc-tall cc-top', added.has(f.id) && 'cc-new')}>
                        <td><div className="cc-stack"><span className="sx-num">{shortDateTime(f.finishedAt)}</span><span className="caption cc-subtle">{relative(f.finishedAt)}</span></div></td>
                        <td><a className="mono-sm" href={href.actor(f.actor)} title={f.actor}>{f.actor}</a></td>
                        <td className="label" title={f.message.type}>{f.message.type}</td>
                        <td><StatusBadge status={f.status} size="sm" /></td>
                        <td className={isOpen ? 'cc-wrap' : undefined}>
                          <button type="button" className="cc-row-btn cc-error-toggle" aria-expanded={isOpen} onClick={toggle}>
                            <span className="cc-toggle-icon"><Icon name={isOpen ? 'chevron-down' : 'chevron-right'} size={14} /></span>
                            {isOpen
                              ? <pre className="mono-sm cc-error-full">{`${error}\n\nFinished ${dateTime(f.finishedAt)} · attempt ${f.message.attempt} · ${count(f.message.failures, 'earlier failure')}`}</pre>
                              : <span className="mono-sm cc-danger-text cc-trunc">{error}</span>}
                          </button>
                        </td>
                        <td className="is-actions"><LinkButton href={href.message(f.id)} size="sm" label={`Open ${f.message.type} message`}>Open</LinkButton></td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            <div className="sx-table-foot">
              <span className="caption sx-num">{`${visible.length} of ${rows.length} shown`}</span>
              <span className="sx-spacer" />
              {visible.length < rows.length
                ? <Button size="sm" variant="ghost" icon="chevron-down" onClick={() => setShown((n) => n + SHOW_STEP)}>Load more</Button>
                : <span className="caption">End of list</span>}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
