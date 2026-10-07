// A form for a schema node (see lib/schema.ts): one field per property, nested objects as fieldsets, a JSON box for
// shapes a form cannot edit. The draft is held by the caller; every edit hands back a new draft.

import { useId, useState, type KeyboardEvent, type ReactNode } from 'react';
import { pointer } from '../lib/diff';
import { hint, type Draft, type DraftObject, type Field, type FieldError, type ObjectNode } from '../lib/schema';
import { cx, FieldError as ErrorText, Icon, Select } from '../sx';

/** The message under the field at [path]: errors of a list's items show under the list. */
function errorsFor(path: string, errors: readonly FieldError[], fieldPaths: ReadonlySet<string>): string | null {
  const own = errors.filter((e) => {
    if (e.path === path) return true;
    if (!e.path.startsWith(path + '/')) return false;
    // A deeper field that has its own slot takes the error.
    return ![...fieldPaths].some((p) => p.length > path.length && (e.path === p || e.path.startsWith(p + '/')));
  });
  return own.length ? own.map((e) => e.message).join('; ') : null;
}

function collectPaths(node: ObjectNode, path: string, out: Set<string>): Set<string> {
  for (const f of node.fields) {
    const p = pointer(path, f.name);
    out.add(p);
    if (f.node.kind === 'object') collectPaths(f.node, p, out);
  }
  return out;
}

export function SchemaForm({ node, draft, onChange, errors }: { node: ObjectNode; draft: DraftObject; onChange: (draft: DraftObject) => void; errors: readonly FieldError[] }) {
  const paths = collectPaths(node, '', new Set());
  return <div className="cc-form-grid"><Fields node={node} draft={draft} path="" onChange={onChange} errors={errors} paths={paths} /></div>;
}

interface FieldsProps {
  node: ObjectNode;
  draft: DraftObject;
  path: string;
  onChange: (draft: DraftObject) => void;
  errors: readonly FieldError[];
  paths: ReadonlySet<string>;
}

function Fields({ node, draft, path, onChange, errors, paths }: FieldsProps) {
  return (
    <>
      {node.fields.map((f) => (
        <FieldInput
          key={f.name} field={f} value={draft[f.name]} path={pointer(path, f.name)} errors={errors} paths={paths}
          onChange={(v) => onChange({ ...draft, [f.name]: v })}
        />
      ))}
    </>
  );
}

function LabelText({ field }: { field: Field }) {
  return <><span>{field.name}</span>{field.required ? <span className="cc-required" aria-hidden="true">*</span> : null}</>;
}

const help = (f: Field) => <span className="sx-help">{hint(f.node) + (f.required ? '' : ' · optional')}</span>;

/** A field: label, control, the type as help text, and the error. */
function Slot({ f, id, wide, label = true, error, errorId, children }: { f: Field; id: string; wide?: boolean; label?: boolean; error: string | null; errorId: string; children: ReactNode }) {
  return (
    <div className={cx('sx-field', wide && 'cc-span-all')}>
      {label ? <label className="sx-label" htmlFor={id}><LabelText field={f} /></label> : null}
      {children}
      {help(f)}
      {error ? <ErrorText id={errorId}>{error}</ErrorText> : null}
    </div>
  );
}

function FieldInput({ field: f, value, path, onChange, errors, paths }: { field: Field; value: Draft | undefined; path: string; onChange: (value: Draft) => void; errors: readonly FieldError[]; paths: ReadonlySet<string> }) {
  const id = useId();
  const errorId = id + '-error';
  const error = errorsFor(path, errors, paths);
  const invalid = error !== null;
  const node = f.node;

  switch (node.kind) {
    case 'object':
      return (
        <fieldset className="cc-fieldset cc-span-all">
          <legend className="overline">
            {f.name}
            {f.required ? <span className="cc-required" aria-hidden="true">*</span> : null}
            <span className="cc-legend-hint">{' · ' + hint(node) + (f.required ? '' : ' · optional')}</span>
          </legend>
          <div className="cc-form-grid">
            <Fields node={node} draft={(value as DraftObject | undefined) ?? {}} path={path} onChange={onChange} errors={errors} paths={paths} />
          </div>
          {error ? <ErrorText id={errorId}>{error}</ErrorText> : null}
        </fieldset>
      );
    case 'boolean': {
      const on = Boolean(value);
      return (
        <div className="sx-field">
          <span className="sx-label" id={id + '-label'}><LabelText field={f} /></span>
          <button type="button" className="sx-switch" role="switch" aria-checked={on} aria-labelledby={id + '-label'} onClick={() => onChange(!on)}>
            <span className="sx-switch-track" />
            <span className="sx-switch-state">{on ? 'true' : 'false'}</span>
          </button>
          {help(f)}
          {error ? <ErrorText id={errorId}>{error}</ErrorText> : null}
        </div>
      );
    }
    case 'enum': {
      const options = [{ value: '', label: f.required && !node.nullable ? 'Choose…' : '—' }, ...node.options];
      return (
        <Slot f={f} id={id} error={error} errorId={errorId}>
          <Select id={id} options={options} value={typeof value === 'string' ? value : ''} onChange={onChange} invalid={invalid} describedBy={errorId} />
        </Slot>
      );
    }
    case 'list':
      return (
        <Slot f={f} id={id} wide error={error} errorId={errorId}>
          <Tags id={id} values={Array.isArray(value) ? value.map(String) : []} onChange={onChange} invalid={invalid} describedBy={errorId} />
        </Slot>
      );
    case 'json':
      return (
        <Slot f={f} id={id} wide error={error} errorId={errorId}>
          <textarea
            id={id} className={cx('sx-textarea sx-mono cc-code-input', invalid && 'is-invalid')} rows={3} spellCheck={false} value={typeof value === 'string' ? value : ''}
            aria-required={f.required || undefined}
            aria-describedby={errorId} aria-invalid={invalid || undefined} onChange={(e) => onChange(e.target.value)}
          />
        </Slot>
      );
    default: {
      const numeric = node.kind === 'integer' || node.kind === 'number';
      return (
        <Slot f={f} id={id} error={error} errorId={errorId}>
          <input
            id={id} className={cx('sx-input', numeric && 'sx-num', invalid && 'is-invalid')} value={typeof value === 'string' ? value : ''}
            aria-required={f.required || undefined}
            spellCheck={false} autoComplete="off" inputMode={node.kind === 'integer' ? 'numeric' : node.kind === 'number' ? 'decimal' : undefined}
            aria-describedby={errorId} aria-invalid={invalid || undefined} onChange={(e) => onChange(e.target.value)}
          />
        </Slot>
      );
    }
  }
}

/** A list of short values as tags: Enter or comma adds the typed value, Backspace in an empty box removes the last. */
function Tags({ id, values, onChange, invalid, describedBy }: { id: string; values: string[]; onChange: (values: string[]) => void; invalid: boolean; describedBy: string }) {
  const [text, setText] = useState('');
  const add = () => {
    const v = text.trim();
    if (v) onChange([...values, v]);
    setText('');
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      add();
    } else if (e.key === 'Backspace' && text === '' && values.length) {
      onChange(values.slice(0, -1));
    }
  };
  return (
    <div className={cx('sx-inputgroup sx-tagfield', invalid && 'is-invalid')}>
      {values.map((v, i) => (
        <span key={i} className="sx-tag">
          <span>{v}</span>
          <button type="button" className="sx-tag-x" aria-label={'Remove ' + v} onClick={() => onChange(values.filter((_, j) => j !== i))}>
            <Icon name="close" size={12} />
          </button>
        </span>
      ))}
      <input
        id={id} placeholder="Add…" spellCheck={false} autoComplete="off" aria-invalid={invalid || undefined} aria-describedby={describedBy}
        value={text} onChange={(e) => setText(e.target.value)} onKeyDown={onKeyDown} onBlur={add}
      />
    </div>
  );
}
