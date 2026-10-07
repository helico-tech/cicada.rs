// Send Message drawer: POST /actors/{path}/messages with a payload from the message type's schema, a delivery time,
// and optionally an id, correlation id, headers, a retry policy and a persistence (retention) policy.

import { useRef, useState, type ReactNode } from 'react';
import { ApiError } from '../api/client';
import type { RetryPolicy, SendRequest, TypeView } from '../api/generated';
import { useConsole } from '../app/context';
import { clock, dateTime, localInputValue, relative, timeZone } from '../lib/format';
import { blankKeep, readRetention, type KeepDraft, type KeepKind } from '../lib/retention';
import { href } from '../lib/routes';
import { Button, Callout, Icon, ProblemCallout, Select, Tabs } from '../halo';
import { Dialog, DialogHeader } from '../ui/Dialog';
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

function Field({ id, label, hint, children }: { id?: string; label: ReactNode; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="cc-field">
      {id ? <label className="cc-label" htmlFor={id}>{label}</label> : <span className="cc-label">{label}</span>}
      {children}
      {hint ? <span className="cc-hint">{hint}</span> : null}
    </div>
  );
}

const KEEP_OPTIONS: Array<{ value: KeepKind; label: string }> = [
  { value: 'default', label: 'Store default' },
  { value: 'remove', label: 'Remove at once' },
  { value: 'for', label: 'Keep for a time' },
  { value: 'last', label: 'Keep the last N' },
];

function KeepFields({ id, outcome, draft, onChange }: { id: string; outcome: string; draft: KeepDraft; onChange: (draft: KeepDraft) => void }) {
  return (
    <div className="cc-keep-row">
      <Field id={id} label={`When ${outcome}`}>
        <Select id={id} size="md" value={draft.kind} options={KEEP_OPTIONS} onChange={(v) => onChange({ ...draft, kind: v as KeepKind })} />
      </Field>
      {draft.kind === 'for' ? (
        <Field id={id + '-age'} label="Keep for" hint="e.g. 30d, 12h, PT30M">
          <input id={id + '-age'} className="cc-input mono" value={draft.age} onChange={(e) => onChange({ ...draft, age: e.target.value })} />
        </Field>
      ) : draft.kind === 'last' ? (
        <Field id={id + '-count'} label="Keep the last" hint="Of this actor, with this outcome">
          <input id={id + '-count'} className="cc-input ha-num" inputMode="numeric" value={draft.count} onChange={(e) => onChange({ ...draft, count: e.target.value })} />
        </Field>
      ) : <div />}
    </div>
  );
}

/** The drawer for the actor [uri] of [type]. */
export function SendMessageDialog({ uri, type, onClose }: { uri: string; type: TypeView; onClose: () => void }) {
  const ctx = useConsole();
  const names = Object.keys(type.messages).sort();
  const [messageType, setMessageType] = useState(names[0] ?? '');
  const editor = useRef<ValueEditorHandle>(null);
  const [delivery, setDelivery] = useState<Delivery>('now');
  const [delayAmount, setDelayAmount] = useState('5');
  const [delayUnit, setDelayUnit] = useState<Unit>('minutes');
  const [at, setAt] = useState(() => localInputValue(Date.now() + 3600_000));
  const [advanced, setAdvanced] = useState(false);
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
  // A generated id is kept until the node gives a definite answer, so Retry Now cannot send twice.
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
      const open = (label: string) => <a className="cc-link cc-nowrap" href={href.message(body.id)}>{label}</a>;
      setResult(status === 202 ? (
        <Callout kind="ok" title="Accepted" action={open('Open Message')}
          detail={<><Mono className="cc-small">{body.id}</Mono> · delivers <span className="ha-num">{body.deliverAt ? `${dateTime(body.deliverAt)} (${relative(body.deliverAt)})` : '—'}</span></>} />
      ) : (
        <Callout kind="info" title="Duplicate — nothing was sent" action={open('Open Existing')}
          detail={<>A message with id <Mono className="cc-small">{body.id}</Mono> was already accepted for this actor.</>} />
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

  return (
    <Dialog kind="drawer" labelledBy="send-title" onClose={onClose}>
      <DialogHeader id="send-title" title="Send Message" caption="Enqueue a message in an actor’s mailbox." onClose={onClose} />
      <div className="cc-dialog-body">
        <Field id="send-target" label="Target actor" hint={<>Type <Mono>{type.template}</Mono>.</>}>
          <input id="send-target" className="cc-input mono" readOnly value={uri} />
        </Field>
        <Field id="send-type" label="Message type"
          hint={<>Types accepted by <Mono>{type.template}</Mono> · contract <span className="mono" title={type.hash}>{type.hash.slice(0, 13) + '…' + type.hash.slice(-4)}</span></>}>
          <Select id="send-type" options={names} value={messageType} size="md" onChange={(v) => { setMessageType(v); setResult(null); }} />
        </Field>
        <ValueEditor key={messageType} title="Payload" schema={type.messages[messageType]} blank={{}} label={messageType} handle={editor} />
        <section className="cc-section">
          <h3 className="cc-section-title">Delivery</h3>
          <Tabs variant="segmented" label="Delivery" value={delivery} onChange={setDelivery}
            items={[{ value: 'now', label: 'Now' }, { value: 'delay', label: 'After Delay' }, { value: 'at', label: 'At Time' }]} />
          {delivery === 'delay' ? (
            <div className="cc-row">
              <input className="cc-input ha-num cc-delay" inputMode="decimal" aria-label="Delay amount" value={delayAmount} onChange={(e) => setDelayAmount(e.target.value)} />
              <Select options={Object.keys(UNITS)} value={delayUnit} label="Delay unit" size="md" onChange={(v) => setDelayUnit(v as Unit)} />
            </div>
          ) : delivery === 'at' ? (
            <input className="cc-input cc-at" type="datetime-local" aria-label="Deliver at" value={at} onChange={(e) => setAt(e.target.value)} />
          ) : null}
          <span className="cc-hint">Delivers <b className="ha-num cc-ink">{deliverText}</b>{timeZone() ? ' · ' + timeZone() : ''}</span>
        </section>
        <section className="cc-advanced">
          <button type="button" className="cc-row-btn cc-advanced-toggle" aria-expanded={advanced} onClick={() => setAdvanced((v) => !v)}>
            <Icon name={advanced ? 'chevron-down' : 'chevron-right'} size={16} />
            Advanced
            <span className="cc-caption">Message id, correlation, headers, retry and persistence</span>
          </button>
          {advanced ? (
            <div className="cc-advanced-body">
              <Field id="send-id" label="Message ID" hint="Set it to deduplicate: a second send with the same id is accepted once, not delivered twice.">
                <input id="send-id" className="cc-input mono" placeholder="Generated when empty" spellCheck={false} autoComplete="off" value={id} onChange={(e) => setId(e.target.value)} />
              </Field>
              <Field id="send-corr" label="Correlation ID">
                <input id="send-corr" className="cc-input mono" spellCheck={false} autoComplete="off" value={correlation} onChange={(e) => setCorrelation(e.target.value)} />
              </Field>
              <Field label="Headers" hint="Passed through to plugins. Names starting with cicada- are reserved.">
                <div className="cc-header-rows">
                  {headers.map((h, i) => (
                    <div key={h.key} className="cc-header-row">
                      <input className="cc-input mono" placeholder="name" aria-label={`Header ${i + 1} name`} value={h.name}
                        onChange={(e) => setHeaders((hs) => hs.map((x) => (x.key === h.key ? { ...x, name: e.target.value } : x)))} />
                      <input className="cc-input mono" placeholder="value" aria-label={`Header ${i + 1} value`} value={h.value}
                        onChange={(e) => setHeaders((hs) => hs.map((x) => (x.key === h.key ? { ...x, value: e.target.value } : x)))} />
                      <button type="button" className="ha-iconbtn ha-iconbtn-ghost ha-iconbtn-sm" aria-label={`Remove header ${h.name || i + 1}`}
                        onClick={() => setHeaders((hs) => hs.filter((x) => x.key !== h.key))}>
                        <Icon name="x" size={16} />
                      </button>
                    </div>
                  ))}
                </div>
                <div><Button variant="ghost" size="sm" icon="plus" onClick={() => setHeaders((hs) => [...hs, { key: nextHeader.current++, name: '', value: '' }])}>Add Header</Button></div>
              </Field>
              <Field id="send-retry" label="Retry policy" hint="Without a policy a failed handler run ends the message as failed.">
                <Select id="send-retry" size="md" value={retryKind} onChange={(v) => setRetryKind(v as RetryKind)}
                  options={[{ value: 'none', label: 'No retries' }, { value: 'fixed', label: 'Fixed delay' }, { value: 'exponential', label: 'Exponential backoff' }]} />
              </Field>
              {retryKind !== 'none' ? (
                <div className="cc-form-grid">
                  <Field id="retry-max" label="Max attempts" hint="Handler runs, including the first (1–100)">
                    <input id="retry-max" className="cc-input ha-num" inputMode="numeric" value={retry.maxAttempts} onChange={field('maxAttempts')} />
                  </Field>
                  {retryKind === 'fixed' ? (
                    <Field id="retry-delay" label="Delay" hint="Before each retry, e.g. 30s, 2m, PT30S">
                      <input id="retry-delay" className="cc-input mono" value={retry.delay} onChange={field('delay')} />
                    </Field>
                  ) : (
                    <>
                      <Field id="retry-initial" label="Initial delay" hint="e.g. 1s"><input id="retry-initial" className="cc-input mono" value={retry.initial} onChange={field('initial')} /></Field>
                      <Field id="retry-maxdelay" label="Max delay" hint="e.g. 1m"><input id="retry-maxdelay" className="cc-input mono" value={retry.max} onChange={field('max')} /></Field>
                      <Field id="retry-factor" label="Factor" hint="≥ 1"><input id="retry-factor" className="cc-input ha-num" inputMode="decimal" value={retry.factor} onChange={field('factor')} /></Field>
                    </>
                  )}
                </div>
              ) : null}
              <div className="cc-field">
                <span className="cc-label">Persistence</span>
                <span className="cc-hint">How long the message’s record stays readable once it finished. A removed message stays a duplicate for its id and keeps its place in message chains.</span>
              </div>
              <KeepFields id="keep-done" outcome="done" draft={keepDone} onChange={setKeepDone} />
              <KeepFields id="keep-failed" outcome="failed" draft={keepFailed} onChange={setKeepFailed} />
            </div>
          ) : null}
        </section>
      </div>
      <footer className="cc-drawer-foot">
        <div className="cc-result">{result}</div>
        <div className="cc-dialog-actions">
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" icon="arrow-right" disabled={sending} onClick={() => void send()}>Send Message</Button>
        </div>
      </footer>
    </Dialog>
  );
}
