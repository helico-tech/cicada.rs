// Page-level patterns shared by the screens: page heads with breadcrumbs, property lists, loading and change notes.

import type { ReactNode } from 'react';
import { count } from '../lib/format';
import { cx, Icon } from '../sx';

export interface Crumb {
  label: ReactNode;
  href?: string;
  mono?: boolean;
  /** The full text, as a tooltip, when the label may be cut off. */
  title?: string;
}

/** Crumbs are `{ label, href, mono }`; the last one is the current page. */
export function Breadcrumb({ crumbs }: { crumbs: Crumb[] }) {
  return (
    <nav aria-label="Breadcrumb">
      <ol className="sx-crumbs cc-crumbs">
        {crumbs.map((c, i) => (
          <li key={i}>
            {i > 0 ? <Icon name="chevron-right" /> : null}
            {c.href
              ? <a className={cx(c.mono && 'mono')} href={c.href} title={c.title}>{c.label}</a>
              : <span className={cx(c.mono && 'mono')} title={c.title} aria-current={i === crumbs.length - 1 ? 'page' : undefined}>{c.label}</span>}
          </li>
        ))}
      </ol>
    </nav>
  );
}

/** A page head: optional breadcrumb, title (Poppins, or mono for a URI or id), description, and actions on the right. */
export function PageHead({ crumbs, title, mono = false, titleAfter, description, after, actions }: { crumbs?: Crumb[]; title: ReactNode; mono?: boolean; titleAfter?: ReactNode; description?: ReactNode; after?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="cc-pagehead">
      {crumbs ? <Breadcrumb crumbs={crumbs} /> : null}
      <div className="cc-pagehead-row">
        <div className="cc-pagehead-text">
          <div className="cc-title-row">
            <h1 className={mono ? 'cc-title-mono' : 'title-lg cc-title'}>{title}</h1>
            {titleAfter ?? null}
          </div>
          {description ? <p className="body cc-muted cc-pagehead-desc">{description}</p> : null}
          {after ?? null}
        </div>
        {actions ? <div className="cc-pagehead-actions">{actions}</div> : null}
      </div>
    </div>
  );
}

export type Definition = [term: ReactNode, value: ReactNode, valueClass?: string | null] | null | false;

/** A read-only property list from `[term, value, valueClass]` rows; empty rows are skipped. */
export function Definitions({ rows }: { rows: Definition[] }) {
  return (
    <dl className="sx-props sx-props--readonly cc-props">
      {rows.filter((r): r is Exclude<Definition, null | false> => Boolean(r)).map(([term, value, cls], i) => (
        <div key={i} className="sx-prop">
          <dt className="sx-prop-label"><span>{term}</span></dt>
          <dd className={cx('sx-prop-value', cls)}>{value}</dd>
          <span />
        </div>
      ))}
    </dl>
  );
}

/** A mono span, for URIs, ids and JSON. */
export function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={cx('mono', className)}>{children}</span>;
}

/** A loading placeholder. */
export function Loading({ children }: { children: ReactNode }) {
  return <div className="cc-loading" role="status"><span className="sx-spinner" aria-hidden="true" /><span>{children}</span></div>;
}

/** "Since last refresh: 2 new actors, 1 removed" next to a live table; nothing when nothing changed. */
export function ChangeNote({ added, addedWord, removed = 0, removedWord }: { added: number; addedWord: string; removed?: number; removedWord?: string }) {
  if (!added && !removed) return null;
  return (
    <span className="cc-change-note caption">
      <span className="sx-dot sx-dot--info" aria-hidden="true" />
      <span>
        Since last refresh: {added ? <b>{count(added, addedWord)}</b> : null}
        {removed > 0 && removedWord ? `${added ? ', ' : ''}${removed} ${removedWord}` : null}
        {added ? ' · highlighted for a few seconds' : null}
      </span>
    </span>
  );
}
