// Send Message dialog: POST /actors/{path}/messages with a payload from the message type's schema, a delivery time,
// and optionally an id, correlation id, headers, a retry policy and a persistence (retention) policy.

import { useRef, useState, type ReactNode } from 'react';
import { ApiError } from '../api/client';
import type { RetryPolicy, SendRequest, TypeView } from '../api/generated';
import { useConsole } from '../app/context';
import { clock, count, dateTime, localInputValue, relative, timeZone } from '../lib/format';
import { blankKeep, readRetention, type KeepDraft, type KeepKind } from '../lib/retention';
import { href } from '../lib/routes';
import { Button, Callout, Field, IconButton, LinkButton, ProblemCallout, Segmented, Select } from '../sx';
import { Dialog, DialogBody, DialogFoot, DialogHeader, DialogResult } from '../ui/Dialog';
import { Mono } from '../ui/layout';
import { ValueEditor, type ValueEditorHandle } from '../ui/ValueEditor';

const UNITS = { seconds: 1, minutes: 60, hours: 3600, days: 86400 } as const;
type Unit = keyof typeof UNITS;
type Delivery = 'now' | 'delay' | 'at';
type RetryKind = 'none' | 'fixed' | 'exponential';

interface Header {
  key: number;
  name: string;
  value: string;
}

const KEEP_OPTIONS: Array<{ value: KeepKind; label: string }> = [
  { value: 'default', label: 'Store default' },
  { value: 'remove', label: 'Remove at once' },
  { value: 'for', label: 'Keep for a time' },
  { value: 'last', label: 'Keep the last N' },
];

function KeepFields({ id, outcome, draft, onChange }: { id: string; outcome: string; draft: KeepDraft; onChange: (draft: KeepDraft) => void }) {
  return (
    <div className="cc-keep">
      <Field id={id} label={`When ${outcome}`}>
        <Select id={id} value={draft.kind} options={KEEP_OPTIONS} onChange={(v) => onChange({ ...draft, kind: v as KeepKind })} />
      </Field>
      {draft.kind === 'for' ? (
        <Field id={id + '-age'} label="Keep for" hint="e.g. 30d, 12h, PT30M">
          <input id={id + '-age'} className="sx-input sx-mono" value={draft.age} onChange={(e) => onChange({ ...draft, age: e.target.value })} />
        </Field>
      ) : draft.kind === 'last' ? (
        <Field id={id + '-count'} label="Keep the last" hint="Of this actor, with this outcome">
          <input id={id + '-count'} className="sx-input sx-num" inputMode="numeric" value={draft.count} onChange={(e) => onChange({ ...draft, count: e.target.value })} />
        </Field>
      ) : null}
    </div>
  );
}

/** The dialog for the actor [uri] of [type]. */
export function SendMessageDialog({ uri, type, onClose }: { uri: string; type: TypeView; onClose: () => void }) {
  const ctx = useConsole();
  const names = Object.keys(type.messages).sort();
  const [messageType, setMessageType] = useState(names[0] ?? '');
  const editor = useRef<ValueEditorHandle>(null);
  const [delivery, setDelivery] = useState<Delivery>('now');
  const [delayAmount, setDelayAmount] = useState('5');
  const [delayUnit, setDelayUnit] = useState<Unit>('minutes');
  const [at, setAt] = useState(() => localInputValue(Date.now() + 3600_000));
  const [id, setId] = useState('');
  const [correlation, setCorrelation] = useState('');
  const [headers, setHeaders] = useState<Header[]>([]);
  const nextHeader = useRef(0);
  const [retryKind, setRetryKind] = useState<RetryKind>('none');
  const [retry, setRetry] = useState({ maxAttempts: '3', delay: '30s', initial: '1s', max: '1m', factor: '2' });
  const [keepDone, setKeepDone] = useState<KeepDraft>(blankKeep);
  const [keepFailed, setKeepFailed] = useState<KeepDraft>(blankKeep);
  const [result, setResult] = useState<ReactNode>(null);
  const [sending, setSending] = useState(false);
  // A generated id is kept until the node gives a definite answer, so Retry now cannot send twice.
  const pendingId = useRef<string | null>(null);

  function deliveryTime(): { at: Date; body: Partial<SendRequest> } | { error: string } {
    if (delivery === 'delay') {
      const amount = Number(delayAmount);
      if (!Number.isFinite(amount) || amount < 0) return { error: 'The delay must be a number of zero or more.' };
      const seconds = Math.round(amount * UNITS[delayUnit]);
      return { at: new Date(Date.now() + seconds * 1000), body: { delay: `PT${seconds}S` } };
    }
    if (delivery === 'at') {
      const when = new Date(at);
      if (!at || Number.isNaN(when.getTime())) return { error: 'Choose a delivery time.' };
      return { at: when, body: { deliverAt: when.toISOString() } };
    }
    return { at: new Date(), body: {} };
  }

  function retryPolicy(): { policy: RetryPolicy | undefined } | { error: string } {
    if (retryKind === 'none') return { policy: undefined };
    const maxAttempts = Number(retry.maxAttempts);
    if (!Number.isInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > 100) return { error: 'Max attempts must be a whole number from 1 to 100.' };
    if (retryKind === 'fixed') return { policy: { maxAttempts, backoff: { kind: 'fixed', delay: retry.delay.trim() } } };
    const factor = Number(retry.factor);
    if (!Number.isFinite(factor) || factor < 1) return { error: 'The backoff factor must be a number of at least 1.' };
    return { policy: { maxAttempts, backoff: { kind: 'exponential', initial: retry.initial.trim(), factor, max: retry.max.trim() } } };
  }

  async function send() {
    if (sending) return;
    setResult(null);
    const payload = editor.current?.read();
    if (!payload) return;
    if (!payload.ok) {
      setResult(<Callout kind="user" title={`Payload does not match the ${messageType} schema`} detail={`${payload.message}: ${payload.invalid.join('; ')}. Fix it above and send again.`} />);
      return;
    }
    const time = deliveryTime();
    const policy = retryPolicy();
    const keep = readRetention(keepDone, keepFailed);
    const problem = 'error' in time ? time.error : 'error' in policy ? policy.error : 'error' in keep ? keep.error : null;
    if (problem || 'error' in time || 'error' in policy || 'error' in keep) {
      setResult(<Callout kind="user" title="Check the delivery settings" detail={problem} />);
      return;
    }
    const messageId = id.trim() || pendingId.current || (pendingId.current = globalThis.crypto?.randomUUID?.() ?? null);
    const named = headers.filter((h) => h.name.trim());
    const message: SendRequest = {
      ...(messageId ? { id: messageId } : {}),
      type: messageType,
      payload: payload.value ?? {},
      ...time.body,
      ...(correlation.trim() ? { correlationId: correlation.trim() } : {}),
      ...(named.length ? { headers: Object.fromEntries(named.map((h) => [h.name.trim(), h.value])) } : {}),
      ...(policy.policy ? { retry: policy.policy } : {}),
      ...(keep.retention ? { retention: keep.retention } : {}),
    };
    setSending(true);
    try {
      const { status, body } = await ctx.api.send(uri, message);
      pendingId.current = null;
      const open = (label: string) => <LinkButton size="sm" href={href.message(body.id)}>{label}</LinkButton>;
      setResult(status === 202 ? (
        <Callout kind="ok" title="Accepted" action={open('Open message')}
          detail={<><Mono>{body.id}</Mono> · delivers <span className="sx-num">{body.deliverAt ? `${dateTime(body.deliverAt)} (${relative(body.deliverAt)})` : '—'}</span></>} />
      ) : (
        <Callout kind="info" title="Duplicate — nothing was sent" action={open('Open existing')}
          detail={<>A message with id <Mono>{body.id}</Mono> was already accepted for this actor.</>} />
      ));
      ctx.refresh();
    } catch (e) {
      if (!(e instanceof ApiError) || !e.temporary) pendingId.current = null;
      setResult(<ProblemCallout error={e} retry={() => void send()} />);
    } finally {
      setSending(false);
    }
  }

  const time = deliveryTime();
  const deliverText = 'error' in time ? '—' : delivery === 'now' ? `now (${clock(time.at)})` : `${dateTime(time.at)} (${relative(time.at)})`;
  const field = (key: keyof typeof retry) => (e: React.ChangeEvent<HTMLInputElement>) => setRetry((r) => ({ ...r, [key]: e.target.value }));
  const shortHash = type.hash.slice(0, 13) + '…' + type.hash.slice(-4);

  return (
    <Dialog size="l" labelledBy="send-title" onClose={onClose}>
      <DialogHeader id="send-title" title="Send message" caption={<>To <span className="mono-sm">{uri}</span></>} onClose={onClose} />
      <DialogBody>
        <Field id="send-type" label="Message type" required
          hint={<>{count(names.length, 'type')} accepted by <Mono>{type.template}</Mono> · contract <span className="mono" title={type.hash}>{shortHash}</span></>}>
          <Select id="send-type" options={names} value={messageType} onChange={(v) => { setMessageType(v); setResult(null); }} />
        </Field>
        <ValueEditor key={messageType} title="Payload" schema={type.messages[messageType]} blank={{}} label={messageType} handle={editor} />
        <div className="sx-field">
          <span className="sx-label" id="send-delivery"><span>Delivery</span></span>
          <div className="cc-row cc-row-wrap">
            <Segmented label="Delivery" value={delivery} onChange={setDelivery}
              items={[{ value: 'now', label: 'Now' }, { value: 'delay', label: 'After a delay' }, { value: 'at', label: 'At a time' }]} />
            {delivery === 'delay' ? (
              <>
                <input className="sx-input sx-num cc-delay" inputMode="decimal" aria-label="Delay amount" value={delayAmount} onChange={(e) => setDelayAmount(e.target.value)} />
                <span className="cc-unit"><Select options={Object.keys(UNITS)} value={delayUnit} label="Delay unit" onChange={(v) => setDelayUnit(v as Unit)} /></span>
              </>
            ) : delivery === 'at' ? (
              <input className="sx-input sx-num cc-at" type="datetime-local" aria-label="Deliver at" value={at} onChange={(e) => setAt(e.target.value)} />
            ) : null}
          </div>
          <span className="sx-help">Delivers <b className="sx-num cc-ink">{deliverText}</b>{timeZone() ? ' · ' + timeZone() : ''}</span>
        </div>
        <div className="cc-grid-2-tight">
          <Field id="send-id" label="Message id" hint="Set it to deduplicate: a second send with the same id is accepted once, not delivered twice.">
            <input id="send-id" className="sx-input sx-mono" placeholder="Generated when empty" spellCheck={false} autoComplete="off" value={id} onChange={(e) => setId(e.target.value)} />
          </Field>
          <Field id="send-corr" label="Correlation id">
            <input id="send-corr" className="sx-input sx-mono" spellCheck={false} autoComplete="off" value={correlation} onChange={(e) => setCorrelation(e.target.value)} />
          </Field>
        </div>
        <Field label="Headers" hint="Passed through to plugins. Names starting with cicada- are reserved.">
          {headers.map((h, i) => (
            <div key={h.key} className="cc-header-row">
              <input className="sx-input sx-mono" placeholder="name" aria-label={`Header ${i + 1} name`} value={h.name}
                onChange={(e) => setHeaders((hs) => hs.map((x) => (x.key === h.key ? { ...x, name: e.target.value } : x)))} />
              <input className="sx-input sx-mono" placeholder="value" aria-label={`Header ${i + 1} value`} value={h.value}
                onChange={(e) => setHeaders((hs) => hs.map((x) => (x.key === h.key ? { ...x, value: e.target.value } : x)))} />
              <IconButton icon="trash" size="sm" label={`Remove header ${h.name || i + 1}`} onClick={() => setHeaders((hs) => hs.filter((x) => x.key !== h.key))} />
            </div>
          ))}
          <div><Button variant="ghost" size="sm" icon="plus" onClick={() => setHeaders((hs) => [...hs, { key: nextHeader.current++, name: '', value: '' }])}>Add header</Button></div>
        </Field>
        <fieldset className="cc-fieldset">
          <legend className="overline">Retry policy</legend>
          <div className="cc-form-grid">
            <Field id="send-retry" label="Backoff" hint={retryKind === 'none' ? 'Without a policy a failed handler run ends the message as failed.' : undefined}>
              <Select id="send-retry" value={retryKind} onChange={(v) => setRetryKind(v as RetryKind)}
                options={[{ value: 'none', label: 'No retries' }, { value: 'fixed', label: 'Fixed delay' }, { value: 'exponential', label: 'Exponential backoff' }]} />
            </Field>
            {retryKind !== 'none' ? (
              <Field id="retry-max" label="Max attempts" hint="Handler runs, including the first (1–100)">
                <input id="retry-max" className="sx-input sx-num" inputMode="numeric" value={retry.maxAttempts} onChange={field('maxAttempts')} />
              </Field>
            ) : null}
            {retryKind === 'fixed' ? (
              <Field id="retry-delay" label="Delay" hint="Before each retry, e.g. 30s, 2m, PT30S">
                <input id="retry-delay" className="sx-input sx-mono" value={retry.delay} onChange={field('delay')} />
              </Field>
            ) : retryKind === 'exponential' ? (
              <>
                <Field id="retry-initial" label="Initial delay" hint="e.g. 1s"><input id="retry-initial" className="sx-input sx-mono" value={retry.initial} onChange={field('initial')} /></Field>
                <Field id="retry-maxdelay" label="Max delay" hint="e.g. 1m"><input id="retry-maxdelay" className="sx-input sx-mono" value={retry.max} onChange={field('max')} /></Field>
                <Field id="retry-factor" label="Factor" hint="≥ 1"><input id="retry-factor" className="sx-input sx-num" inputMode="decimal" value={retry.factor} onChange={field('factor')} /></Field>
              </>
            ) : null}
          </div>
        </fieldset>
        <fieldset className="cc-fieldset">
          <legend className="overline">Keep the record</legend>
          <p className="sx-help cc-flat">How long the message’s record stays readable once it finished. A removed message stays a duplicate for its id and keeps its place in message chains.</p>
          <div className="cc-grid-2-tight">
            <KeepFields id="keep-done" outcome="done" draft={keepDone} onChange={setKeepDone} />
            <KeepFields id="keep-failed" outcome="failed" draft={keepFailed} onChange={setKeepFailed} />
          </div>
        </fieldset>
      </DialogBody>
      <DialogResult>{result}</DialogResult>
      <DialogFoot note="The node checks the payload when it is sent">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" icon="send" disabled={sending} onClick={() => void send()}>Send message</Button>
      </DialogFoot>
    </Dialog>
  );
}
