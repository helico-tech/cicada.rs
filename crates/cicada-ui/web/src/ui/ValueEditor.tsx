// A JSON value editor with Form and Raw JSON modes, for message payloads and initial states. The form comes from the
// schema; switching to raw keeps what was typed, and switching back keeps it when the form can show it.

import { useImperativeHandle, useMemo, useState, type Ref } from 'react';
import type { Json } from '../api/wire';
import { describe, draftOf, emptyDraft, readDraft, skeleton, type DraftObject, type FieldError } from '../lib/schema';
import { Callout, Segmented } from '../sx';
import { SchemaForm } from './SchemaForm';

export type ReadResult = { ok: true; value: Json | undefined } | { ok: false; message: string; invalid: string[] };

export interface ValueEditorHandle {
  /** The value to send, or why it cannot be sent (and the fields are marked). */
  read(): ReadResult;
}

function parse(text: string, blank: Json | undefined): { ok: true; value: Json | undefined } | { ok: false; message: string } {
  if (text.trim() === '') return { ok: true, value: blank };
  try {
    return { ok: true, value: JSON.parse(text) as Json };
  } catch (e) {
    return { ok: false, message: 'Not valid JSON: ' + (e instanceof Error ? e.message : String(e)) };
  }
}

/**
 * The editor for [schema], headed [title]. [blank] is the value of an empty raw box (`{}` for a payload, undefined for
 * "no state"). [label] names the value (the message type) in the form's legend and the editor's labels.
 */
export function ValueEditor({ title, schema, blank, label, handle }: { title: string; schema: unknown; blank: Json | undefined; label: string; handle: Ref<ValueEditorHandle> }) {
  const node = useMemo(() => describe(schema ?? {}), [schema]);
  const formable = node.kind === 'object';
  const [mode, setMode] = useState<'form' | 'raw'>(formable ? 'form' : 'raw');
  const [draft, setDraft] = useState<DraftObject>(() => (formable ? (emptyDraft(node) as DraftObject) : {}));
  const [raw, setRaw] = useState(() => (formable || blank === undefined ? '' : JSON.stringify(skeleton(node), null, 2)));
  const [errors, setErrors] = useState<FieldError[]>([]);
  const [note, setNote] = useState<string | null>(null);

  useImperativeHandle(handle, () => ({
    read(): ReadResult {
      setNote(null);
      if (mode === 'raw' || node.kind !== 'object') {
        const parsed = parse(raw, blank);
        return parsed.ok ? parsed : { ok: false, message: parsed.message, invalid: ['the JSON'] };
      }
      const { value, errors: found } = readDraft(node, draft);
      setErrors(found);
      if (found.length) {
        return {
          ok: false,
          message: `${found.length === 1 ? '1 field is' : found.length + ' fields are'} invalid`,
          invalid: found.map((e) => `${e.path} ${e.message.toLowerCase()}`),
        };
      }
      return { ok: true, value: value as Json };
    },
  }), [mode, node, raw, blank, draft]);

  const switchTo = (next: 'form' | 'raw') => {
    setNote(null);
    if (node.kind !== 'object' || next === mode) return;
    if (next === 'raw') {
      setRaw(JSON.stringify(readDraft(node, draft).value, null, 2));
      setMode('raw');
      return;
    }
    const parsed = parse(raw, blank);
    const nextDraft = parsed.ok ? draftOf(node, parsed.value ?? {}) : null;
    if (nextDraft === null) {
      // Stay in raw: the form would drop or misread something.
      setNote(parsed.ok ? 'This JSON has keys or shapes the form cannot show; switching would lose them.' : parsed.message);
      return;
    }
    setDraft(nextDraft as DraftObject);
    setErrors([]);
    setMode('form');
  };

  return (
    <section className="cc-section">
      <div className="cc-section-head">
        <h3 className="heading cc-section-title">{title}</h3>
        {formable ? (
          <Segmented label={`${label} editor`} value={mode} onChange={switchTo}
            items={[{ value: 'form', label: 'Form' }, { value: 'raw', label: 'Raw JSON' }]} />
        ) : null}
      </div>
      {note ? <Callout kind="info" title="Kept in Raw JSON" detail={note} /> : null}
      {mode === 'form' && node.kind === 'object' ? (
        <fieldset className="cc-fieldset">
          <legend className="overline"><span className="cc-legend-name">{label}</span> · from its JSON Schema</legend>
          {node.fields.length === 0
            ? <p className="caption cc-muted cc-flat">{label} has no fields. It is sent as {'{}'}.</p>
            : <SchemaForm node={node} draft={draft} onChange={setDraft} errors={errors} />}
        </fieldset>
      ) : (
        <div className="sx-field">
          <textarea className="sx-textarea sx-mono cc-code-input" rows={8} spellCheck={false} aria-label={`${label} as JSON`} value={raw} onChange={(e) => setRaw(e.target.value)} />
          <span className="sx-help">Sent as typed. The node decodes it with the class’s serializer; a mistake comes back as a 422.</span>
        </div>
      )}
    </section>
  );
}
