// A collapsible JSON viewer with change highlights (amber tint and the previous value), as in the design's State card.

import type { ReactNode } from 'react';
import { pointer, type Change } from '../lib/diff';
import { Icon } from '../sx';

export interface JsonViewOptions {
  /** Whether the container at [path] is expanded. */
  isOpen?: (path: string, depth: number, size: number) => boolean;
  onToggle?: (path: string, open: boolean) => void;
  /** What changed since the last refresh, by JSON Pointer. */
  changes?: ReadonlyMap<string, Change> | null;
}

const Punct = ({ children }: { children: ReactNode }) => <span className="p">{children}</span>;

function Scalar({ value }: { value: unknown }) {
  if (typeof value === 'string') return <span className="s">{JSON.stringify(value)}</span>;
  if (typeof value === 'number') return <span className="n">{String(value)}</span>;
  return <span className="b">{String(value)}</span>;
}

function preview(value: unknown): string {
  const text = JSON.stringify(value) ?? 'undefined';
  return text.length > 40 ? text.slice(0, 39) + '…' : text;
}

function Note({ change }: { change: Change | undefined }) {
  if (!change) return null;
  return <span className="c">{change.before === undefined ? ' new' : ` was ${preview(change.before)}`}</span>;
}

export function JsonView({ value, isOpen = () => true, onToggle = () => {}, changes = null }: { value: unknown } & JsonViewOptions) {
  return (
    <div className="cc-json cc-json-box">
      <Lines value={value} name={null} path="" depth={0} last opts={{ isOpen, onToggle, changes }} />
    </div>
  );
}

interface LineProps {
  value: unknown;
  name: string | null;
  path: string;
  depth: number;
  last: boolean;
  opts: Required<JsonViewOptions>;
}

function Lines({ value, name, path, depth, last, opts }: LineProps) {
  const comma = last ? null : <Punct>,</Punct>;
  const label = name === null ? null : <><span className="k">{JSON.stringify(name)}</span><Punct>: </Punct></>;
  const change = opts.changes?.get(path);
  const lineClass = change ? 'cc-changed cc-json-changed' : undefined;

  if (value === null || typeof value !== 'object') {
    return <div className={lineClass}>{label}<Scalar value={value} />{comma}<Note change={change} /></div>;
  }
  const isArray = Array.isArray(value);
  const entries: Array<[string | number, unknown]> = isArray ? value.map((v, i) => [i, v]) : Object.entries(value);
  const [open, close] = isArray ? ['[', ']'] : ['{', '}'];
  if (entries.length === 0) return <div className={lineClass}>{label}<Punct>{open + close}</Punct>{comma}<Note change={change} /></div>;

  const expanded = opts.isOpen(path, depth, entries.length);
  const what = name === null ? 'value' : name;
  const size = entries.length + (isArray ? (entries.length === 1 ? ' item' : ' items') : entries.length === 1 ? ' key' : ' keys');
  return (
    <>
      <div className={lineClass}>
        <button type="button" className="cc-btn-reset cc-tog" aria-expanded={expanded} aria-label={(expanded ? 'Collapse ' : 'Expand ') + what} onClick={() => opts.onToggle(path, !expanded)}>
          <Icon name={expanded ? 'chevron-down' : 'chevron-right'} size={14} />
        </button>
        {label}
        <Punct>{open}</Punct>
        {expanded ? null : <><span className="c">{` ${size} `}</span><Punct>{close}</Punct>{comma}</>}
        <Note change={change} />
      </div>
      {expanded ? (
        <>
          <div className="cc-json-children">
            {entries.map(([k, v], i) => (
              <Lines key={String(k)} value={v} name={isArray ? null : String(k)} path={pointer(path, k)} depth={depth + 1} last={i === entries.length - 1} opts={opts} />
            ))}
          </div>
          <div><Punct>{close}</Punct>{comma}</div>
        </>
      ) : null}
    </>
  );
}
