import { test } from 'vitest';
import assert from 'node:assert/strict';
import { addedKeys, jsonChanges, pointer, removedKeys } from '../src/lib/diff';

test('nothing is new or removed on the first load', () => {
  assert.equal(addedKeys(null, ['a']).size, 0);
  assert.deepEqual(removedKeys(null, ['a']), []);
});

test('keys are new or removed against the previous refresh', () => {
  const previous = new Set(['a', 'b']);
  assert.deepEqual([...addedKeys(previous, ['b', 'c'])], ['c']);
  assert.deepEqual(removedKeys(previous, ['b', 'c']), ['a']);
});

test('pointers escape ~ and /', () => {
  assert.equal(pointer('', 'a/b~c'), '/a~1b~0c');
  assert.equal(pointer('/x', 0), '/x/0');
});

test('JSON changes are reported at the changed leaf, with the value before', () => {
  const before = { status: 'idle', pending: 2, source: { kind: 'x', objects: ['A'] }, gone: 1 };
  const after = { status: 'syncing', pending: 2, source: { kind: 'x', objects: ['A', 'B'] }, cursor: 'c1' };
  const { changed, removed } = jsonChanges(before, after)!;
  assert.deepEqual([...changed.keys()].sort(), ['/cursor', '/source/objects/1', '/status']);
  assert.equal(changed.get('/status')!.before, 'idle');
  assert.equal(changed.get('/cursor')!.before, undefined);
  assert.equal(removed, 1);
});

test('a value changing shape is one change; equal values are none; the first load is null', () => {
  assert.deepEqual([...jsonChanges({ a: [1] }, { a: { 0: 1 } })!.changed.keys()], ['/a']);
  assert.equal(jsonChanges({ a: { b: [1, { c: null }] } }, { a: { b: [1, { c: null }] } })!.changed.size, 0);
  assert.deepEqual([...jsonChanges(null, { a: 1 })!.changed.keys()], ['']);
  assert.equal(jsonChanges(undefined, { a: 1 }), null);
});
