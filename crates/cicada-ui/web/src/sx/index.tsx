// Halo design system components (the markup and classes of the Halo React bundle in styles/halo.css), plus the
// console's own patterns from the design: problem callouts, status badges, empty states.

import { useEffect, useRef, useState, type ReactNode, type KeyboardEvent, type Ref } from 'react';
import type { ApiError } from '../api/client';
import type { MessageState } from '../api/generated';

/** Joins class names, skipping falsy ones. */
export function cx(...names: Array<string | false | null | undefined>): string {
  return names.filter(Boolean).join(' ');
}

// Halo's icon paths (24px grid, 1.6 stroke, round caps), plus a few from the console design.
const ICONS = {
  search: ['M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14', 'm20 20-3.6-3.6'],
  refresh: ['M3 12a9 9 0 0 1 15.4-6.4L21 8', 'M21 3v5h-5', 'M21 12a9 9 0 0 1-15.4 6.4L3 16', 'M3 21v-5h5'],
  'chevron-down': ['m6 9 6 6 6-6'],
  'chevron-right': ['m9 6 6 6-6 6'],
  'arrow-up': ['M12 19V5', 'm5 12 7-7 7 7'],
  'arrow-right': ['M5 12h14', 'm12 5 7 7-7 7'],
  'x-circle': ['M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18', 'm15 9-6 6', 'm9 9 6 6'],
  alert: ['M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z', 'M12 9v4', 'M12 17h.01'],
  clock: ['M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18', 'M12 7v5l3 2'],
  grid: ['M4 4h6v6H4z', 'M14 4h6v6h-6z', 'M4 14h6v6H4z', 'M14 14h6v6h-6z'],
  tag: ['M3 12V4h8l9 9-8 8-9-9z', 'M7.5 7.5h.01'],
  plus: ['M12 5v14', 'M5 12h14'],
  check: ['m5 12 5 5 9-10'],
  copy: ['M11 9h7a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z', 'M5 15V6a2 2 0 0 1 2-2h9'],
  trash: ['M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3'],
  x: ['M6 6l12 12M18 6L6 18'],
  pause: ['M7 5h2a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z', 'M15 5h2a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1h-2a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z'],
  key: ['M15 3a6 6 0 1 0 0 12a6 6 0 1 0 0-12', 'M10.5 13.5 3 21', 'M6 18l2 2', 'M8 16l2 2'],
  sun: ['M12 8a4 4 0 1 0 0 8a4 4 0 1 0 0-8', 'M12 2v2', 'M12 20v2', 'm4.9 4.9 1.4 1.4', 'm17.7 17.7 1.4 1.4', 'M2 12h2', 'M20 12h2', 'm4.9 19.1 1.4-1.4', 'm17.7 6.3 1.4-1.4'],
  moon: ['M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z'],
  branch: ['M6 3v12', 'M18 9a3 3 0 1 0 0-6a3 3 0 0 0 0 6z', 'M6 21a3 3 0 1 0 0-6a3 3 0 0 0 0 6z', 'M18 9a9 9 0 0 1-9 9'],
  target: ['M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18', 'M12 8a4 4 0 1 0 0 8a4 4 0 1 0 0-8', 'M12 12h.01'],
} as const;

export type IconName = keyof typeof ICONS;

/** An icon; decorative (hidden from screen readers) unless it has a [label]. */
export function Icon({ name, size = 20, label }: { name: IconName; size?: number; label?: string }) {
  return (
    <svg
      className="ha-icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round"
      {...(label ? { role: 'img', 'aria-label': label } : { 'aria-hidden': true })}
    >
      {ICONS[name].map((d) => <path key={d} d={d} />)}
    </svg>
  );
}

type Variant = 'primary' | 'secondary' | 'ghost';
type Size = 'sm' | 'md';

const buttonClass = (variant: Variant, size: Size, extra?: string) => cx('ha-btn', 'ha-btn-' + variant, size !== 'md' && 'ha-btn-' + size, extra);

export interface ButtonProps {
  variant?: Variant;
  size?: Size;
  icon?: IconName;
  onClick?: () => void;
  disabled?: boolean;
  label?: string;
  className?: string;
  describedBy?: string;
  children?: ReactNode;
}

/** Halo.Button. */
export function Button({ variant = 'secondary', size = 'md', icon, onClick, disabled, label, className, describedBy, children }: ButtonProps) {
  return (
    <button type="button" className={buttonClass(variant, size, className)} onClick={onClick} disabled={disabled} aria-label={label} aria-describedby={describedBy}>
      {icon ? <Icon name={icon} size={18} /> : null}
      {children}
    </button>
  );
}

/** A link styled as Halo.Button. */
export function LinkButton({ href, variant = 'secondary', size = 'md', icon, children }: { href: string; variant?: Variant; size?: Size; icon?: IconName; children: ReactNode }) {
  return (
    <a href={href} className={buttonClass(variant, size)}>
      {icon ? <Icon name={icon} size={18} /> : null}
      {children}
    </a>
  );
}

/** Halo.IconButton. */
export function IconButton({ icon, label, variant, size, onClick, buttonRef }: { icon: IconName; label: string; variant?: 'ghost'; size?: 'sm'; onClick?: () => void; buttonRef?: Ref<HTMLButtonElement> }) {
  return (
    <button ref={buttonRef} type="button" aria-label={label} title={label} onClick={onClick} className={cx('ha-iconbtn', variant === 'ghost' && 'ha-iconbtn-ghost', size === 'sm' && 'ha-iconbtn-sm')}>
      <Icon name={icon} size={size === 'sm' ? 18 : 20} />
    </button>
  );
}

export type Tone = 'neutral' | 'info' | 'primary' | 'success' | 'warning' | 'critical';

/** Halo.Badge. */
export function Badge({ tone = 'neutral', dot = false, children }: { tone?: Tone; dot?: boolean; children: ReactNode }) {
  return <span className={cx('ha-badge', 'ha-badge-' + tone, dot && 'ha-badge-dot')}>{children}</span>;
}

/** Message status: always the word, never colour alone. */
export const STATUS: Record<MessageState, { label: string; tone: Tone }> = {
  pending: { label: 'Pending', tone: 'info' },
  running: { label: 'Running', tone: 'primary' },
  done: { label: 'Done', tone: 'success' },
  failed: { label: 'Failed', tone: 'critical' },
  cancelled: { label: 'Cancelled', tone: 'neutral' },
  purged: { label: 'Purged', tone: 'warning' },
};

export function StatusBadge({ status }: { status: MessageState }) {
  const s = STATUS[status];
  return <Badge tone={s.tone} dot>{s.label}</Badge>;
}

/** Halo.Card (nested variant). [flush] removes the body padding, for tables. */
export function Card({ title, subtitle, flush = false, actions, className, children }: { title?: ReactNode; subtitle?: ReactNode; flush?: boolean; actions?: ReactNode; className?: string; children?: ReactNode }) {
  return (
    <section className={cx('ha-card', 'ha-card-nested', className)}>
      {title ? (
        <header className="ha-card-head">
          <h3 className="ha-card-title">
            {title}
            {subtitle ? <p className="ha-card-sub">{subtitle}</p> : null}
          </h3>
          {actions ?? null}
        </header>
      ) : null}
      <div className={cx('ha-card-body', flush && 'ha-flush')}>{children}</div>
    </section>
  );
}

export interface Option<T extends string = string> {
  value: T;
  label: string;
}

/** Halo.Tabs. Variants: filter (pills, primary when selected) and segmented. */
export function Tabs<T extends string>({ items, value, onChange, label, variant = 'filter' }: { items: ReadonlyArray<Option<T>>; value: T; onChange: (value: T) => void; label: string; variant?: 'filter' | 'segmented' }) {
  return (
    <div role="tablist" aria-label={label} className={cx('ha-tabs', 'ha-tabs-' + variant)}>
      {items.map((item) => (
        <button key={item.value} type="button" role="tab" className="ha-tab" aria-selected={item.value === value} onClick={() => onChange(item.value)}>
          {item.label}
        </button>
      ))}
    </div>
  );
}

/** Halo.Select. Options are strings or `{ value, label }`. */
export function Select({ options, value, onChange, label, size = 'sm', id, invalid, describedBy }: { options: ReadonlyArray<string | Option>; value: string; onChange: (value: string) => void; label?: string; size?: Size; id?: string; invalid?: boolean; describedBy?: string }) {
  return (
    <span className={cx('ha-select', size === 'md' && 'ha-select-md')}>
      <select
        aria-label={id ? undefined : label} id={id} value={value} onChange={(e) => onChange(e.target.value)}
        className={invalid ? 'cc-input-error' : undefined} aria-invalid={invalid || undefined} aria-describedby={describedBy}
      >
        {options.map((o) => {
          const v = typeof o === 'string' ? o : o.value;
          return <option key={v} value={v}>{typeof o === 'string' ? o : o.label}</option>;
        })}
      </select>
      <Icon name="chevron-down" size={16} />
    </span>
  );
}

/** Halo.SearchInput (uncontrolled: [onInput] gets every change). */
export function SearchInput({ placeholder, defaultValue = '', onInput, onKeyDown, size, label, shortcut, inputRef }: { placeholder: string; defaultValue?: string; onInput?: (value: string) => void; onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void; size?: 'sm'; label?: string; shortcut?: string; inputRef?: Ref<HTMLInputElement> }) {
  return (
    <label className={cx('ha-search', size === 'sm' && 'ha-search-sm')}>
      <Icon name="search" size={18} />
      <input
        ref={inputRef} type="search" placeholder={placeholder} aria-label={label ?? placeholder} defaultValue={defaultValue}
        spellCheck={false} autoComplete="off" onChange={(e) => onInput?.(e.target.value)} onKeyDown={onKeyDown}
      />
      {shortcut ? <kbd className="ha-kbd">{shortcut}</kbd> : null}
    </label>
  );
}

export interface NavItem {
  key: string;
  icon: IconName;
  label: string;
  href: string;
  count?: string;
}

/** Halo.Sidebar with brand mark and navigation sections. */
export function Sidebar({ brand, sections, active }: { brand: { name: string; subtitle: string; letter: string }; sections: Array<{ title: string; items: NavItem[] }>; active: string }) {
  return (
    <nav className="ha-sidebar cc-sidebar" aria-label="Console">
      <div className="ha-brand">
        <div className="ha-brand-mark" aria-hidden="true">{brand.letter}</div>
        <div>
          <div className="ha-brand-name">{brand.name}</div>
          <div className="ha-brand-sub">{brand.subtitle}</div>
        </div>
      </div>
      {sections.map((s) => (
        <div key={s.title} className="ha-nav-section">
          <div className="ha-nav-title">{s.title}</div>
          {s.items.map((it) => (
            <a key={it.key} href={it.href} className="ha-nav-item" aria-current={it.key === active ? 'page' : undefined}>
              <Icon name={it.icon} size={20} />
              {it.label}
              <span className="ha-nav-count">{it.count ?? ''}</span>
            </a>
          ))}
        </div>
      ))}
    </nav>
  );
}

export type CalloutKind = 'user' | 'temp' | 'ok' | 'info';

const CALLOUT_ICONS: Record<CalloutKind, IconName> = { user: 'x-circle', temp: 'clock', ok: 'check', info: 'alert' };

/** A callout next to the action it is about: user (red), temp (amber, try again), ok (green), info (blue). */
export function Callout({ kind, title, detail, code, action, role }: { kind: CalloutKind; title?: ReactNode; detail?: ReactNode; code?: ReactNode; action?: ReactNode; role?: 'status' | 'alert' }) {
  return (
    <div className={cx('cc-problem', 'cc-problem-' + kind)} role={role ?? (kind === 'ok' || kind === 'info' ? 'status' : 'alert')}>
      <span className="cc-problem-icon"><Icon name={CALLOUT_ICONS[kind]} size={18} /></span>
      <div className="cc-grow">
        {title ? <div className="cc-problem-title">{title}</div> : null}
        {detail ? <div>{detail}</div> : null}
        {code ? <div className="cc-problem-code">{code}</div> : null}
      </div>
      {action ?? null}
    </div>
  );
}

/** A callout for a failed request: temporary failures in amber with [retry], the rest in red. */
export function ProblemCallout({ error, title, retry, action }: { error: unknown; title?: string; retry?: () => void; action?: ReactNode }) {
  const e = error as Partial<ApiError> & { message?: string };
  const temporary = Boolean(e.temporary);
  return (
    <Callout
      kind={temporary ? 'temp' : 'user'}
      title={title ?? (temporary && e.status === 503 ? 'Store unavailable — temporary' : e.title ?? 'Something went wrong')}
      detail={temporary ? `${e.detail ? e.detail + ' ' : ''}Nothing was changed; try again.` : e.detail || e.message}
      code={e.code ? e.code + (e.type ? ' · ' + e.type : '') : null}
      action={action ?? (temporary && retry ? <Button size="sm" icon="refresh" onClick={retry}>Retry Now</Button> : null)}
    />
  );
}

/** A centered empty state inside a card. */
export function EmptyState({ title, caption, badge, mark, action }: { title: ReactNode; caption?: ReactNode; badge?: ReactNode; mark?: ReactNode; action?: ReactNode }) {
  return (
    <div className="cc-empty">
      {mark ?? null}
      {badge ?? null}
      <div className="cc-empty-title">{title}</div>
      {caption ? <div className="cc-caption cc-empty-caption">{caption}</div> : null}
      {action ? <div className="cc-empty-action">{action}</div> : null}
    </div>
  );
}

/** A green check mark for "nothing wrong" empty states. */
export function OkMark() {
  return <span className="cc-ok-mark"><Icon name="check" size={16} /></span>;
}

/** A small button that copies [text] and says so. */
export function CopyButton({ text, label }: { text: string | (() => string); label: string }) {
  const [state, setState] = useState<'idle' | 'ok' | 'failed'>('idle');
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const copy = async () => {
    let ok = true;
    try {
      await navigator.clipboard.writeText(typeof text === 'function' ? text() : text);
    } catch {
      ok = false;
    }
    setState(ok ? 'ok' : 'failed');
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setState('idle'), 1500);
  };
  const current = state === 'ok' ? 'Copied' : state === 'failed' ? 'Copy failed' : label;
  return (
    <button type="button" className="cc-btn-reset cc-copy" aria-label={current} title={current} onClick={copy}>
      <Icon name={state === 'ok' ? 'check' : state === 'failed' ? 'x' : 'copy'} size={16} />
    </button>
  );
}

/** Visually hidden text for screen readers. */
export function SrOnly({ children }: { children: ReactNode }) {
  return <span className="cc-sr-only">{children}</span>;
}
