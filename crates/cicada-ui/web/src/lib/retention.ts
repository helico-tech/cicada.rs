// The persistence (retention) policy of a send, as the send drawer edits it and the message page shows it.

import type { Keep, Retention } from '../api/generated';
import { isoDurationText } from './format';

export type KeepKind = 'default' | 'remove' | 'for' | 'last';

/** One rule as the form holds it: a kind, and the text of its age or count. */
export interface KeepDraft {
  kind: KeepKind;
  age: string;
  count: string;
}

export const blankKeep = (): KeepDraft => ({ kind: 'default', age: '30d', count: '100' });

export const MAX_COUNT = 10_000;

/** A form's age as the API takes it: Kotlin's `30d` / `2h 30m` form or ISO-8601 (`P30D`) pass through as typed. */
function readKeep(draft: KeepDraft, outcome: string): { keep: Keep | null } | { error: string } {
  switch (draft.kind) {
    case 'default':
      return { keep: null };
    case 'remove':
      return { keep: { kind: 'remove' } };
    case 'for': {
      const age = draft.age.trim();
      if (!age) return { error: `Give how long to keep a ${outcome} message, e.g. 30d or PT12H.` };
      return { keep: { kind: 'for', age } };
    }
    case 'last': {
      const n = Number(draft.count);
      if (!Number.isInteger(n) || n < 1 || n > MAX_COUNT) return { error: `Keep last must be a whole number from 1 to ${MAX_COUNT.toLocaleString('en')}.` };
      return { keep: { kind: 'last', count: n } };
    }
  }
}

/** The policy to send, or undefined when both rules are the default; an error names what to fix. */
export function readRetention(done: KeepDraft, failed: KeepDraft): { retention: Retention | undefined } | { error: string } {
  const d = readKeep(done, 'done');
  if ('error' in d) return d;
  const f = readKeep(failed, 'failed');
  if ('error' in f) return f;
  if (!d.keep && !f.keep) return { retention: undefined };
  return { retention: { ...(d.keep ? { done: d.keep } : {}), ...(f.keep ? { failed: f.keep } : {}) } };
}

/** A retention rule in words. */
export function keepText(keep: Keep | null | undefined): string {
  if (!keep) return 'Store default (its retention period, 7 days unless configured)';
  switch (keep.kind) {
    case 'remove':
      return 'Removed as soon as it finishes';
    case 'for':
      return `Kept for ${isoDurationText(keep.age)} after it finished`;
    case 'last':
      return `Kept while among the last ${keep.count} of this actor`;
  }
}
