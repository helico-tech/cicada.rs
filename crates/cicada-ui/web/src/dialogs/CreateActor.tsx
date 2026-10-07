// Create Actor dialog: pick a type, fill its template parameters, and give stateful types an initial state.
// PUT /actors/{path} is idempotent: an existing actor is left as it is and reported.

import { useRef, useState, type ReactNode } from 'react';
import { useConsole } from '../app/context';
import { buildUri, templateParams } from '../lib/address';
import { href } from '../lib/routes';
import { Badge, Button, Callout, Field, LinkButton, ProblemCallout, Select } from '../sx';
import { Dialog, DialogBody, DialogFoot, DialogHeader, DialogResult } from '../ui/Dialog';
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
      const open = <LinkButton size="sm" href={href.actor(built.uri)}>Open actor</LinkButton>;
      setResult(status === 201
        ? <Callout kind="ok" title="Created" detail={<Mono>{built.uri}</Mono>} action={open} />
        : <Callout kind="info" title="Already exists" detail={<><Mono>{built.uri}</Mono> already existed. Nothing was changed.</>} action={open} />);
      ctx.refresh();
    } catch (e) {
      setResult(<ProblemCallout error={e} retry={() => void create()} />);
    } finally {
      setCreating(false);
    }
  }

  return (
    <Dialog labelledBy="create-title" onClose={onClose}>
      <DialogHeader id="create-title" title="Create actor" caption="Pick a type, fill its template parameters and, for stateful types, an initial state." onClose={onClose} />
      <DialogBody>
        <Field id="create-type" label="Type template" required>
          <Select id="create-type" mono value={type.template} onChange={(v) => { setChosen(v); setValues({}); setResult(null); }}
            options={types.map((t) => ({ value: t.template, label: t.template + (t.state === null ? ' (stateless)' : '') }))} />
        </Field>
        <div className="cc-form-grid">
          {params.length === 0 ? (
            <p className="sx-help cc-span-all cc-flat">This template has no parameters: it names exactly one actor.</p>
          ) : params.map((name) => (
            <Field key={type.template + name} id={'param-' + name} label={<span className="mono">{'{' + name + '}'}</span>} required hint="Any text; it is percent-encoded in the URI.">
              <input id={'param-' + name} className="sx-input sx-mono" placeholder={name} spellCheck={false} autoComplete="off"
                value={values[name] ?? ''} onChange={(e) => setValues((v) => ({ ...v, [name]: e.target.value }))} />
            </Field>
          ))}
        </div>
        <div className="cc-uri-box">
          <div className="overline cc-subtle">Resulting URI</div>
          <div className="cc-uri-preview">
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
            <h3 className="heading cc-section-title">Initial state</h3>
            <div className="cc-dashed"><Badge tone="outline">Stateless</Badge><span>This type keeps no state; the actor is created with a mailbox only.</span></div>
          </section>
        ) : (
          <ValueEditor key={type.template} title="Initial state" schema={type.state} blank={undefined} label="Initial state" handle={editor} />
        )}
      </DialogBody>
      <DialogResult>{result}</DialogResult>
      <DialogFoot>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" icon="plus" disabled={creating} onClick={() => void create()}>Create actor</Button>
      </DialogFoot>
    </Dialog>
  );
}
