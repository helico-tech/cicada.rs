// Actor detail: header (type, params, version), the mailbox with live changes and cancel, the state as collapsible
// JSON with change highlights, and the actor's recent failures.

import { useRef, useState } from 'react';
import { ApiError, fetchPages } from '../api/client';
import type { ActorView, MessageState, MessageStatusView, MessageView, OutcomeView, TypeView } from '../api/generated';
import { ProblemTypes } from '../api/generated';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { withoutScheme } from '../lib/address';
import { addedKeys, jsonChanges, removedKeys, type JsonChanges } from '../lib/diff';
import { bytes, clock, count, dateTime, relative, shortDateTime, utf8Length } from '../lib/format';
import { href } from '../lib/routes';
import { Badge, Button, Callout, Card, CopyButton, cx, EmptyState, Icon, OkMark, ProblemCallout, StatusBadge } from '../halo';
import { DeleteActorDialog } from '../dialogs/DeleteActor';
import { SendMessageDialog } from '../dialogs/SendMessage';
import { JsonView } from '../ui/JsonView';
import { Loading, Mono, PageHead } from '../ui/layout';

const MAILBOX_LIMIT = 100;
const RECENT_FAILURES = 5;
const DAY_MS = 24 * 3600_000;

interface Leaving {
  message: MessageView;
  status: MessageState;
  finishedAt: string | null | undefined;
}

interface Loaded {
  actor: ActorView;
  mailbox: MessageView[];
  mailboxMore: boolean;
  head: MessageStatusView | null;
  leaving: Leaving[];
  failures: OutcomeView[];
  added: Set<string>;
  versionFrom: number | null;
  changes: JsonChanges | null;
}

export function ActorPage({ uri }: { uri: string }) {
  const ctx = useConsole();
  useTitle(uri);
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [cancelled, setCancelled] = useState<ReadonlyMap<string, number>>(new Map());
  const [cancelProblems, setCancelProblems] = useState<ReadonlyMap<string, unknown>>(new Map());
  const [notice, setNotice] = useState<unknown>(null);
  const [sending, setSending] = useState<TypeView | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [jsonMode, setJsonMode] = useState<'auto' | 'expanded' | 'collapsed'>('auto');
  const [overrides, setOverrides] = useState<ReadonlyMap<string, boolean>>(new Map());
  const knownIds = useRef<Set<string> | null>(null);
  const previous = useRef<Loaded | null>(null);

  usePageRefresh(async () => {
    let actor: ActorView;
    let page;
    try {
      actor = await ctx.api.getActor(uri);
      page = await ctx.api.pending(uri, { limit: MAILBOX_LIMIT });
    } catch (e) {
      if (!(e instanceof ApiError) || e.status !== 404) throw e;
      setNotFound(true); // absent, or deleted in between
      return;
    }
    // Only the head of a mailbox can be running: one turn at a time, always for the earliest due message.
    const [failures, head] = await Promise.all([
      recentFailures(),
      page.items[0] ? ctx.api.message(page.items[0].id).catch(() => null) : null,
    ]);
    const ids = page.items.map((m) => m.id);
    const gone = removedKeys(knownIds.current, ids);
    // Messages that left since the last refresh stay one more refresh, dimmed, with how they ended. One cancelled
    // here was already shown as cancelled, and leaves now.
    const before = previous.current;
    const leaving = (await Promise.all(gone.filter((id) => !cancelled.has(id)).slice(0, 10).map(async (id): Promise<Leaving | null> => {
      const was = before?.mailbox.find((m) => m.id === id);
      const status = await ctx.api.message(id).catch(() => null);
      const message = status?.message ?? was;
      return message ? { message, status: status?.status ?? 'done', finishedAt: status?.finishedAt } : null;
    }))).filter((l): l is Leaving => l !== null);
    if (gone.some((id) => cancelled.has(id))) {
      setCancelled((m) => new Map([...m].filter(([id]) => !gone.includes(id))));
    }
    setCancelProblems((m) => (m.size ? new Map([...m].filter(([id]) => ids.includes(id))) : m));

    const changed = before && actor.version !== before.actor.version;
    const next: Loaded = {
      actor,
      mailbox: page.items,
      mailboxMore: Boolean(page.next),
      head: head?.status === 'running' ? head : null,
      leaving,
      failures,
      added: addedKeys(knownIds.current, ids),
      versionFrom: changed ? before.actor.version : null,
      changes: changed ? jsonChanges(before.actor.state, actor.state) : null,
    };
    knownIds.current = new Set(ids);
    previous.current = next;
    setNotFound(false);
    setLoaded(next);
  });

  async function recentFailures(): Promise<OutcomeView[]> {
    const since = new Date(Date.now() - DAY_MS).toISOString();
    const { items } = await fetchPages((cursor) => ctx.api.failures({ prefix: uri, since, cursor, limit: 1000 }), { maxPages: 2 });
    // A prefix also covers longer URIs; keep this actor's own.
    return items.filter((f) => f.actor === uri).sort((a, b) => b.finishedAt.localeCompare(a.finishedAt));
  }

  async function openSend() {
    setNotice(null);
    let type = ctx.typeOf(uri)?.type;
    if (!type) {
      try {
        type = (await ctx.api.typeOf(uri)).type;
      } catch (e) {
        setNotice(e);
        return;
      }
    }
    setSending(type);
  }

  async function cancel(id: string) {
    setCancelProblems((m) => new Map([...m].filter(([k]) => k !== id)));
    try {
      await ctx.api.cancel(id);
      setCancelled((m) => new Map(m).set(id, Date.now()));
    } catch (e) {
      setCancelProblems((m) => new Map(m).set(id, e));
    }
  }

  const crumbs = [{ label: 'Actors', href: href.actors() }];
  if (notFound) {
    return (
      <div>
        <PageHead crumbs={[...crumbs, { label: 'Not found' }]} title={uri} titleClass="mono cc-title-uri" />
        <Callout kind="user" title="No such actor" code="404 Not Found"
          detail={<><Mono>{uri}</Mono> does not exist: it was deleted, stopped itself, or was never created. This page keeps checking.</>} />
      </div>
    );
  }
  if (!loaded) {
    return (
      <div>
        <PageHead crumbs={[...crumbs, { label: '…' }]} title={uri} titleClass="mono cc-title-uri" />
        <Loading>Loading actor…</Loading>
      </div>
    );
  }

  const a = loaded.actor;
  return (
    <div>
      <PageHead
        crumbs={[...crumbs, { label: withoutScheme(a.type) }]}
        title={a.uri}
        titleClass="mono cc-title-uri"
        titleAfter={<CopyButton text={a.uri} label="Copy actor URI" />}
        after={
          <div className="cc-meta">
            <div><span className="cc-muted">Type </span><a className="cc-link mono cc-small" href={href.types({ template: a.type })}>{a.type}</a></div>
            {Object.keys(a.params).length ? (
              <div className="cc-row cc-gap-6">
                <span className="cc-muted">Params</span>
                {Object.entries(a.params).map(([k, v]) => <span key={k} className="cc-chip cc-chip-static">{`${k} = ${v}`}</span>)}
              </div>
            ) : null}
            <div className="cc-row cc-gap-6">
              <span className="cc-muted">Version</span>
              <span className={cx('mono cc-version', loaded.versionFrom !== null && 'cc-changed')}>{String(a.version)}</span>
              {loaded.versionFrom !== null ? <span className="cc-version-delta"><Icon name="arrow-up" size={12} />{`from ${loaded.versionFrom} at last refresh`}</span> : null}
            </div>
            <div><span className="cc-muted">Created </span><span className="ha-num">{dateTime(a.createdAt)}</span></div>
          </div>
        }
        actions={
          <>
            <Button className="cc-danger" icon="trash" onClick={() => setDeleting(true)}>Delete Actor</Button>
            <Button variant="primary" icon="arrow-right" onClick={() => void openSend()}>Send Message</Button>
          </>
        }
      />
      {notice ? <ProblemCallout error={notice} title="Cannot load the type of this actor" /> : null}
      <Mailbox loaded={loaded} cancelled={cancelled} cancelProblems={cancelProblems} onCancel={(id) => void cancel(id)} onSend={() => void openSend()}
        onDismiss={(id) => setCancelProblems((m) => new Map([...m].filter(([k]) => k !== id)))} />
      <div className="cc-grid-2">
        <StateCard actor={a} stateless={ctx.typeOf(uri)?.type.state === null} changes={loaded.changes} mode={jsonMode} overrides={overrides}
          onMode={(mode) => { setJsonMode(mode); setOverrides(new Map()); }}
          onToggle={(path, open) => setOverrides((m) => new Map(m).set(path, open))} />
        <FailuresCard uri={uri} failures={loaded.failures} />
      </div>
      {sending ? <SendMessageDialog uri={uri} type={sending} onClose={() => setSending(null)} /> : null}
      {deleting ? (
        <DeleteActorDialog uri={uri} version={a.version} pending={loaded.mailbox.length} morePending={loaded.mailboxMore}
          onClose={() => setDeleting(false)} onDeleted={() => { location.hash = href.actors(); }} />
      ) : null}
    </div>
  );
}

function Mailbox({ loaded, cancelled, cancelProblems, onCancel, onSend, onDismiss }: { loaded: Loaded; cancelled: ReadonlyMap<string, number>; cancelProblems: ReadonlyMap<string, unknown>; onCancel: (id: string) => void; onSend: () => void; onDismiss: (id: string) => void }) {
  const total = loaded.mailbox.length;
  const parts = [loaded.mailboxMore ? `first ${total} pending` : `${total} pending`];
  if (loaded.leaving.length) parts.push(`${loaded.leaving.length} left since last refresh`);
  parts.push('ordered by deliver at');
  const rows = [
    ...loaded.mailbox.map((m, i) => ({ m, running: i === 0 ? loaded.head : null, leaving: null })),
    ...loaded.leaving.map((l) => ({ m: l.message, running: null, leaving: l })),
  ];
  return (
    <Card title="Mailbox" subtitle={parts.join(' · ')} flush>
      {rows.length === 0 ? (
        <EmptyState title="Mailbox is empty" caption="No pending messages. Finished messages aren’t listed — find them by id or under Failures."
          action={<Button size="sm" onClick={onSend}>Send Message</Button>} />
      ) : (
        <div className="cc-scroll-x">
          <table className="ha-table cc-table cc-table-mailbox">
            <thead>
              <tr>
                <th>Type</th><th>Status</th><th>Deliver at</th><th className="cc-num">Attempt</th><th className="cc-num">Failures</th>
                <th className="cc-col-error">Last error</th><th>From</th><th>Correlation</th><th className="cc-num">Action</th>
              </tr>
            </thead>
            <tbody>
              {rows.map(({ m, running, leaving }) => (
                <MailboxRow key={(leaving ? 'left:' : '') + m.id} m={m} running={running} leaving={leaving} isNew={loaded.added.has(m.id)}
                  cancelledAt={cancelled.get(m.id)} problem={cancelProblems.get(m.id)} onCancel={onCancel} onDismiss={onDismiss} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  );
}

function MailboxRow({ m, running, leaving, isNew, cancelledAt, problem, onCancel, onDismiss }: { m: MessageView; running: MessageStatusView | null; leaving: Leaving | null; isNew: boolean; cancelledAt: number | undefined; problem: unknown; onCancel: (id: string) => void; onDismiss: (id: string) => void }) {
  const status: MessageState = leaving ? leaving.status : cancelledAt ? 'cancelled' : running ? 'running' : 'pending';
  const due = Date.parse(m.deliverAt) <= Date.now();
  let when: string;
  if (leaving) when = 'left the mailbox' + (leaving.finishedAt ? ' ' + relative(leaving.finishedAt) : '');
  else if (cancelledAt) when = 'cancelled just now · leaves on next refresh';
  else if (running) when = `due ${relative(m.deliverAt)} · lease to ${clock(running.leaseUntil)}`;
  else when = due ? `due ${relative(m.deliverAt)}` : relative(m.deliverAt);
  const finished = Boolean(leaving || cancelledAt);
  const error = problem instanceof ApiError ? problem : null;
  return (
    <>
      <tr className={finished ? 'cc-leaving' : isNew ? 'cc-new' : undefined}>
        <td><a className="cc-link mono cc-strong" href={href.message(m.id)}>{m.type}</a></td>
        <td><StatusBadge status={status} /></td>
        <td>
          <div className="ha-num cc-nowrap">{shortDateTime(m.deliverAt)}</div>
          <div className={cx('cc-caption cc-nowrap', !finished && !due && 'cc-ink')}>{when}</div>
        </td>
        <td className="ha-num cc-num">{String(m.attempt)}</td>
        <td className={cx('ha-num cc-num', m.failures > 0 && 'cc-critical cc-strong')}>{String(m.failures)}</td>
        <td className="cc-col-error">{m.lastError ? <span className="cc-trunc mono" title={m.lastError}>{m.lastError}</span> : <span className="cc-faint">—</span>}</td>
        <td><span className="mono cc-trunc cc-muted cc-from" title={m.from ?? 'outside an actor'}>{m.from ?? 'external'}</span></td>
        <td><span className="mono cc-muted">{m.correlationId ?? '—'}</span></td>
        <td className="cc-num cc-nowrap">
          {finished
            ? <span className="cc-caption">{status === 'cancelled' ? 'Cancelled' : 'Finished'}</span>
            : running
              ? <span className="cc-caption">Running — can’t cancel</span>
              : <Button variant="ghost" size="sm" label={`Cancel ${m.type} message`} onClick={() => onCancel(m.id)}>Cancel</Button>}
        </td>
      </tr>
      {problem ? (
        <tr className="cc-problem-row">
          <td colSpan={9}>
            <ProblemCallout
              error={problem}
              title={error?.type === ProblemTypes.MESSAGE_RUNNING ? 'Cancel failed — Message is running.' : error?.temporary ? undefined : 'Cancel failed — ' + (error?.title ?? 'error') + '.'}
              retry={() => onCancel(m.id)}
              action={error?.temporary ? undefined : <button type="button" className="cc-btn-reset cc-caption cc-underline" onClick={() => onDismiss(m.id)}>Dismiss</button>}
            />
          </td>
        </tr>
      ) : null}
    </>
  );
}

function StateCard({ actor: a, stateless, changes, mode, overrides, onMode, onToggle }: { actor: ActorView; stateless: boolean; changes: JsonChanges | null; mode: 'auto' | 'expanded' | 'collapsed'; overrides: ReadonlyMap<string, boolean>; onMode: (mode: 'expanded' | 'collapsed') => void; onToggle: (path: string, open: boolean) => void }) {
  if (a.state === null) {
    return (
      <Card title="State" subtitle={`v${a.version}`}>
        {stateless
          ? <EmptyState badge={<Badge>Stateless</Badge>} title="This actor has no state" caption={<>Its type <Mono>{a.type}</Mono> is stateless. Version stays 0.</>} />
          : <EmptyState title="State is null" caption="The actor was created without a state, or its handler set none." />}
      </Card>
    );
  }
  const text = JSON.stringify(a.state);
  const changed = changes ? changes.changed.size + changes.removed : 0;
  const isOpen = (path: string, depth: number, size: number): boolean => {
    const override = overrides.get(path);
    if (override !== undefined) return override;
    if (mode === 'expanded') return true;
    if (mode === 'collapsed') return depth === 0;
    return depth < 2 || size <= 20;
  };
  return (
    <Card title="State" subtitle={`v${a.version} · ${bytes(utf8Length(text))}${changed ? ` · ${count(changed, 'field')} changed since last refresh` : ''}`}>
      <div className="cc-toolbar">
        <Button size="sm" onClick={() => onMode('expanded')}>Expand All</Button>
        <Button size="sm" onClick={() => onMode('collapsed')}>Collapse All</Button>
        <span className="cc-grow" />
        <CopyJsonButton text={() => JSON.stringify(a.state, null, 2)} />
      </div>
      <JsonView value={a.state} isOpen={isOpen} onToggle={onToggle} changes={changes?.changed ?? null} />
    </Card>
  );
}

function CopyJsonButton({ text }: { text: () => string }) {
  const [label, setLabel] = useState('Copy JSON');
  return (
    <Button variant="ghost" size="sm" onClick={() => {
      navigator.clipboard.writeText(text()).then(() => setLabel('Copied'), () => setLabel('Copy failed'));
      setTimeout(() => setLabel('Copy JSON'), 1500);
    }}>{label}</Button>
  );
}

function FailuresCard({ uri, failures }: { uri: string; failures: OutcomeView[] }) {
  const recent = failures.slice(0, RECENT_FAILURES);
  return (
    <Card title="Recent Failures" subtitle={`This actor · last 24 h${failures.length > RECENT_FAILURES ? ` · ${failures.length} in all` : ''}`} flush>
      {recent.length === 0 ? (
        <EmptyState mark={<OkMark />} title="No failures in the last 24 h" caption="Widen the range on the Failures page to look further back." />
      ) : (
        <div>
          {recent.map((f) => (
            <a key={f.id} className="cc-failure-item" href={href.message(f.id)}>
              <div className="cc-row">
                <span className="mono cc-strong cc-small">{f.message.type}</span>
                <StatusBadge status={f.status} />
                <span className="cc-grow" />
                <span className="cc-caption ha-num">{shortDateTime(f.finishedAt)}</span>
              </div>
              <span className="mono cc-trunc cc-muted cc-xsmall">{f.error ?? '—'}</span>
            </a>
          ))}
        </div>
      )}
      <div className="cc-card-foot cc-card-foot-plain">
        <a className="cc-link cc-small cc-strong" href={href.failures({ prefix: uri })}>View all failures for this actor</a>
      </div>
    </Card>
  );
}
