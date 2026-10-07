import { test } from 'vitest';
import assert from 'node:assert/strict';
import {
  actorPath, buildUri, combinePrefixes, encodeSegment, matchTemplate, normalizePrefix, resolveType, templateParams,
  templatePrefix,
} from '../src/lib/address';

test('segments are percent-encoded outside the unreserved characters, as UTF-8', () => {
  assert.equal(encodeSegment('acme'), 'acme');
  assert.equal(encodeSegment('a b/c'), 'a%20b%2Fc');
  assert.equal(encodeSegment('é'), '%C3%A9');
  assert.equal(encodeSegment('A-z_0.9~'), 'A-z_0.9~');
});

test('an instance path escapes a last segment named messages and dot segments, like ActorAddress.toHttpPath', () => {
  assert.equal(actorPath('actor://sync/acme'), 'actors/sync/acme');
  assert.equal(actorPath('actor://jobs/messages'), 'actors/jobs/%6Dessages');
  assert.equal(actorPath('actor://messages/x'), 'actors/messages/x');
  assert.equal(actorPath('actor://a/./..'), 'actors/a/%2E/%2E%2E');
  assert.equal(actorPath('actor://counter/a%20b'), 'actors/counter/a%20b');
});

test('templates match by segment count and literals, and decode their parameters', () => {
  const t = 'actor://billing/{tenant}/invoice/{invoiceId}';
  assert.deepEqual(templateParams(t), ['tenant', 'invoiceId']);
  assert.deepEqual(matchTemplate(t, 'actor://billing/acme/invoice/INV%201'), { tenant: 'acme', invoiceId: 'INV 1' });
  assert.equal(matchTemplate(t, 'actor://billing/acme/invoice'), null);
  assert.equal(matchTemplate(t, 'actor://billing/acme/receipt/1'), null);
  assert.equal(matchTemplate('actor://sync/{tenant}', 'http://sync/acme'), null);
  assert.deepEqual(matchTemplate('actor://maintenance/cleanup', 'actor://maintenance/cleanup'), {});
});

test('a URI resolves to the one type whose template matches', () => {
  const types = [{ template: 'actor://jobs/{id}' }, { template: 'actor://jobs/{id}/steps/{n}' }];
  assert.equal(resolveType(types, 'actor://jobs/7/steps/2')!.type.template, 'actor://jobs/{id}/steps/{n}');
  assert.deepEqual(resolveType(types, 'actor://jobs/7')!.params, { id: '7' });
  assert.equal(resolveType(types, 'actor://other/7'), null);
});

test('a template prefix is its literal segments up to the first parameter', () => {
  assert.equal(templatePrefix('actor://sync/{tenant}'), 'actor://sync/');
  assert.equal(templatePrefix('actor://e2e/counter/{id}'), 'actor://e2e/counter/');
  assert.equal(templatePrefix('actor://{a}/x'), 'actor://');
  assert.equal(templatePrefix('actor://maintenance/cleanup'), 'actor://maintenance/cleanup');
});

test('prefix filters may leave out the scheme and combine into the narrower one, or none', () => {
  assert.equal(normalizePrefix(' sync/ac '), 'actor://sync/ac');
  assert.equal(normalizePrefix(''), '');
  assert.equal(combinePrefixes('actor://sync/', 'actor://sync/ac'), 'actor://sync/ac');
  assert.equal(combinePrefixes('actor://sync/ac', 'actor://'), 'actor://sync/ac');
  assert.equal(combinePrefixes('actor://sync/', 'actor://jobs/'), null);
  assert.equal(combinePrefixes('', 'actor://jobs/'), 'actor://jobs/');
});

test('a URI is built from a template with encoded values and knows when it is complete', () => {
  const partial = buildUri('actor://billing/{tenant}/invoice/{invoiceId}', { tenant: 'a b' });
  assert.equal(partial.uri, 'actor://billing/a%20b/invoice/{invoiceId}');
  assert.equal(partial.complete, false);
  assert.deepEqual(partial.parts.map((p) => p.filled), [true, true, true, false]);
  const full = buildUri('actor://sync/{tenant}', { tenant: 'acme' });
  assert.equal(full.uri, 'actor://sync/acme');
  assert.equal(full.complete, true);
});
