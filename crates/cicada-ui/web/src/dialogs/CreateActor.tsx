// Create Actor dialog: pick a type, fill its template parameters, and give stateful types an initial state.
// PUT /actors/{path} is idempotent: an existing actor is left as it is and reported.

import { useRef, useState, type ReactNode } from 'react';
import { useConsole } from '../app/context';
import { buildUri, templateParams } from '../lib/address';
import { href } from '../lib/routes';
import { Badge, Button, Callout, ProblemCallout, Select } from '../halo';
import { Dialog, DialogHeader } from '../ui/Dialog';
import { Mono } from '../ui/layout';
import { ValueEditor, type ValueEditorHandle } from '../ui/ValueEditor';

/** Opens the dialog, with [template] preselected when given. */
export function CreateActorDialog({ template, onClose }: { template?: string; onClose: () => void }) {
  const ctx = useConsole();
  const types = [...ctx.types].sort((a, b) => a.template.localeCompare(b.template));
  const [chosen, setChosen] = useState(() => (types.find((t) => t.template === template) ?? types[0])?.template ?? '');
  const type = types.find((t) => t.template === chosen);
  const [values, setValues] = useState<Record<string, string>>({});
  const [result, setResult] = useState<ReactNode>(null);
  const [creating, setCreating] = useState(false);
  const editor = useRef<ValueEditorHandle>(null);
  if (!type) return null;
  const params = templateParams(type.template);
  const built = buildUri(type.template, values);

  async function create() {
    if (creating || !type) return;
    setResult(null);
    if (!built.complete) {
      setResult(<Callout kind="user" title="Fill in every parameter" detail="The URI still has a {placeholder}." />);
      return;
    }
    let state;
    if (type.state !== null) {
      const read = editor.current?.read();
      if (!read) return;
      if (!read.ok) {
        setResult(<Callout kind="user" title="Invalid initial state" detail={`${read.message}: ${read.invalid.join('; ')}.`} />);
        return;
      }
      state = read.value;
    }
    setCreating(true);
    try {
      const { status } = await ctx.api.createActor(built.uri, state);
      const open = <a className="cc-link cc-nowrap" href={href.actor(built.uri)}>Open Actor</a>;
      setResult(status === 201
        ? <Callout kind="ok" title="Created" detail={<Mono className="cc-small">{built.uri}</Mono>} action={open} />
        : <Callout kind="info" title="Already exists" detail={<><Mono className="cc-small">{built.uri}</Mono> already existed. Nothing was changed.</>} action={open} />);
      ctx.refresh();
    } catch (e) {
      setResult(<ProblemCallout error={e} retry={() => void create()} />);
    } finally {
      setCreating(false);
    }
  }

  return (
    <Dialog labelledBy="create-title" onClose={onClose}>
      <DialogHeader id="create-title" title="Create Actor" caption="Pick a type, fill its template parameters and, for stateful types, an initial state." onClose={onClose} />
      <div className="cc-dialog-body">
        <div className="cc-field">
          <label className="cc-label" htmlFor="create-type">Type template</label>
          <Select id="create-type" size="md" value={type.template} onChange={(v) => { setChosen(v); setValues({}); setResult(null); }}
            options={types.map((t) => ({ value: t.template, label: t.template + (t.state === null ? ' (stateless)' : '') }))} />
        </div>
        <div className="cc-form-grid">
          {params.length === 0 ? (
            <div className="cc-hint cc-span-all">This template has no parameters: it names exactly one actor.</div>
          ) : params.map((name) => (
            <div key={type.template + name} className="cc-field">
              <label className="cc-label mono" htmlFor={'param-' + name}>{'{' + name + '}'}</label>
              <input id={'param-' + name} className="cc-input mono" placeholder={name} spellCheck={false} autoComplete="off"
                value={values[name] ?? ''} onChange={(e) => setValues((v) => ({ ...v, [name]: e.target.value }))} />
              <span className="cc-hint">Any text; it is percent-encoded in the URI.</span>
            </div>
          ))}
        </div>
        <div className="cc-uri-box">
          <div className="cc-overline">Resulting URI</div>
          <div className="mono cc-uri-preview">
            <span className="cc-uri-literal">actor://</span>
            {built.parts.map((p, i) => (
              <span key={i}>
                {i > 0 ? <span className="cc-uri-literal">/</span> : null}
                <span className={!p.param ? 'cc-uri-literal' : p.filled ? 'cc-uri-param' : 'cc-uri-missing'}>{p.text}</span>
              </span>
            ))}
          </div>
        </div>
        {type.state === null ? (
          <section className="cc-section">
            <div className="cc-section-head"><h3 className="cc-section-title">Initial state</h3></div>
            <div className="cc-dashed cc-row"><Badge>Stateless</Badge>This type keeps no state; the actor is created with a mailbox only.</div>
          </section>
        ) : (
          <ValueEditor key={type.template} title="Initial state" schema={type.state} blank={undefined} label="Initial state" handle={editor} />
        )}
        <div className="cc-result">{result}</div>
      </div>
      <footer className="cc-dialog-foot">
        <Button variant="ghost" onClick={onClose}>Cancel</Button>
        <Button variant="primary" icon="plus" disabled={creating} onClick={() => void create()}>Create Actor</Button>
      </footer>
    </Dialog>
  );
}
