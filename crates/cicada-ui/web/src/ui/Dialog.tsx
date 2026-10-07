// Modal dialogs over the page (the system's scrim and dialog). Escape and the scrim close the topmost one; focus stays
// inside while it is open and returns to where it was afterwards. A page owns its dialogs, so leaving the page closes
// them.

import { useEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { cx, Icon, IconButton, type IconName } from '../sx';

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Open dialogs, topmost last: keys go to the topmost one only. */
const stack: HTMLElement[] = [];

/** [size]: s 400 px (confirmations), m 600 px, l 640 px wide here (the send form). */
export function Dialog({ size = 'm', labelledBy, onClose, children }: { size?: 's' | 'm' | 'l'; labelledBy: string; onClose: () => void; children: ReactNode }) {
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
      className="sx-scrim cc-scrim"
      onMouseDown={(e) => {
        if (e.target === overlay.current) close.current();
      }}
    >
      <div ref={panel} role="dialog" aria-modal="true" aria-labelledby={labelledBy} className={cx('sx-dialog', size === 's' && 'sx-dialog--s', size === 'l' && 'cc-dialog-l')}>
        {children}
      </div>
    </div>,
    document.body,
  );
}

/** A dialog head with title (Poppins), caption, an optional status icon and a close button. */
export function DialogHeader({ id, title, caption, icon, onClose }: { id: string; title: ReactNode; caption?: ReactNode; icon?: { name: IconName; tone: 'danger' | 'warning' | 'info' }; onClose: () => void }) {
  return (
    <header className="sx-dialog-head">
      {icon ? <span className={cx('sx-dialog-icon', 'sx-dialog-icon--' + icon.tone)}><Icon name={icon.name} size={20} /></span> : null}
      <div className="cc-grow">
        <h2 id={id}>{title}</h2>
        {caption ? <p>{caption}</p> : null}
      </div>
      <IconButton icon="close" label="Close" onClick={onClose} />
    </header>
  );
}

/** The scrolling body of a dialog. */
export function DialogBody({ children }: { children: ReactNode }) {
  return <div className="sx-dialog-body cc-dialog-body">{children}</div>;
}

/** The result of the dialog's action, between body and foot so it stays in view. */
export function DialogResult({ children }: { children: ReactNode }) {
  return children ? <div className="cc-dialog-result">{children}</div> : null;
}

/** The dialog's foot: an optional note on the left, the actions on the right. */
export function DialogFoot({ note, children }: { note?: ReactNode; children: ReactNode }) {
  return (
    <footer className="sx-dialog-foot">
      {note ? <span className="caption cc-subtle">{note}</span> : null}
      <span className="sx-spacer" />
      {children}
    </footer>
  );
}
