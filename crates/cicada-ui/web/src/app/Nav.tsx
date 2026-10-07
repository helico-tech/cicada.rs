// The left navigation: the console's sections as tree rows, with counts where the console knows them.

import { cx, Count, Icon, type IconName } from '../sx';

export interface NavItem {
  key: string;
  icon: IconName;
  label: string;
  href: string;
  count?: string;
  /** Danger: the icon and count in the danger colours (failures). */
  danger?: boolean;
}

export function Nav({ items, active }: { items: NavItem[]; active: string }) {
  return (
    <aside className="sx-sidebar cc-sidebar" aria-label="Sections">
      <nav className="sx-tree cc-nav" aria-label="Console">
        {items.map((it) => (
          <a key={it.key} href={it.href} className="sx-tree-row cc-nav-row" aria-current={it.key === active ? 'page' : undefined}>
            <span className={cx('sx-tree-icon', it.danger && 'sx-tone-danger')}><Icon name={it.icon} /></span>
            <span className="sx-tree-label label">{it.label}</span>
            {it.count ? <Count tone={it.danger ? 'danger' : undefined}>{it.count}</Count> : null}
          </a>
        ))}
      </nav>
    </aside>
  );
}
