import { test } from 'vitest';
import assert from 'node:assert/strict';
import {
  capitalize, compactDuration, count, dateTime, duration, isoDurationText, localInputValue, parseIsoDuration, relative,
} from '../src/lib/format';

test('durations read as words at a useful precision', () => {
  assert.equal(duration(14_200), '14 s');
  assert.equal(duration(5 * 60_000), '5 min');
  assert.equal(duration((3 * 60 + 57) * 60_000), '3 h 57 min');
  assert.equal(duration(2 * 3_600_000), '2 h');
  assert.equal(duration((2 * 24 + 3) * 3_600_000), '2 d 3 h');
});

test('relative times say in which direction', () => {
  const now = Date.parse('2026-10-03T12:00:00Z');
  assert.equal(relative('2026-10-03T12:05:00Z', now), 'in 5 min');
  assert.equal(relative('2026-10-03T11:59:46Z', now), '14 s ago');
  assert.equal(relative('2026-10-03T12:00:00.400Z', now), 'just now');
  assert.equal(relative('not a time', now), '');
});

test('ISO-8601 durations as stored in retry policies are read and shown compactly', () => {
  assert.equal(parseIsoDuration('PT1M30S'), 90_000);
  assert.equal(parseIsoDuration('PT0.5S'), 500);
  assert.equal(parseIsoDuration('P1DT2H'), 26 * 3_600_000);
  assert.equal(parseIsoDuration('5s'), null);
  assert.equal(parseIsoDuration('PT'), null);
  assert.equal(compactDuration(90_000), '1m 30s');
  assert.equal(isoDurationText('PT5M'), '5m');
  assert.equal(isoDurationText('weird'), 'weird');
});

test('dates use day/month/year and a 24-hour clock, in local time', () => {
  const d = new Date(2026, 9, 3, 14, 2, 10);
  assert.equal(dateTime(d), '03/10/2026 • 14:02:10');
  assert.equal(localInputValue(d), '2026-10-03T14:02');
  assert.equal(dateTime('nope'), '—');
});

test('small words', () => {
  assert.equal(count(1, 'message'), '1 message');
  assert.equal(count(3, 'message'), '3 messages');
  assert.equal(capitalize('RUNNING'), 'Running');
});
