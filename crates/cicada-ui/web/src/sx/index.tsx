// The console's components on the Sensus BPM Online design system: each emits the system's markup and `sx-` classes
// (styles/sensus/bundle.css), plus the console's own patterns: problem alerts, status badges, empty states.

import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode, type Ref } from 'react';
import type { ApiError } from '../api/client';
import type { MessageState } from '../api/generated';

/** Joins class names, skipping falsy ones. */
export function cx(...names: Array<string | false | null | undefined>): string {
  return names.filter(Boolean).join(' ');
}

// The system's line icons (bundle.js `ICONS`): a 16 px grid, 1.5 px stroke, round caps and joins. Shapes: a path, a
// circle [cx, cy, r], a rect [x, y, w, h, rx] or a filled dot [cx, cy, r]. Sun, moon and pause are drawn on the same
// grid (sun and moon as in the approved mockups).
type Shape = string | { c: [number, number, number] } | { r: [number, number, number, number, number] } | { d: [number, number, number] };

const ICONS = {
  'chevron-right': ['M6 4l4 4-4 4'],
  'chevron-down': ['M4 6l4 4 4-4'],
  'chevron-left': ['M10 4L6 8l4 4'],
  'arrow-right': ['M2.5 8h11M9.5 4l4 4-4 4'],
  'arrow-up': ['M8 13.5v-11M4 6.5l4-4 4 4'],
  plus: ['M8 3v10M3 8h10'],
  close: ['M4 4l8 8M12 4l-8 8'],
  check: ['M3 8.5l3 3 7-7'],
  search: [{ c: [7, 7, 4.25] }, 'M10.25 10.25l3.25 3.25'],
  filter: ['M2.5 3.5h11l-4.25 5v4.5l-2.5-1.25V8.5z'],
  refresh: ['M13.25 8a5.25 5.25 0 1 1-1.55-3.7', 'M13.5 2.5V5H11'],
  sync: ['M13 6A5.25 5.25 0 0 0 3.4 5M3 10a5.25 5.25 0 0 0 9.6 1', 'M13.25 2.75V6H10M2.75 13.25V10H6'],
  info: [{ c: [8, 8, 6] }, 'M8 7.25v4', { d: [8, 4.9, 0.85] }],
  warning: ['M8 2.25l6.25 11H1.75z', 'M8 6.5v3', { d: [8, 11.25, 0.85] }],
  error: ['M5.5 2h5l3.5 3.5v5L10.5 14h-5L2 10.5v-5z', 'M6 6l4 4M10 6l-4 4'],
  success: [{ c: [8, 8, 6] }, 'M5.25 8.25l1.9 1.9 3.6-3.9'],
  clock: [{ c: [8, 8, 6] }, 'M8 4.75V8l2.25 1.5'],
  hourglass: ['M4 2.5h8M4 13.5h8', 'M5 2.5c0 3 6 3 6 5.5s-6 2.5-6 5.5M11 2.5c0 3-6 3-6 5.5s6 2.5 6 5.5'],
  users: [{ c: [6, 5.75, 2.25] }, 'M1.75 13.25a4.25 4.25 0 0 1 8.5 0M10.5 3.6a2.25 2.25 0 0 1 0 4.3M12 9.5a4.25 4.25 0 0 1 2.25 3.75'],
  template: [{ r: [2.5, 2.5, 11, 11, 1] }, 'M2.5 6h11M6.5 6v7.5'],
  copy: [{ r: [5.5, 5.5, 8, 8, 1] }, 'M10.5 5.5v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2'],
  trash: ['M2.75 4.25h10.5M6.25 4.25v-1.5h3.5v1.5M4.25 4.25l.6 9h6.3l.6-9', 'M6.75 6.75v4M9.25 6.75v4'],
  key: [{ c: [5, 10.5, 2.75] }, 'M7 8.5L12.5 3M10.5 5L12 6.5M12 3.5L13.5 5'],
  send: ['M2.5 8L13.5 2.5 10.5 13.5 7.5 9zM7.5 9l6-6.5'],
  list: ['M5.5 4h8M5.5 8h8M5.5 12h8', { d: [2.75, 4, 0.8] }, { d: [2.75, 8, 0.8] }, { d: [2.75, 12, 0.8] }],
  'flow-v': [{ r: [5.75, 1.75, 4.5, 4.5, 1] }, { r: [5.75, 9.75, 4.5, 4.5, 1] }, 'M8 6.25v3.5'],
  play: ['M5 3.25v9.5L12.5 8z'],
  pause: ['M5.75 3.5v9M10.25 3.5v9'],
  sun: [{ c: [8, 8, 3] }, 'M8 1.75v1.5M8 12.75v1.5M1.75 8h1.5M12.75 8h1.5M3.6 3.6l1.05 1.05M11.35 11.35l1.05 1.05M3.6 12.4l1.05-1.05M11.35 4.65l1.05-1.05'],
  moon: ['M13.25 9.5A5.5 5.5 0 0 1 6.5 2.75a5.5 5.5 0 1 0 6.75 6.75z'],
} satisfies Record<string, Shape[]>;

export type IconName = keyof typeof ICONS;

function ShapeElement({ shape }: { shape: Shape }) {
  if (typeof shape === 'string') return <path d={shape} />;
  if ('c' in shape) return <circle cx={shape.c[0]} cy={shape.c[1]} r={shape.c[2]} />;
  if ('r' in shape) return <rect x={shape.r[0]} y={shape.r[1]} width={shape.r[2]} height={shape.r[3]} rx={shape.r[4]} />;
  return <circle cx={shape.d[0]} cy={shape.d[1]} r={shape.d[2]} fill="currentColor" stroke="none" />;
}

/** A line icon; decorative (hidden from screen readers) unless it has a [label]. */
export function Icon({ name, size = 16, label }: { name: IconName; size?: number; label?: string }) {
  const shapes: readonly Shape[] = ICONS[name];
  return (
    <svg
      className="sx-icon" width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor"
      strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" focusable="false"
      {...(label ? { role: 'img', 'aria-label': label } : { 'aria-hidden': true })}
    >
      {shapes.map((s, i) => <ShapeElement key={i} shape={s} />)}
    </svg>
  );
}

export type Variant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'danger-ghost';
export type Size = 'xs' | 'sm' | 'md';

const buttonClass = (variant: Variant, size: Size, extra?: string | false) =>
  cx('sx-btn', variant !== 'secondary' && 'sx-btn--' + variant, size !== 'md' && 'sx-btn--' + size, extra);

export interface ButtonProps {
  variant?: Variant;
  size?: Size;
  icon?: IconName;
  onClick?: () => void;
  disabled?: boolean;
  label?: string;
  title?: string;
  className?: string;
  describedBy?: string;
  children?: ReactNode;
}

/** A button: a verb plus an object, sentence case; an ellipsis when a dialog with more input follows. */
export function Button({ variant = 'secondary', size = 'md', icon, onClick, disabled, label, title, className, describedBy, children }: ButtonProps) {
  return (
    <button type="button" className={buttonClass(variant, size, className)} onClick={onClick} disabled={disabled} aria-label={label} title={title} aria-describedby={describedBy}>
      {icon ? <Icon name={icon} /> : null}
      {children !== undefined && children !== null ? <span>{children}</span> : null}
    </button>
  );
}

/** A link styled as a button. */
export function LinkButton({ href, variant = 'secondary', size = 'md', icon, label, children }: { href: string; variant?: Variant; size?: Size; icon?: IconName; label?: string; children?: ReactNode }) {
  return (
    <a href={href} className={cx(buttonClass(variant, size, 'cc-linkbtn'), (children === undefined || children === null) && 'sx-btn--icon')} aria-label={label} title={label}>
      {icon ? <Icon name={icon} /> : null}
      {children !== undefined && children !== null ? <span>{children}</span> : null}
    </a>
  );
}

/** A square ghost button with an icon only; [label] names it for screen readers and as its tooltip. */
export function IconButton({ icon, label, size = 'md', onClick, buttonRef }: { icon: IconName; label: string; size?: Size; onClick?: () => void; buttonRef?: Ref<HTMLButtonElement> }) {
  return (
    <button ref={buttonRef} type="button" aria-label={label} title={label} onClick={onClick} className={cx(buttonClass('ghost', size), 'sx-btn--icon')}>
      <Icon name={icon} />
    </button>
  );
}

export type Tone = 'neutral' | 'info' | 'accent' | 'success' | 'warning' | 'danger' | 'outline';

/** A badge: a word, with an icon where the state needs a shape too. */
export function Badge({ tone = 'neutral', size = 'md', solid = false, icon, children }: { tone?: Tone; size?: 'sm' | 'md'; solid?: boolean; icon?: IconName; children: ReactNode }) {
  return (
    <span className={cx('sx-badge', tone !== 'neutral' && 'sx-badge--' + tone, size === 'sm' && 'sx-badge--sm', solid && 'sx-badge--solid')}>
      {icon ? <Icon name={icon} /> : null}
      <span>{children}</span>
    </span>
  );
}

/** A count pill (navigation, widget heads). */
export function Count({ tone, children }: { tone?: 'danger' | 'warning' | 'accent'; children: ReactNode }) {
  return <span className={cx('sx-count sx-num', tone && 'sx-count--' + tone)}>{children}</span>;
}

/** Message status: always the word, and a shape in the regular size; never colour alone. */
export const STATUS: Record<MessageState, { label: string; tone: Tone; icon: IconName }> = {
  pending: { label: 'Pending', tone: 'neutral', icon: 'clock' },
  running: { label: 'Running', tone: 'info', icon: 'sync' },
  done: { label: 'Done', tone: 'success', icon: 'success' },
  failed: { label: 'Failed', tone: 'danger', icon: 'error' },
  cancelled: { label: 'Cancelled', tone: 'outline', icon: 'close' },
  purged: { label: 'Purged', tone: 'warning', icon: 'warning' },
};

export function StatusBadge({ status, size = 'md' }: { status: MessageState; size?: 'sm' | 'md' }) {
  const s = STATUS[status];
  return <Badge tone={s.tone} size={size} icon={size === 'md' ? s.icon : undefined}>{s.label}</Badge>;
}

/** A failure count: a plain 0, or a warning badge with its triangle. [suffix] follows the number ("2 retried"). */
export function Failures({ n, suffix }: { n: number; suffix?: string }) {
  if (n <= 0) return <span className="sx-num">0</span>;
  return <Badge tone="warning" size="sm" icon="warning">{String(n) + (suffix ? ' ' + suffix : '')}</Badge>;
}

/** A widget: a titled panel. [flush] drops the body padding (tables, lists); [sunken] is for code and JSON. */
export function Widget({ title, count, subtitle, flush = false, sunken = false, actions, className, children }: { title: ReactNode; count?: ReactNode; subtitle?: ReactNode; flush?: boolean; sunken?: boolean; actions?: ReactNode; className?: string; children?: ReactNode }) {
  return (
    <section className={cx('sx-widget', className)}>
      <header className="sx-widget-head">
        <h3>{title}</h3>
        {count ?? null}
        {subtitle ? <span className="caption cc-subtle cc-widget-sub">{subtitle}</span> : null}
        {actions ?? null}
      </header>
      <div className={cx('sx-widget-body', flush && 'sx-widget-body--flush', sunken && 'cc-sunken')}>{children}</div>
    </section>
  );
}

export interface Option<T extends string = string> {
  value: T;
  label: string;
}

/** A segmented control: toggle buttons, one pressed. */
export function Segmented<T extends string>({ items, value, onChange, label, icons }: { items: ReadonlyArray<Option<T>>; value: T; onChange: (value: T) => void; label: string; icons?: Partial<Record<T, IconName>> }) {
  return (
    <div className="sx-segmented" role="group" aria-label={label}>
      {items.map((item) => {
        const icon = icons?.[item.value];
        return (
          <button key={item.value} type="button" aria-pressed={item.value === value} onClick={() => onChange(item.value)}>
            {icon ? <Icon name={icon} /> : null}
            <span>{item.label}</span>
          </button>
        );
      })}
    </div>
  );
}

/** A native select in the system's look. Options are strings or `{ value, label }`. */
export function Select({ options, value, onChange, label, size = 'md', id, invalid, describedBy, mono = false }: { options: ReadonlyArray<string | Option>; value: string; onChange: (value: string) => void; label?: string; size?: 'sm' | 'md'; id?: string; invalid?: boolean; describedBy?: string; mono?: boolean }) {
  return (
    <span className="cc-select">
      <select
        aria-label={id ? undefined : label} id={id} value={value} onChange={(e) => onChange(e.target.value)}
        className={cx('sx-select', size === 'sm' && 'sx-select--sm', invalid && 'is-invalid', mono && 'sx-mono')}
        aria-invalid={invalid || undefined} aria-describedby={describedBy}
      >
        {options.map((o) => {
          const v = typeof o === 'string' ? o : o.value;
          return <option key={v} value={v}>{typeof o === 'string' ? o : o.label}</option>;
        })}
      </select>
      <Icon name="chevron-down" />
    </span>
  );
}

/** A text box with a leading icon (uncontrolled: [onInput] gets every change). */
export function SearchInput({ placeholder, defaultValue = '', onInput, onKeyDown, label, id, shortcut, inputRef, icon = 'search', mono = false }: { placeholder: string; defaultValue?: string; onInput?: (value: string) => void; onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void; label?: string; id?: string; shortcut?: string; inputRef?: Ref<HTMLInputElement>; icon?: IconName; mono?: boolean }) {
  return (
    <div className={cx('sx-inputgroup', mono && 'is-mono')}>
      <Icon name={icon} />
      <input
        ref={inputRef} id={id} type="search" placeholder={placeholder} aria-label={id ? undefined : label ?? placeholder} defaultValue={defaultValue}
        spellCheck={false} autoComplete="off" onChange={(e) => onInput?.(e.target.value)} onKeyDown={onKeyDown}
      />
      {shortcut ? <kbd className="sx-kbd cc-kbd">{shortcut}</kbd> : null}
    </div>
  );
}

/** A form field: label, control, help text and an error with its icon. */
export function Field({ id, label, required, hint, error, errorId, className, children }: { id?: string; label: ReactNode; required?: boolean; hint?: ReactNode; error?: ReactNode; errorId?: string; className?: string; children: ReactNode }) {
  const text = <><span>{label}</span>{required ? <span className="cc-required" aria-hidden="true">*</span> : null}</>;
  return (
    <div className={cx('sx-field', className)}>
      {id ? <label className="sx-label" htmlFor={id}>{text}</label> : <span className="sx-label">{text}</span>}
      {children}
      {hint ? <span className="sx-help">{hint}</span> : null}
      {error ? <FieldError id={errorId}>{error}</FieldError> : null}
    </div>
  );
}

/** A field's error: the octagon and the words, in the danger text colour. */
export function FieldError({ id, children }: { id?: string; children: ReactNode }) {
  return <span className="cc-field-error" id={id}><Icon name="error" size={14} /><span>{children}</span></span>;
}

export type CalloutKind = 'user' | 'temp' | 'ok' | 'info';

const CALLOUT: Record<CalloutKind, { tone: string; icon: IconName }> = {
  user: { tone: 'danger', icon: 'error' },
  temp: { tone: 'warning', icon: 'warning' },
  ok: { tone: 'success', icon: 'success' },
  info: { tone: 'info', icon: 'info' },
};

/** An alert next to the action it is about: user (error, octagon), temp (warning, try again), ok, info. */
export function Callout({ kind, title, detail, code, action, role }: { kind: CalloutKind; title?: ReactNode; detail?: ReactNode; code?: ReactNode; action?: ReactNode; role?: 'status' | 'alert' }) {
  const { tone, icon } = CALLOUT[kind];
  return (
    <div className={cx('sx-alert', 'sx-alert--' + tone)} role={role ?? (kind === 'ok' || kind === 'info' ? 'status' : 'alert')}>
      <span className={'sx-tone-' + tone}><Icon name={icon} size={20} /></span>
      <div className="cc-alert-text">
        {title ? <div className="sx-alert-title">{title}</div> : null}
        {detail ? <p>{detail}</p> : null}
        {code ? <p className="mono-sm">{code}</p> : null}
      </div>
      {action ?? <span />}
    </div>
  );
}

/** An alert for a failed request: temporary failures as a warning with [retry], the rest as an error. */
export function ProblemCallout({ error, title, retry, action }: { error: unknown; title?: string; retry?: () => void; action?: ReactNode }) {
  const e = error as Partial<ApiError> & { message?: string };
  const temporary = Boolean(e.temporary);
  return (
    <Callout
      kind={temporary ? 'temp' : 'user'}
      title={title ?? (temporary && e.status === 503 ? 'Store unavailable — temporary' : e.title ?? 'Something went wrong')}
      detail={temporary ? `${e.detail ? e.detail + ' ' : ''}Nothing was changed; try again.` : e.detail || e.message}
      code={e.code ? e.code + (e.type ? ' · ' + e.type : '') : null}
      action={action ?? (temporary && retry ? <Button size="sm" icon="refresh" onClick={retry}>Retry now</Button> : null)}
    />
  );
}

/** An empty state inside a widget. */
export function EmptyState({ title, caption, badge, mark, action }: { title: ReactNode; caption?: ReactNode; badge?: ReactNode; mark?: ReactNode; action?: ReactNode }) {
  return (
    <div className="sx-empty sx-empty--compact cc-empty">
      {mark ?? null}
      {badge ?? null}
      <div className="sx-empty-title">{title}</div>
      {caption ? <p>{caption}</p> : null}
      {action ? <div className="cc-empty-action">{action}</div> : null}
    </div>
  );
}

/** The success circle for "nothing wrong" empty states. */
export function OkMark() {
  return <span className="sx-empty-icon sx-tone-success"><Icon name="success" size={20} /></span>;
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
  return <IconButton icon={state === 'ok' ? 'check' : state === 'failed' ? 'close' : 'copy'} label={current} size="sm" onClick={() => void copy()} />;
}

/** Visually hidden text for screen readers. */
export function SrOnly({ children }: { children: ReactNode }) {
  return <span className="sx-sr-only">{children}</span>;
}
