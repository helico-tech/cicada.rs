// The top bar: open an actor or message by URI or id, the node's health, the live/updating/paused state and the theme.

import type { Ref } from 'react';
import { clock, relative } from '../lib/format';
import { POLL_INTERVAL_MS, type Poller } from '../lib/poller';
import { Badge, Button, Icon, IconButton, SearchInput } from '../halo';

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
    <div className="cc-topbar">
      <div className="cc-topbar-row">
        <div className="cc-topbar-search">
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
        <div className="cc-grow" />
        <div className="cc-health">
          {!health ? (
            <Badge tone="neutral" dot>Checking…</Badge>
          ) : (
            <>
              {health.up ? <Badge tone="success" dot>Up</Badge> : <Badge tone="critical" dot>Unhealthy</Badge>}
              <div className="cc-stack">
                <span className="mono cc-node">{health.node ?? 'unknown node'}</span>
                <span className="cc-caption" title={health.detail ?? ''}>{health.up ? 'Lifecycle: ' + health.state : health.detail || 'Stopping, or its store is unreachable'}</span>
              </div>
            </>
          )}
        </div>
        <div className="cc-divider" aria-hidden="true" />
        {/* Not a live region: it re-renders every second for "2 s ago". */}
        <div className="cc-live-status">
          {busy ? (
            <>
              <span className="cc-spin" aria-hidden="true" />
              <div className="cc-stack">
                <span className="cc-status-title">Updating…</span>
                <span className="cc-caption ha-num">{last ? 'Last ' + last : 'First load'}</span>
              </div>
            </>
          ) : paused ? (
            <>
              <span className="cc-paused-icon"><Icon name="pause" size={16} /></span>
              <div className="cc-stack">
                <span className="cc-status-title cc-warning">Paused</span>
                <span className="cc-caption ha-num">{last ? `Data from ${last} · ${relative(lastAt)}` : 'No data yet'}</span>
              </div>
            </>
          ) : (
            <>
              <span className="cc-pulse" aria-hidden="true" />
              <div className="cc-stack">
                <span className="cc-status-title">{last ? 'Refreshed ' + last : 'Loading…'}</span>
                <span className="cc-caption ha-num">{`${last ? relative(lastAt) + ' · ' : ''}every ${POLL_INTERVAL_MS / 1000}s`}</span>
              </div>
            </>
          )}
        </div>
        <button type="button" className="cc-switch" aria-pressed={!paused} onClick={() => poller.setPaused(!paused)}>
          <span className="cc-switch-track"><span className="cc-switch-knob" /></span>
          <span>Live</span>
        </button>
        <IconButton icon="refresh" label="Refresh now" size="sm" onClick={() => void poller.refresh()} />
        <div className="cc-theme">
          <IconButton icon={dark ? 'sun' : 'moon'} label={dark ? 'Switch to light mode' : 'Switch to dark mode'} size="sm" onClick={onToggleTheme} />
        </div>
      </div>
      {paused ? (
        <div className="cc-paused-banner">
          <span className="cc-grow">
            <b>Live updates paused. </b>
            {last ? `The data below is from ${last} and may be stale. ` : ''}
            Highlights for changes resume with the next refresh.
          </span>
          <Button size="sm" onClick={() => poller.setPaused(false)}>Resume Live</Button>
        </div>
      ) : null}
      {busy ? <div className="cc-progress" role="progressbar" aria-label="Refreshing data" /> : null}
    </div>
  );
}
