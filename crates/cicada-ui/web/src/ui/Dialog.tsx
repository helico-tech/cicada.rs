// Dialogs over the page: a centered modal or a drawer on the right. Escape and the backdrop close the topmost one;
// focus stays inside while it is open and returns to where it was afterwards. A page owns its dialogs, so leaving
// the page closes them.

import { useEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { cx, IconButton } from '../halo';

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Open dialogs, topmost last: keys go to the topmost one only. */
const stack: HTMLElement[] = [];

export function Dialog({ kind = 'modal', labelledBy, onClose, children }: { kind?: 'modal' | 'drawer'; labelledBy: string; onClose: () => void; children: ReactNode }) {
  const overlay = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    const el = overlay.current;
    const box = panel.current;
    if (!el || !box) return;
    const previous = document.activeElement;
    stack.push(el);
    document.body.classList.add('cc-noscroll');
    const onKey = (e: KeyboardEvent) => {
      if (stack[stack.length - 1] !== el) return;
      if (e.key === 'Escape') {
        e.preventDefault();
        close.current();
      } else if (e.key === 'Tab') {
        const items = [...box.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((x) => x.offsetParent !== null);
        const first = items[0];
        const last = items[items.length - 1];
        if (!first || !last) return;
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener('keydown', onKey, true);
    (box.querySelector<HTMLElement>('[data-autofocus]') ?? box.querySelector<HTMLElement>('input:not([readonly]), select, textarea') ?? box.querySelector<HTMLElement>(FOCUSABLE))?.focus();
    return () => {
      document.removeEventListener('keydown', onKey, true);
      stack.splice(stack.indexOf(el), 1);
      if (stack.length === 0) document.body.classList.remove('cc-noscroll');
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, []);

  return createPortal(
    <div
      ref={overlay}
      className={cx('cc-overlay', kind === 'drawer' && 'cc-overlay-drawer')}
      onMouseDown={(e) => {
        if (e.target === overlay.current) close.current();
      }}
    >
      <div ref={panel} role="dialog" aria-modal="true" aria-labelledby={labelledBy} className={kind === 'drawer' ? 'cc-drawer' : 'cc-dialog'}>
        {children}
      </div>
    </div>,
    document.body,
  );
}

/** A dialog header with title, caption and a close button. */
export function DialogHeader({ id, title, caption, onClose }: { id: string; title: ReactNode; caption?: ReactNode; onClose: () => void }) {
  return (
    <header className="cc-dialog-head">
      <div className="cc-grow">
        <h2 id={id} className="cc-dialog-title">{title}</h2>
        {caption ? <div className="cc-caption">{caption}</div> : null}
      </div>
      <IconButton icon="x" label="Close" variant="ghost" size="sm" onClick={onClose} />
    </header>
  );
}
