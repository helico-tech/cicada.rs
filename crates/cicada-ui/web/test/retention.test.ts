import { test } from 'vitest';
import assert from 'node:assert/strict';
import { blankKeep, keepText, readRetention, type KeepDraft } from '../src/lib/retention';

const keep = (kind: KeepDraft['kind'], extra: Partial<KeepDraft> = {}): KeepDraft => ({ ...blankKeep(), kind, ...extra });

test('two defaults send no policy; any rule sends only the rules given', () => {
  assert.deepEqual(readRetention(blankKeep(), blankKeep()), { retention: undefined });
  assert.deepEqual(readRetention(keep('remove'), blankKeep()), { retention: { done: { kind: 'remove' } } });
  assert.deepEqual(readRetention(keep('last', { count: '25' }), keep('for', { age: ' P30D ' })), {
    retention: { done: { kind: 'last', count: 25 }, failed: { kind: 'for', age: 'P30D' } },
  });
});

test('a count outside 1..10000 or an empty age is refused before sending', () => {
  for (const count of ['0', '10001', '2.5', 'many']) {
    assert.ok('error' in readRetention(keep('last', { count }), blankKeep()), count);
  }
  assert.ok('error' in readRetention(blankKeep(), keep('for', { age: '  ' })));
});

test('rules read as words', () => {
  assert.equal(keepText(null), 'Store default (its retention period, 7 days unless configured)');
  assert.equal(keepText({ kind: 'remove' }), 'Removed as soon as it finishes');
  assert.equal(keepText({ kind: 'for', age: 'P30D' }), 'Kept for 30d after it finished');
  assert.equal(keepText({ kind: 'last', count: 5 }), 'Kept while among the last 5 of this actor');
});
