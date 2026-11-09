// Actor detail: header (type, params, version), the mailbox with live changes and cancel, the state as collapsible
// JSON with change highlights, and the actor's recent failures.

import { useRef, useState } from 'react';
import { ApiError, fetchPages } from '../api/client';
import type { ActorView, MessageState, MessageStatusView, MessageView, OutcomeView, TypeView } from '../api/wire';
import { ProblemTypes } from '../api/wire';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { withoutScheme } from '../lib/address';
import { addedKeys, jsonChanges, removedKeys, type JsonChanges } from '../lib/diff';
import { bytes, clock, count, dateTime, relative, shortDateTime, utf8Length } from '../lib/format';
import { href } from '../lib/routes';
import { Badge, Button, Callout, CopyButton, Count, cx, EmptyState, Failures, Icon, LinkButton, OkMark, ProblemCallout, SrOnly, StatusBadge, Widget } from '../sx';
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
      <div className="cc-page">
        <PageHead crumbs={[...crumbs, { label: 'Not found' }]} title={uri} mono />
        <Callout kind="user" title="No such actor" code="404 Not Found"
          detail={<><Mono>{uri}</Mono> does not exist: it was deleted, stopped itself, or was never created. This page keeps checking.</>} />
      </div>
    );
  }
  if (!loaded) {
    return (
      <div className="cc-page">
        <PageHead crumbs={[...crumbs, { label: '…' }]} title={uri} mono />
        <Loading>Loading actor…</Loading>
      </div>
    );
  }

  const a = loaded.actor;
  const params = Object.entries(a.params);
  return (
    <div className="cc-page">
      <PageHead
        crumbs={[...crumbs, { label: withoutScheme(a.type), href: href.actors({ type: a.type }), mono: true, title: a.type }, { label: withoutScheme(a.uri), mono: true, title: a.uri }]}
        title={a.uri}
        mono
        titleAfter={<CopyButton text={a.uri} label="Copy actor URI" />}
        after={
          <div className="cc-meta">
            <a className="cc-type-link" href={href.types({ template: a.type })} title="Open this type">
              <Badge tone="outline" icon="template"><span className="mono-sm">{a.type}</span></Badge>
            </a>
            <span className="caption cc-muted">{params.length ? params.map(([k, v]) => `${k} = ${v}`).join(', ') : 'no parameters'}</span>
            <span className="sx-vr cc-vr-sm" aria-hidden="true" />
            <span className="caption cc-muted sx-num">
              Version <b className={cx('cc-ink', loaded.versionFrom !== null && 'cc-changed')}>{String(a.version)}</b>
            </span>
            {loaded.versionFrom !== null ? <span className="caption cc-version-delta"><Icon name="arrow-up" size={12} />{`from ${loaded.versionFrom} at last refresh`}</span> : null}
            <span className="sx-vr cc-vr-sm" aria-hidden="true" />
            <span className="caption cc-muted sx-num">Created {dateTime(a.createdAt)}</span>
          </div>
        }
        actions={
          <>
            <Button variant="danger-ghost" icon="trash" onClick={() => setDeleting(true)}>Delete actor…</Button>
            <Button variant="primary" icon="send" onClick={() => void openSend()}>Send message…</Button>
          </>
        }
      />
      {notice ? <ProblemCallout error={notice} title="Cannot load the type of this actor" /> : null}
      <div className="cc-actor-grid">
        <StateCard actor={a} stateless={ctx.typeOf(uri)?.type.state === null} changes={loaded.changes} mode={jsonMode} overrides={overrides}
          onMode={(mode) => { setJsonMode(mode); setOverrides(new Map()); }}
          onToggle={(path, open) => setOverrides((m) => new Map(m).set(path, open))} />
        <Mailbox loaded={loaded} cancelled={cancelled} cancelProblems={cancelProblems} onCancel={(id) => void cancel(id)} onSend={() => void openSend()}
          onDismiss={(id) => setCancelProblems((m) => new Map([...m].filter(([k]) => k !== id)))} />
      </div>
      <FailuresCard uri={uri} failures={loaded.failures} />
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
    <Widget title="Mailbox" count={<Count>{loaded.mailboxMore ? `${total}+` : String(total)}</Count>} subtitle={parts.join(' · ')} flush>
      {rows.length === 0 ? (
        <EmptyState title="Mailbox is empty" caption="No pending messages. Finished messages aren’t listed — find them by id or under Failures."
          action={<Button size="sm" icon="send" onClick={onSend}>Send message…</Button>} />
      ) : (
        <div className="sx-tablewrap">
          <table className="sx-table cc-table-mailbox">
            <colgroup><col /><col className="cc-w-240" /><col className="cc-w-80" /><col className="cc-w-160" /><col className="cc-w-100" /></colgroup>
            <thead>
              <tr><th>Message</th><th>Deliver at</th><th className="is-num">Attempt</th><th>Last error</th><th className="is-actions cc-actions-wide"><SrOnly>Action</SrOnly></th></tr>
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
    </Widget>
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
  const origin = (m.from ?? 'external') + (m.correlationId ? ' · ' + m.correlationId : '');
  const originTitle = 'From ' + (m.from ?? 'outside an actor') + (m.correlationId ? ' · correlation ' + m.correlationId : '');
  return (
    <>
      <tr className={cx('cc-tall', finished ? 'cc-leaving' : isNew && 'cc-new')}>
        <td>
          <div className="cc-stack">
            <a className="label cc-trunc" href={href.message(m.id)} title={m.type}>{m.type}</a>
            <span className="mono-sm cc-subtle cc-trunc" title={originTitle}>{origin}</span>
          </div>
        </td>
        <td>
          <div className="cc-stack">
            <span className="sx-num">{shortDateTime(m.deliverAt)}</span>
            <span className="cc-row cc-gap-4">
              <StatusBadge status={status} size="sm" />
              <span className={cx('caption cc-trunc', !finished && !due ? 'cc-ink' : 'cc-subtle')} title={when}>{when}</span>
            </span>
          </div>
        </td>
        <td className="is-num">
          <div className="cc-stack cc-end">
            <span className="sx-num">{String(m.attempt)}</span>
            {m.failures > 0 ? <Failures n={m.failures} suffix="failed" /> : null}
          </div>
        </td>
        <td>{m.lastError ? <span className="mono-sm cc-danger-text" title={m.lastError}>{m.lastError}</span> : <span className="cc-subtle">—</span>}</td>
        <td className="is-actions cc-actions-wide">
          {finished
            ? <span className="caption cc-muted">{status === 'cancelled' ? 'Cancelled' : 'Finished'}</span>
            : running
              ? <Button size="sm" disabled title="A running message can’t be cancelled">Cancel</Button>
              : <Button size="sm" label={`Cancel ${m.type} message`} onClick={() => onCancel(m.id)}>Cancel</Button>}
        </td>
      </tr>
      {problem ? (
        <tr className="cc-problem-row">
          <td colSpan={5} className="cc-wrap">
            <ProblemCallout
              error={problem}
              title={error?.type === ProblemTypes.MESSAGE_RUNNING ? 'Cancel failed — Message is running.' : error?.temporary ? undefined : 'Cancel failed — ' + (error?.title ?? 'error') + '.'}
              retry={() => onCancel(m.id)}
              action={error?.temporary ? undefined : <Button size="sm" variant="ghost" onClick={() => onDismiss(m.id)}>Dismiss</Button>}
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
      <Widget title="State" subtitle={`v${a.version}`}>
        {stateless
          ? <EmptyState badge={<Badge tone="outline">Stateless</Badge>} title="This actor has no state" caption={<>Its type <Mono>{a.type}</Mono> is stateless. Version stays 0.</>} />
          : <EmptyState title="State is null" caption="The actor was created without a state, or its handler set none." />}
      </Widget>
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
    <Widget
      title="State"
      subtitle={`v${a.version} · ${bytes(utf8Length(text))}${changed ? ` · ${count(changed, 'field')} changed since last refresh` : ''}`}
      actions={
        <span className="cc-row cc-gap-2">
          <Button variant="ghost" size="sm" onClick={() => onMode('expanded')}>Expand all</Button>
          <Button variant="ghost" size="sm" onClick={() => onMode('collapsed')}>Collapse all</Button>
          <CopyButton text={() => JSON.stringify(a.state, null, 2)} label="Copy state as JSON" />
        </span>
      }
      flush
    >
      <JsonView value={a.state} isOpen={isOpen} onToggle={onToggle} changes={changes?.changed ?? null} />
    </Widget>
  );
}

function FailuresCard({ uri, failures }: { uri: string; failures: OutcomeView[] }) {
  const recent = failures.slice(0, RECENT_FAILURES);
  return (
    <Widget
      title="Recent failures"
      count={failures.length ? <Count tone="danger">{String(failures.length)}</Count> : undefined}
      subtitle={`This actor · last 24 h${failures.length > RECENT_FAILURES ? ` · ${failures.length} in all` : ''}`}
      actions={<LinkButton variant="ghost" size="sm" href={href.failures({ prefix: uri })}>All failures for this actor</LinkButton>}
    >
      {recent.length === 0 ? (
        <EmptyState mark={<OkMark />} title="No failures in the last 24 h" caption="Widen the range on the Failures page to look further back." />
      ) : (
        <div className="cc-stack cc-gap-8">
          {recent.map((f) => (
            <div key={f.id} className="sx-alert sx-alert--danger">
              <span className="sx-tone-danger"><Icon name="error" size={20} /></span>
              <div className="cc-alert-text">
                <div className="sx-alert-title cc-row cc-gap-4"><span>{f.message.type}</span><StatusBadge status={f.status} size="sm" /></div>
                <p className="mono-sm cc-break">{f.error ?? '—'}</p>
                <p className="caption sx-num">{`${dateTime(f.finishedAt)} · attempt ${f.message.attempt}`}</p>
              </div>
              <LinkButton size="sm" href={href.message(f.id)} label={`Open ${f.message.type} message`}>Open message</LinkButton>
            </div>
          ))}
        </div>
      )}
    </Widget>
  );
}
