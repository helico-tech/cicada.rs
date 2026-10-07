import { test } from 'vitest';
import assert from 'node:assert/strict';
import { href, parseRoute } from '../src/lib/routes';

test('routes round-trip through their links', () => {
  assert.deepEqual(parseRoute(href.actor('actor://sync/a%20b')), { name: 'actor', uri: 'actor://sync/a%20b' });
  assert.deepEqual(parseRoute(href.message('m/1?x')), { name: 'message', id: 'm/1?x' });
  assert.deepEqual(parseRoute(href.actors({ type: 'actor://sync/{tenant}', prefix: '' })), {
    name: 'actors', query: { type: 'actor://sync/{tenant}' },
  });
  assert.deepEqual(parseRoute(href.failures({ prefix: 'actor://sync/acme' })), {
    name: 'failures', query: { prefix: 'actor://sync/acme' },
  });
  assert.deepEqual(parseRoute(href.types()), { name: 'types', query: {} });
  assert.deepEqual(parseRoute(href.pending({ type: 'actor://sync/{tenant}', prefix: 'actor://sync/a' })), {
    name: 'pending', query: { type: 'actor://sync/{tenant}', prefix: 'actor://sync/a' },
  });
});

test('an empty or unknown hash is the actor list', () => {
  assert.deepEqual(parseRoute(''), { name: 'actors', query: {} });
  assert.deepEqual(parseRoute('#/nope'), { name: 'actors', query: {} });
  assert.deepEqual(parseRoute('#/actor/'), { name: 'actors', query: {} });
});
