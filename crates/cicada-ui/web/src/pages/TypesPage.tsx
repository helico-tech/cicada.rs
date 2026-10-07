// Types: the registered actor types with their template, contract hash and JSON Schemas (GET /types).

import { useEffect, useRef, useState } from 'react';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { matchTemplate, templateParams, templatePrefix } from '../lib/address';
import { count } from '../lib/format';
import { href, replaceHash, type Query } from '../lib/routes';
import { summary } from '../lib/schema';
import { Badge, Count, EmptyState, Icon, Widget } from '../sx';
import { Definitions, PageHead } from '../ui/layout';

const shortHash = (hash: string): string => (hash.length > 20 ? hash.slice(0, 13) + '…' + hash.slice(-4) : hash);

export function TypesPage({ query }: { query: Query }) {
  const ctx = useConsole();
  useTitle('Types');
  const [selected, setSelected] = useState<string | null>(query.template ?? null);
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const [examples, setExamples] = useState<ReadonlyMap<string, string | null>>(new Map());
  const examplesRef = useRef(examples);
  examplesRef.current = examples;
  const types = ctx.types;
  const sel = types.find((t) => t.template === selected) ?? types[0] ?? null;

  usePageRefresh(async () => {
    await ctx.reloadTypes();
    if (sel && !examplesRef.current.has(sel.template)) {
      // An example instance: the first one listed under the template's prefix that really is of this type.
      const page = await ctx.api.listActors({ prefix: templatePrefix(sel.template), limit: 50 });
      const example = page.items.find((uri) => matchTemplate(sel.template, uri)) ?? null;
      setExamples((m) => new Map(m).set(sel.template, example));
    }
  });

  const choose = (template: string) => {
    setSelected(template);
    setOpen(new Set());
    replaceHash(href.types({ template }));
  };
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    ctx.refresh(); // loads the example of the newly selected type
  }, [selected, ctx]);

  const messages = sel ? Object.keys(sel.messages).sort() : [];
  const params = sel ? templateParams(sel.template) : [];
  const example = sel ? examples.get(sel.template) : undefined;
  return (
    <div className="cc-page">
      <PageHead title="Types" description="Actor types registered in the store: their URI template, contract hash and schemas. Any node can create, send to and query them; only nodes hosting a type run its turns." />
      <div className="cc-types">
        <Widget title="Registered types" count={<Count>{String(types.length)}</Count>} className="cc-types-list" flush>
          {types.length === 0 ? (
            <EmptyState title="No types registered" caption="A node registers the types it hosts when it starts." />
          ) : (
            <ul className="cc-plain-list">
              {types.map((t) => (
                <li key={t.template}>
                  <button type="button" className="cc-row-btn cc-type-item" aria-current={t === sel} onClick={() => choose(t.template)}>
                    <span className="mono cc-break">{t.template}</span>
                    <span className="cc-row cc-gap-8">
                      {t.state === null ? <Badge tone="outline" size="sm">Stateless</Badge> : <Badge tone="info" size="sm">Stateful</Badge>}
                      <span className="caption cc-muted">{count(Object.keys(t.messages).length, 'message type')}</span>
                      <span className="mono-sm cc-subtle cc-push">{shortHash(t.hash)}</span>
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Widget>
        <div className="cc-types-detail">
          {sel ? (
            <>
              <Widget title={<span className="mono cc-widget-mono">{sel.template}</span>} subtitle={`${sel.state === null ? 'Stateless' : 'Stateful'} · ${count(messages.length, 'message type')}`} flush>
                <Definitions rows={[
                  ['Contract hash', sel.hash, 'mono-sm'],
                  ['Parameters', params.length ? params.join(', ') : 'none — one fixed address', 'mono-sm'],
                  ['Example', example === undefined ? 'Looking…' : example === null ? 'No instances yet'
                    : <a className="mono-sm" href={href.actor(example)}>{example}</a>, example ? null : 'cc-muted'],
                  ['Instances', <a href={href.actors({ type: sel.template })}>List all of this type</a>],
                ]} />
                <div className="cc-schema-block">
                  <div className="overline cc-subtle">State schema</div>
                  {sel.state === null
                    ? <div className="cc-dashed">Stateless — instances of this type have no state.</div>
                    : <pre className="cc-json cc-json-pre">{JSON.stringify(sel.state, null, 2)}</pre>}
                </div>
              </Widget>
              <Widget title="Message types" count={<Count>{String(messages.length)}</Count>} flush>
                <ul className="cc-plain-list">
                  {messages.map((name) => {
                    const isOpen = open.has(name);
                    return (
                      <li key={name}>
                        <button type="button" className="cc-row-btn cc-message-item" aria-expanded={isOpen} onClick={() => setOpen((s) => {
                          const next = new Set(s);
                          if (isOpen) next.delete(name);
                          else next.add(name);
                          return next;
                        })}>
                          <span className="cc-toggle-icon"><Icon name={isOpen ? 'chevron-down' : 'chevron-right'} /></span>
                          <span className="label">{name}</span>
                          <span className="mono-sm cc-muted cc-trunc cc-grow">{summary(sel.messages[name])}</span>
                        </button>
                        {isOpen ? <pre className="cc-json cc-json-pre cc-message-schema">{JSON.stringify(sel.messages[name], null, 2)}</pre> : null}
                      </li>
                    );
                  })}
                </ul>
              </Widget>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}
