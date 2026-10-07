// Message detail: GET /messages/{id} — status, execution (lease or outcome), retries, persistence, envelope and
// payload — and GET /messages/{id}/chain: what led to the message and what it caused.

import { lazy, Suspense, useState, type ReactNode } from 'react';
import { ApiError } from '../api/client';
import type { MessageChainView, MessageStatusView, MessageView, Retention, RetryPolicy } from '../api/generated';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { bytes, dateTime, isoDurationText, relative, utf8Length } from '../lib/format';
import { keepText } from '../lib/retention';
import { href } from '../lib/routes';
import { Button, Callout, Card, CopyButton, cx, ProblemCallout, STATUS, StatusBadge } from '../halo';
import { JsonView } from '../ui/JsonView';
import { Definitions, Loading, PageHead } from '../ui/layout';

// The graph library is only needed here, so it loads with the first message page.
const ChainView = lazy(() => import('../chain/ChainView').then((m) => ({ default: m.ChainView })));

function Chain({ chain }: { chain: MessageChainView }) {
  return <Suspense fallback={<Loading>Loading message chain…</Loading>}><ChainView chain={chain} /></Suspense>;
}

const FINISHED_SUBTITLE: Record<string, string> = {
  done: 'Handled successfully',
  failed: 'The handler failed, the retry policy ran out, or the failure was permanent',
  cancelled: 'Cancelled before delivery',
  purged: 'Removed without being handled: its actor was stopped or deleted',
};

export function MessagePage({ id }: { id: string }) {
  const ctx = useConsole();
  useTitle('Message ' + id);
  const [view, setView] = useState<MessageStatusView | null>(null);
  const [chain, setChain] = useState<MessageChainView | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [cancelError, setCancelError] = useState<unknown>(null);

  usePageRefresh(async () => {
    const [status, links] = await Promise.all([
      ctx.api.message(id).catch((e: unknown) => {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }),
      ctx.api.chain(id).catch((e: unknown) => {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }),
    ]);
    setView(status);
    setChain(links);
    setNotFound(status === null);
  });

  async function cancel() {
    setCancelError(null);
    try {
      await ctx.api.cancel(id);
    } catch (e) {
      setCancelError(e);
    }
    ctx.refresh();
  }

  const crumbs = [{ label: 'Actors', href: href.actors() }];
  if (notFound) {
    // A message removed by its retention policy is still part of its chain.
    const removed = chain?.nodes.find((n) => n.id === id && n.removed);
    return (
      <div>
        <PageHead crumbs={[...crumbs, { label: 'Message' }]} title={id} titleClass="mono cc-title-uri" />
        {removed ? (
          <Callout kind="info" title="Record removed" code="404 Not Found"
            detail={`This ${removed.type} message ended ${STATUS[removed.status].label.toLowerCase()} and its sender asked for its record to be removed. Only its place in the chain below is kept.`} />
        ) : (
          <Callout kind="user" title="No such message" code="404 Not Found"
            detail="The id is unknown, or the message finished longer ago than the node keeps outcomes (7 days by default)." />
        )}
        {chain ? <Chain chain={chain} /> : null}
      </div>
    );
  }
  if (!view) {
    return (
      <div>
        <PageHead crumbs={[...crumbs, { label: 'Message' }]} title={id} titleClass="mono cc-title-uri" />
        <Loading>Loading message…</Loading>
      </div>
    );
  }
  const m = view.message;
  const pending = view.status === 'pending';
  const why = view.status === 'running'
    ? 'Running messages can’t be cancelled. If this attempt fails and its retry policy allows another, it returns to the mailbox and can be cancelled then.'
    : `This message already finished (${STATUS[view.status].label}). Only pending messages can be cancelled.`;
  return (
    <div>
      <PageHead
        crumbs={[...crumbs, { label: m.to, href: href.actor(m.to), mono: true }, { label: 'Message' }]}
        title={<>{m.type} <StatusBadge status={view.status} /></>}
        titleClass="mono cc-title-message"
        after={<div className="cc-row cc-gap-6 cc-id-line"><span className="mono cc-muted cc-small">{view.id}</span><CopyButton text={view.id} label="Copy message id" /></div>}
        actions={
          <div className="cc-cancel-box">
            {pending ? (
              <Button icon="x-circle" onClick={() => void cancel()}>Cancel Message</Button>
            ) : (
              <>
                <Button icon="x-circle" disabled describedBy="cancel-why">Cancel Message</Button>
                <span id="cancel-why" className="cc-caption cc-right">{why}</span>
              </>
            )}
          </div>
        }
      />
      {cancelError ? <ProblemCallout error={cancelError} title={'Cancel failed — ' + (cancelError instanceof ApiError ? cancelError.title : 'error')} retry={() => void cancel()} /> : null}
      {chain ? <Chain chain={chain} /> : null}
      <div className="cc-grid-2 cc-grid-top">
        <div className="cc-stack-24">
          <ExecutionCard view={view} />
          <RetriesCard view={view} />
          <PersistenceCard retention={m.retention} />
        </div>
        <div className="cc-stack-24">
          <EnvelopeCard m={m} />
          <Card title="Payload" subtitle={`application/json · ${bytes(utf8Length(JSON.stringify(m.payload)))}`}><JsonView value={m.payload} /></Card>
        </div>
      </div>
    </div>
  );
}

function ExecutionCard({ view: v }: { view: MessageStatusView }) {
  const m = v.message;
  if (v.status === 'pending') {
    return (
      <Card title="Waiting In Mailbox" subtitle="Delivered when due; can still be cancelled">
        <Definitions rows={[
          ['Deliver at', `${dateTime(m.deliverAt)} (${relative(m.deliverAt)})`, 'ha-num'],
          ['Attempts so far', String(m.attempt), 'ha-num'],
        ]} />
      </Card>
    );
  }
  if (v.status === 'running') {
    return (
      <Card title="Running" subtitle="Held by a node under a lease">
        <Definitions rows={[
          ['Node', v.node ?? '—', 'mono'],
          ['Lease expires', v.leaseUntil ? `${dateTime(v.leaseUntil)} (${relative(v.leaseUntil)})` : '—', 'ha-num'],
          ['Due since', `${dateTime(m.deliverAt)} (${relative(m.deliverAt)})`, 'ha-num'],
        ]} />
      </Card>
    );
  }
  return (
    <Card title="Finished" subtitle={FINISHED_SUBTITLE[v.status] ?? ''}>
      <Definitions rows={[
        ['Finished at', v.finishedAt ? `${dateTime(v.finishedAt)} (${relative(v.finishedAt)})` : '—', 'ha-num'],
        v.error ? null : ['Error', '—'],
      ]} />
      {v.error ? <pre className="mono cc-error-block">{v.error}</pre> : null}
    </Card>
  );
}

function Tile({ label, value, className }: { label: string; value: ReactNode; className?: string }) {
  return (
    <div className="cc-tile">
      <div className="cc-caption">{label}</div>
      <div className={cx('ha-num cc-tile-value', className)}>{value}</div>
    </div>
  );
}

function RetriesCard({ view: v }: { view: MessageStatusView }) {
  const m = v.message;
  const retry = m.retry;
  let next = '—';
  if (v.status === 'pending' && m.failures > 0) next = relative(m.deliverAt);
  else if (v.status === 'running' && retry && m.failures + 1 < retry.maxAttempts) next = 'if this run fails';
  else if (v.status === 'failed' && retry) next = 'none left';
  return (
    <Card title="Retries" subtitle="Policy and failure history">
      <div className="cc-tiles">
        <Tile label="Failures" value={String(m.failures)} className={m.failures > 0 ? 'cc-critical' : undefined} />
        <Tile label="Attempt" value={<>{String(m.attempt)}{retry ? <span className="cc-tile-of">{` of ${retry.maxAttempts}`}</span> : null}</>} />
        <Tile label="Next retry" value={next} className="cc-tile-small" />
      </div>
      <Definitions rows={[
        ['Last error', v.lastError ?? m.lastError ?? '—', 'mono cc-small'],
        ['Retry policy', <PolicyText retry={retry} />],
      ]} />
    </Card>
  );
}

function PolicyText({ retry }: { retry: RetryPolicy | null }) {
  if (!retry) return <><div>None</div><div className="cc-caption">A failed run ends the message as failed.</div></>;
  const b = retry.backoff;
  const detail = b.kind === 'fixed'
    ? `delay ${isoDurationText(b.delay)} · max attempts ${retry.maxAttempts}`
    : `initial ${isoDurationText(b.initial)} · factor ${Number(b.factor ?? 2).toFixed(1)} · max delay ${isoDurationText(b.max)} · max attempts ${retry.maxAttempts}`;
  return <><div>{b.kind === 'fixed' ? 'Fixed delay' : 'Exponential backoff'}</div><div className="cc-caption mono">{detail}</div></>;
}

function PersistenceCard({ retention }: { retention: Retention | null }) {
  return (
    <Card title="Persistence" subtitle="How long the record stays once the message finished">
      <Definitions rows={[
        ['When done', keepText(retention?.done)],
        ['When failed', keepText(retention?.failed)],
        ['Cancelled or purged', 'Store default'],
      ]} />
    </Card>
  );
}

function EnvelopeCard({ m }: { m: MessageView }) {
  const headers = Object.entries(m.headers);
  return (
    <Card title="Envelope">
      <Definitions rows={[
        ['ID', m.id, 'mono'],
        ['To', <a className="cc-link mono" href={href.actor(m.to)}>{m.to}</a>],
        ['From', m.from ? <a className="cc-link mono" href={href.actor(m.from)}>{m.from}</a> : 'outside an actor', m.from ? null : 'cc-muted'],
        ['Caused by', m.causedBy ? <a className="cc-link mono" href={href.message(m.causedBy)}>{m.causedBy}</a> : 'nothing — sent from outside an actor', m.causedBy ? null : 'cc-muted'],
        ['Type', m.type, 'mono'],
        ['Deliver at', <><span className="ha-num">{dateTime(m.deliverAt)}</span> <span className="cc-caption">{relative(m.deliverAt)}</span></>],
        ['Correlation ID', m.correlationId ?? '—', 'mono'],
        ['Sequence', String(m.seq), 'mono'],
        ['Attempt', String(m.attempt), 'mono'],
        ['Headers', headers.length
          ? <div className="cc-headers">{headers.map(([k, val]) => <div key={k} className="mono cc-xsmall"><span className="cc-muted">{k}</span> {val}</div>)}</div>
          : '—'],
      ]} />
    </Card>
  );
}
