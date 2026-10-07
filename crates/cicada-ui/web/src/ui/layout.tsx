// Page-level patterns shared by the screens: page heads with breadcrumbs and definition lists.

import { Fragment, type ReactNode } from 'react';
import { count } from '../lib/format';
import { cx, Icon } from '../halo';

export interface Crumb {
  label: ReactNode;
  href?: string;
  mono?: boolean;
}

/** Crumbs are `{ label, href, mono }`; the last one is the current page. */
export function Breadcrumb({ crumbs }: { crumbs: Crumb[] }) {
  return (
    <nav className="cc-breadcrumb" aria-label="Breadcrumb">
      {crumbs.map((c, i) => (
        <Fragment key={i}>
          {i > 0 ? <Icon name="chevron-right" size={14} /> : null}
          {c.href ? <a className={cx('cc-link', c.mono && 'mono')} href={c.href}>{c.label}</a> : <span className={c.mono ? 'mono' : undefined}>{c.label}</span>}
        </Fragment>
      ))}
    </nav>
  );
}

/** A page head: optional breadcrumb, title, description, and actions on the right. */
export function PageHead({ crumbs, title, titleClass, titleAfter, description, after, actions }: { crumbs?: Crumb[]; title: ReactNode; titleClass?: string; titleAfter?: ReactNode; description?: ReactNode; after?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="cc-pagehead">
      {crumbs ? <Breadcrumb crumbs={crumbs} /> : null}
      <div className="cc-pagehead-row">
        <div className="cc-pagehead-text">
          <h1 className={cx('ha-pagehead-title', titleClass)}>
            {title}
            {titleAfter}
          </h1>
          {description ? <p className="cc-pagehead-desc">{description}</p> : null}
          {after ?? null}
        </div>
        {actions ? <div className="cc-pagehead-actions">{actions}</div> : null}
      </div>
    </div>
  );
}

export type Definition = [term: ReactNode, value: ReactNode, valueClass?: string | null] | null | false;

/** A definition list from `[term, value, valueClass]` rows; empty rows are skipped. */
export function Definitions({ rows }: { rows: Definition[] }) {
  return (
    <dl className="cc-dl">
      {rows.filter((r): r is Exclude<Definition, null | false> => Boolean(r)).map(([term, value, cls], i) => (
        <Fragment key={i}>
          <dt>{term}</dt>
          <dd className={cls ?? undefined}>{value}</dd>
        </Fragment>
      ))}
    </dl>
  );
}

/** A mono span. */
export function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={cx('mono', className)}>{children}</span>;
}

/** A loading placeholder. */
export function Loading({ children }: { children: ReactNode }) {
  return <div className="cc-loading">{children}</div>;
}

/** "Since last refresh: 2 new actors, 1 removed" above a live table; nothing when nothing changed. */
export function ChangeNote({ added, addedWord, removed = 0, removedWord }: { added: number; addedWord: string; removed?: number; removedWord?: string }) {
  if (!added && !removed) return null;
  return (
    <div className="cc-change-note">
      <span className="cc-dot" aria-hidden="true" />
      <span className="cc-caption">
        Since last refresh: {added ? <b>{count(added, addedWord)}</b> : null}
        {removed > 0 && removedWord ? `${added ? ', ' : ''}${removed} ${removedWord}` : null}
        {added ? ' · highlighted for a few seconds' : null}
      </span>
    </div>
  );
}
