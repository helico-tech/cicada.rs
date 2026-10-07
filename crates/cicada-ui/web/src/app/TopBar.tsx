// The app bar: the console's name, open an actor or message by URI or id, the node's health, the live/updating/paused
// state, the Live switch, refresh and the theme. While paused, an alert bar under it says the data may be stale.

import type { Ref } from 'react';
import { clock, relative } from '../lib/format';
import { POLL_INTERVAL_MS, type Poller } from '../lib/poller';
import { href } from '../lib/routes';
import { Badge, Button, Icon, IconButton, SearchInput } from '../sx';

export interface Health {
  up: boolean;
  node?: string;
  state?: string;
  detail?: string;
}

export function TopBar({ poller, health, dark, onToggleTheme, onSearch, searchRef }: { poller: Poller; health: Health | null; dark: boolean; onToggleTheme: () => void; onSearch: (text: string) => void; searchRef: Ref<HTMLInputElement> }) {
  const { paused, busy, lastAt } = poller;
  const last = lastAt ? clock(lastAt) : null;
  return (
    <>
      <header className="sx-appbar cc-appbar">
        <a className="sx-appbar-logo cc-logo" href={href.actors()} aria-label="Cicada actor console, start">
          <span className="cc-logo-name">Cicada</span>
          <span className="caption cc-muted">Actor console</span>
        </a>
        <span className="sx-vr cc-vr" aria-hidden="true" />
        <div className="sx-globalsearch cc-search">
          <SearchInput
            placeholder="Paste an actor URI or message id" label="Open actor or message" shortcut="⌘K" inputRef={searchRef}
            onKeyDown={(e) => {
              if (e.key !== 'Enter') return;
              const text = e.currentTarget.value.trim();
              if (text) {
                onSearch(text);
                e.currentTarget.value = '';
              }
            }}
          />
        </div>
        <span className="sx-spacer" />
        <div className="cc-appbar-group cc-health">
          {!health ? (
            <Badge icon="clock">Checking…</Badge>
          ) : (
            <>
              {health.up ? <Badge tone="success" icon="success">Up</Badge> : <Badge tone="danger" icon="error">Unhealthy</Badge>}
              <div className="cc-stack">
                <span className="mono-sm">{health.node ?? 'unknown node'}</span>
                <span className="caption cc-muted cc-health-detail" title={health.detail ?? ''}>{health.up ? 'Lifecycle: ' + health.state : health.detail || 'Stopping, or its store is unreachable'}</span>
              </div>
            </>
          )}
        </div>
        <span className="sx-vr cc-vr" aria-hidden="true" />
        {/* Not a live region: it re-renders every second for "2 s ago". */}
        <div className="cc-appbar-group cc-live-status">
          {busy ? (
            <>
              <span className="sx-spinner" aria-hidden="true" />
              <div className="cc-stack">
                <span className="label">Updating…</span>
                <span className="caption sx-num cc-muted">{last ? 'Last ' + last : 'First load'}</span>
              </div>
            </>
          ) : paused ? (
            <>
              <span className="sx-tone-warning"><Icon name="pause" /></span>
              <div className="cc-stack">
                <span className="label cc-warning">Paused</span>
                <span className="caption sx-num cc-muted">{last ? `Data from ${last} · ${relative(lastAt)}` : 'No data yet'}</span>
              </div>
            </>
          ) : (
            <>
              <span className="sx-dot sx-dot--success" aria-hidden="true" />
              <div className="cc-stack">
                <span className="label">{last ? 'Refreshed ' + last : 'Loading…'}</span>
                <span className="caption sx-num cc-muted">{`${last ? relative(lastAt) + ' · ' : ''}every ${POLL_INTERVAL_MS / 1000} s`}</span>
              </div>
            </>
          )}
        </div>
        <button type="button" className="sx-switch cc-live-switch" role="switch" aria-checked={!paused} onClick={() => poller.setPaused(!paused)}>
          <span className="sx-switch-track" />
          <span className="sx-switch-state">Live</span>
        </button>
        <div className="sx-appbar-end">
          <IconButton icon="refresh" label="Refresh now" onClick={() => void poller.refresh()} />
          <IconButton icon={dark ? 'sun' : 'moon'} label={dark ? 'Switch to light mode' : 'Switch to dark mode'} onClick={onToggleTheme} />
        </div>
        {busy ? <div className="sx-progress is-indeterminate cc-progress" role="progressbar" aria-label="Refreshing data"><i /></div> : null}
      </header>
      {paused ? (
        <div className="sx-alertbar sx-alertbar--warning" role="status">
          <span className="sx-tone-warning"><Icon name="warning" /></span>
          <span className="sx-alertbar-text">
            <b>Live updates paused. </b>
            {last ? `The data below is from ${last} and may be stale. ` : ''}
            Highlights for changes resume with the next refresh.
          </span>
          <Button size="sm" icon="play" onClick={() => poller.setPaused(false)}>Resume live</Button>
        </div>
      ) : null}
    </>
  );
}
