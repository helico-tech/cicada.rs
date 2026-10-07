// Formatting of times, durations and sizes. Times are shown in the browser's time zone.

const pad = (n: number): string => String(n).padStart(2, '0');

export type TimeLike = Date | string | number;

function toDate(value: TimeLike | null | undefined): Date | null {
  if (value === null || value === undefined) return null;
  const d = value instanceof Date ? value : new Date(value);
  return Number.isNaN(d.getTime()) ? null : d;
}

/** `14:02:10` */
export function clock(value: TimeLike | null | undefined): string {
  const d = toDate(value);
  return d ? `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}` : '—';
}

/** `03/10/2026 • 14:02:10` */
export function dateTime(value: TimeLike | null | undefined): string {
  const d = toDate(value);
  return d ? `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${d.getFullYear()} • ${clock(d)}` : '—';
}

/** `03/10 • 14:02:10` */
export function shortDateTime(value: TimeLike | null | undefined): string {
  const d = toDate(value);
  return d ? `${pad(d.getDate())}/${pad(d.getMonth() + 1)} • ${clock(d)}` : '—';
}

/** A length of time in words: `14 s`, `5 min`, `3 h 57 min`, `2 d 3 h`. */
export function duration(ms: number): string {
  const s = Math.round(Math.abs(ms) / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
  const d = Math.floor(h / 24);
  return h % 24 ? `${d} d ${h % 24} h` : `${d} d`;
}

/** `in 5 min`, `14 s ago`, `just now`. */
export function relative(value: TimeLike | null | undefined, now: number = Date.now()): string {
  const d = toDate(value);
  if (!d) return '';
  const diff = d.getTime() - now;
  if (Math.abs(diff) < 1000) return 'just now';
  return diff > 0 ? `in ${duration(diff)}` : `${duration(diff)} ago`;
}

/** An ISO-8601 duration (`PT1M30S`, `P1DT2H`) in milliseconds, or null when it is not one. */
export function parseIsoDuration(text: string | null | undefined): number | null {
  const m = /^(-)?P(?:(\d+(?:\.\d+)?)D)?(?:T(?:(\d+(?:\.\d+)?)H)?(?:(\d+(?:\.\d+)?)M)?(?:(\d+(?:\.\d+)?)S)?)?$/.exec(text ?? '');
  if (!m || text === 'P' || text?.endsWith('T')) return null;
  const [, sign, d, h, min, s] = m;
  const ms = ((Number(d) || 0) * 86400 + (Number(h) || 0) * 3600 + (Number(min) || 0) * 60 + (Number(s) || 0)) * 1000;
  return sign ? -ms : ms;
}

/** A duration for a caption: `1m 30s`, `2s`, `500ms`. */
export function compactDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const units: Array<[string, number]> = [['d', 86400000], ['h', 3600000], ['m', 60000], ['s', 1000]];
  let rest = ms;
  const out: string[] = [];
  for (const [unit, size] of units) {
    const n = Math.floor(rest / size);
    if (n) out.push(n + unit);
    rest -= n * size;
  }
  if (rest) out.push(rest + 'ms');
  return out.join(' ');
}

/**
 * A wire duration (an ISO-8601 string, or milliseconds) as `compactDuration`, or the text itself when it cannot be
 * read.
 */
export function isoDurationText(value: string | number): string {
  if (typeof value === 'number') return compactDuration(value);
  const ms = parseIsoDuration(value);
  return ms === null ? String(value) : compactDuration(ms);
}

export function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} bytes`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** `3 messages`, `1 message`. */
export function count(n: number, word: string, plural: string = word + 's'): string {
  return `${n} ${n === 1 ? word : plural}`;
}

/** `RUNNING` → `Running`. */
export function capitalize(text: string | null | undefined): string {
  const s = String(text ?? '').toLowerCase();
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** The value of a `datetime-local` input for a date, in local time: `2026-10-03T14:02`. */
export function localInputValue(value: TimeLike): string {
  const d = toDate(value);
  return d ? `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}` : '';
}

/** The browser's time zone, for captions such as "Europe/Amsterdam". */
export function timeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || '';
  } catch {
    return '';
  }
}
