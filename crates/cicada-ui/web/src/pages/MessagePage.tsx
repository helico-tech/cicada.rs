// Message detail: GET /messages/{id} — status, execution (lease or outcome), retries, persistence, envelope and
// payload — and GET /messages/{id}/chain: what led to the message and what it caused.

import { lazy, Suspense, useState } from 'react';
import { ApiError } from '../api/client';
import type { MessageChainView, MessageStatusView, MessageView, RetryPolicy } from '../api/generated';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { bytes, dateTime, isoDurationText, relative, utf8Length } from '../lib/format';
import { keepText } from '../lib/retention';
import { href } from '../lib/routes';
import { Button, Callout, CopyButton, Failures, ProblemCallout, STATUS, StatusBadge, Widget } from '../sx';
import { JsonView } from '../ui/JsonView';
import { Definitions, Loading, PageHead, type Definition } from '../ui/layout';

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
      <div className="cc-page">
        <PageHead crumbs={[...crumbs, { label: 'Message' }]} title={id} mono />
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
      <div className="cc-page">
        <PageHead crumbs={[...crumbs, { label: 'Message' }]} title={id} mono />
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
    <div className="cc-page">
      <PageHead
        crumbs={[...crumbs, { label: m.to, href: href.actor(m.to), mono: true, title: m.to }, { label: 'Message' }]}
        title={m.type}
        titleAfter={<StatusBadge status={view.status} />}
        after={<div className="cc-row cc-gap-4"><span className="mono-sm cc-muted">{view.id}</span><CopyButton text={view.id} label="Copy message id" /></div>}
        actions={
          <div className="cc-cancel-box">
            {pending ? (
              <Button icon="close" onClick={() => void cancel()}>Cancel message</Button>
            ) : (
              <>
                <Button icon="close" disabled describedBy="cancel-why">Cancel message</Button>
                <span id="cancel-why" className="caption cc-muted cc-right">{why}</span>
              </>
            )}
          </div>
        }
      />
      {cancelError ? <ProblemCallout error={cancelError} title={'Cancel failed — ' + (cancelError instanceof ApiError ? cancelError.title : 'error')} retry={() => void cancel()} /> : null}
      <div className="cc-message-grid">
        <div className="cc-stack cc-gap-16">
          <DeliveryCard view={view} />
          <EnvelopeCard m={m} />
          <Widget title="Payload" subtitle={`application/json · ${bytes(utf8Length(JSON.stringify(m.payload)))}`} flush>
            <JsonView value={m.payload} />
          </Widget>
        </div>
        {chain ? <Chain chain={chain} /> : null}
      </div>
    </div>
  );
}

/** Execution (lease or outcome), retries and persistence, as one property list. */
function DeliveryCard({ view: v }: { view: MessageStatusView }) {
  const m = v.message;
  const retry = m.retry;
  let next = '—';
  if (v.status === 'pending' && m.failures > 0) next = relative(m.deliverAt);
  else if (v.status === 'running' && retry && m.failures + 1 < retry.maxAttempts) next = 'if this run fails';
  else if (v.status === 'failed' && retry) next = 'none left';

  let state: string;
  let execution: Definition[];
  if (v.status === 'pending') {
    state = 'Waiting in mailbox · delivered when due; can still be cancelled';
    execution = [
      ['Deliver at', `${dateTime(m.deliverAt)} (${relative(m.deliverAt)})`, 'sx-num'],
      ['Attempts so far', String(m.attempt), 'sx-num'],
    ];
  } else if (v.status === 'running') {
    state = 'Running · held by a node under a lease';
    execution = [
      ['Node', v.node ?? '—', 'mono-sm'],
      ['Lease expires', v.leaseUntil ? `${dateTime(v.leaseUntil)} (${relative(v.leaseUntil)})` : '—', 'sx-num'],
      ['Due since', `${dateTime(m.deliverAt)} (${relative(m.deliverAt)})`, 'sx-num'],
    ];
  } else {
    state = 'Finished · ' + (FINISHED_SUBTITLE[v.status] ?? '').toLowerCase();
    execution = [
      ['Finished at', v.finishedAt ? `${dateTime(v.finishedAt)} (${relative(v.finishedAt)})` : '—', 'sx-num'],
      ['Error', v.error ? <pre className="mono-sm cc-error-block">{v.error}</pre> : '—'],
    ];
  }
  const lastError = v.lastError ?? m.lastError;
  return (
    <Widget title="Delivery" subtitle={state} flush>
      <Definitions rows={[
        ...execution,
        ['Attempt', <span className="sx-num">{String(m.attempt)}{retry ? ` of ${retry.maxAttempts}` : ''}</span>],
        ['Failures', <Failures n={m.failures} />],
        ['Next retry', next],
        ['Last error', lastError ?? '—', lastError ? 'mono-sm cc-danger-text' : null],
        ['Retry policy', <PolicyText retry={retry} />],
        ['When done', keepText(m.retention?.done)],
        ['When failed', keepText(m.retention?.failed)],
        ['Cancelled or purged', 'Store default'],
      ]} />
    </Widget>
  );
}

function PolicyText({ retry }: { retry: RetryPolicy | null }) {
  if (!retry) return <span className="cc-stack"><span>None</span><span className="caption cc-muted">A failed run ends the message as failed.</span></span>;
  const b = retry.backoff;
  const detail = b.kind === 'fixed'
    ? `delay ${isoDurationText(b.delay)} · max attempts ${retry.maxAttempts}`
    : `initial ${isoDurationText(b.initial)} · factor ${Number(b.factor ?? 2).toFixed(1)} · max delay ${isoDurationText(b.max)} · max attempts ${retry.maxAttempts}`;
  return <span className="cc-stack"><span>{b.kind === 'fixed' ? 'Fixed delay' : 'Exponential backoff'}</span><span className="mono-sm cc-muted">{detail}</span></span>;
}

function EnvelopeCard({ m }: { m: MessageView }) {
  const headers = Object.entries(m.headers);
  return (
    <Widget title="Envelope" flush>
      <Definitions rows={[
        ['Id', m.id, 'mono-sm'],
        ['To', <a className="mono-sm" href={href.actor(m.to)}>{m.to}</a>],
        ['From', m.from ? <a className="mono-sm" href={href.actor(m.from)}>{m.from}</a> : 'outside an actor', m.from ? null : 'cc-muted'],
        ['Caused by', m.causedBy ? <a className="mono-sm" href={href.message(m.causedBy)}>{m.causedBy}</a> : 'nothing — sent from outside an actor', m.causedBy ? null : 'cc-muted'],
        ['Type', m.type, 'mono-sm'],
        ['Deliver at', <span><span className="sx-num">{dateTime(m.deliverAt)}</span> <span className="caption cc-muted">{relative(m.deliverAt)}</span></span>],
        ['Correlation id', m.correlationId ?? '—', 'mono-sm'],
        ['Sequence', String(m.seq), 'mono-sm'],
        ['Attempt', String(m.attempt), 'mono-sm'],
        ['Headers', headers.length
          ? <div className="cc-headers">{headers.map(([k, val]) => <div key={k} className="mono-sm"><span className="cc-muted">{k}:</span> {val}</div>)}</div>
          : '—'],
      ]} />
    </Widget>
  );
}
