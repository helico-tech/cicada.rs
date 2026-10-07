// Types: the registered actor types with their template, contract hash and JSON Schemas (GET /types).

import { useEffect, useRef, useState } from 'react';
import { useConsole, usePageRefresh, useTitle } from '../app/context';
import { matchTemplate, templateParams, templatePrefix } from '../lib/address';
import { count } from '../lib/format';
import { href, replaceHash, type Query } from '../lib/routes';
import { summary } from '../lib/schema';
import { Badge, Card, cx, EmptyState, Icon } from '../halo';
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
    <div>
      <PageHead title="Types" description="Actor types registered in the store: their URI template, contract hash and schemas. Any node can create, send to and query them; only nodes hosting a type run its turns." />
      <div className="cc-types">
        <div className="cc-types-list">
          <Card title="Registered Types" subtitle={count(types.length, 'type')} flush>
            {types.length === 0 ? (
              <EmptyState title="No types registered" caption="A node registers the types it hosts when it starts." />
            ) : (
              <ul className="cc-plain-list">
                {types.map((t) => (
                  <li key={t.template}>
                    <button type="button" className={cx('cc-row-btn cc-type-item', t === sel && 'cc-selected')} aria-current={t === sel} onClick={() => choose(t.template)}>
                      <span className="mono cc-strong cc-small cc-break">{t.template}</span>
                      <span className="cc-row">
                        {t.state === null ? <Badge>Stateless</Badge> : <Badge tone="info">Stateful</Badge>}
                        <span className="cc-caption">{count(Object.keys(t.messages).length, 'message type')}</span>
                        <span className="cc-caption mono cc-push">{shortHash(t.hash)}</span>
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </div>
        <div className="cc-types-detail">
          {sel ? (
            <>
              <Card title={sel.template} subtitle={`${sel.state === null ? 'Stateless' : 'Stateful'} · ${count(messages.length, 'message type')}`}>
                <Definitions rows={[
                  ['Contract hash', sel.hash, 'mono cc-small'],
                  ['Parameters', params.length ? params.join(', ') : 'none — one fixed address', 'mono cc-small'],
                  ['Example', example === undefined ? 'Looking…' : example === null ? 'No instances yet'
                    : <a className="cc-link mono cc-small" href={href.actor(example)}>{example}</a>, example ? null : 'cc-muted'],
                  ['Instances', <a className="cc-link cc-small" href={href.actors({ type: sel.template })}>List all of this type</a>],
                ]} />
                <div className="cc-schema-block">
                  <div className="cc-overline">State schema</div>
                  {sel.state === null
                    ? <div className="cc-dashed">Stateless — instances of this type have no state.</div>
                    : <pre className="cc-json cc-json-pre">{JSON.stringify(sel.state, null, 2)}</pre>}
                </div>
              </Card>
              <Card title="Message Types" subtitle={count(messages.length, 'message type')} flush>
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
                          <span className="cc-faint"><Icon name={isOpen ? 'chevron-down' : 'chevron-right'} size={16} /></span>
                          <span className="mono cc-strong cc-small">{name}</span>
                          <span className="cc-caption mono cc-trunc cc-grow">{summary(sel.messages[name])}</span>
                        </button>
                        {isOpen ? <pre className="cc-json cc-json-pre cc-message-schema">{JSON.stringify(sel.messages[name], null, 2)}</pre> : null}
                      </li>
                    );
                  })}
                </ul>
              </Card>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}
