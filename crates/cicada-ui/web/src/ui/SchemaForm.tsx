// A form for a schema node (see lib/schema.ts): one field per property, nested objects as fieldsets, a JSON box for
// shapes a form cannot edit. The draft is held by the caller; every edit hands back a new draft.

import { useId, useState, type KeyboardEvent } from 'react';
import { pointer } from '../lib/diff';
import { hint, type Draft, type DraftObject, type Field, type FieldError, type ObjectNode } from '../lib/schema';
import { cx, Icon, Select } from '../halo';

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

function Label({ field, htmlFor }: { field: Field; htmlFor?: string }) {
  return (
    <label className="cc-label mono" htmlFor={htmlFor}>
      {field.name}
      {field.required ? <span className="cc-required" aria-hidden="true"> *</span> : null}
    </label>
  );
}

const caption = (f: Field) => <span className="cc-hint mono">{hint(f.node) + (f.required ? '' : ' · optional')}</span>;

function FieldInput({ field: f, value, path, onChange, errors, paths }: { field: Field; value: Draft | undefined; path: string; onChange: (value: Draft) => void; errors: readonly FieldError[]; paths: ReadonlySet<string> }) {
  const id = useId();
  const error = errorsFor(path, errors, paths);
  const errorText = error ? <span className="cc-hint cc-error" id={id + '-error'}>{error}</span> : null;
  const invalid = error !== null;
  const node = f.node;

  switch (node.kind) {
    case 'object':
      return (
        <fieldset className="cc-fieldset cc-span-all">
          <legend className="cc-label mono">
            {f.name}
            {f.required ? <span className="cc-required" aria-hidden="true"> *</span> : null}
            <span className="cc-hint mono">{' ' + hint(node) + (f.required ? '' : ' · optional')}</span>
          </legend>
          <div className="cc-form-grid">
            <Fields node={node} draft={(value as DraftObject | undefined) ?? {}} path={path} onChange={onChange} errors={errors} paths={paths} />
          </div>
          {errorText}
        </fieldset>
      );
    case 'boolean':
      return (
        <div className="cc-field">
          <label className="cc-check">
            <input type="checkbox" checked={Boolean(value)} onChange={(e) => onChange(e.target.checked)} />
            <span className="mono">{f.name}</span>
          </label>
          {caption(f)}
          {errorText}
        </div>
      );
    case 'enum': {
      const options = [{ value: '', label: f.required && !node.nullable ? 'Choose…' : '—' }, ...node.options];
      return (
        <div className="cc-field">
          <Label field={f} htmlFor={id} />
          <Select id={id} options={options} value={typeof value === 'string' ? value : ''} onChange={onChange} size="md" invalid={invalid} describedBy={id + '-error'} />
          {caption(f)}
          {errorText}
        </div>
      );
    }
    case 'list':
      return (
        <div className="cc-field cc-span-all">
          <Label field={f} htmlFor={id} />
          <Chips id={id} values={Array.isArray(value) ? value.map(String) : []} onChange={onChange} invalid={invalid} />
          {caption(f)}
          {errorText}
        </div>
      );
    case 'json':
      return (
        <div className="cc-field cc-span-all">
          <Label field={f} htmlFor={id} />
          <textarea
            id={id} className={cx('cc-textarea', invalid && 'cc-input-error')} rows={3} spellCheck={false} value={typeof value === 'string' ? value : ''}
            aria-required={f.required || undefined}
            aria-describedby={id + '-error'} aria-invalid={invalid || undefined} onChange={(e) => onChange(e.target.value)}
          />
          {caption(f)}
          {errorText}
        </div>
      );
    default: {
      const numeric = node.kind === 'integer' || node.kind === 'number';
      return (
        <div className="cc-field">
          <Label field={f} htmlFor={id} />
          <input
            id={id} className={cx('cc-input', numeric && 'ha-num', invalid && 'cc-input-error')} value={typeof value === 'string' ? value : ''}
            aria-required={f.required || undefined}
            spellCheck={false} autoComplete="off" inputMode={node.kind === 'integer' ? 'numeric' : node.kind === 'number' ? 'decimal' : undefined}
            aria-describedby={id + '-error'} aria-invalid={invalid || undefined} onChange={(e) => onChange(e.target.value)}
          />
          {caption(f)}
          {errorText}
        </div>
      );
    }
  }
}

/** A list of short values as chips: Enter or comma adds the typed value, Backspace in an empty box removes the last. */
function Chips({ id, values, onChange, invalid }: { id: string; values: string[]; onChange: (values: string[]) => void; invalid: boolean }) {
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
    <div className="cc-chips">
      <span className="cc-chip-list">
        {values.map((v, i) => (
          <span key={i} className="cc-chip">
            {v}
            <button type="button" className="cc-btn-reset cc-chip-remove" aria-label={'Remove ' + v} onClick={() => onChange(values.filter((_, j) => j !== i))}>
              <Icon name="x" size={12} />
            </button>
          </span>
        ))}
      </span>
      <input
        id={id} className={cx('cc-chip-input', invalid && 'cc-input-error')} placeholder="Add…" spellCheck={false} autoComplete="off"
        value={text} onChange={(e) => setText(e.target.value)} onKeyDown={onKeyDown} onBlur={add}
      />
    </div>
  );
}
