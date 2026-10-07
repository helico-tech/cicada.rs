import { test } from 'vitest';
import assert from 'node:assert/strict';
import { ApiError, ContractError, createApi, fetchPages } from '../src/api/client';
import { ProblemTypes } from '../src/api/generated';

type Answer = { status: number; body?: unknown } | Error;

/** A fetch that records requests and answers from [responses] in order. */
function fakeFetch(...responses: Answer[]) {
  const calls: Array<{ url: string } & RequestInit> = [];
  const fetch = (async (url: string, init: RequestInit) => {
    calls.push({ url, ...init });
    const r = responses.shift()!;
    if (r instanceof Error) throw r;
    const body = r.body === undefined ? '' : typeof r.body === 'string' ? r.body : JSON.stringify(r.body);
    return { ok: r.status >= 200 && r.status < 300, status: r.status, text: async () => body } as Response;
  }) as typeof globalThis.fetch;
  return { fetch, calls };
}

const header = (call: RequestInit, name: string) => (call.headers as Record<string, string>)[name];

test('requests go below the API root with query parameters, JSON bodies and the bearer token', async () => {
  const { fetch, calls } = fakeFetch({ status: 200, body: { items: [], next: null } }, { status: 202, body: { id: 'm1', deliverAt: 't' } });
  const api = createApi({ base: '/api', fetch, token: () => 's3cret' });
  await api.listActors({ prefix: 'actor://sync/', limit: 100, cursor: null });
  const sent = await api.send('actor://jobs/messages', { type: 'Run', payload: {}, retention: { done: { kind: 'remove' } } });
  assert.equal(calls[0]!.url, '/api/actors?prefix=actor%3A%2F%2Fsync%2F&limit=100');
  assert.equal(calls[0]!.method, 'GET');
  assert.equal(header(calls[0]!, 'Authorization'), 'Bearer s3cret');
  assert.equal(calls[1]!.url, '/api/actors/jobs/%6Dessages/messages');
  assert.equal(header(calls[1]!, 'Content-Type'), 'application/json');
  assert.equal(calls[1]!.body, '{"type":"Run","payload":{},"retention":{"done":{"kind":"remove"}}}');
  assert.deepEqual(sent, { status: 202, body: { id: 'm1', deliverAt: 't' } });
});

test('message ids and URIs in queries are encoded; create sends the state envelope; chains have their own path', async () => {
  const page = { items: [], next: null };
  const { fetch, calls } = fakeFetch(
    { status: 200, body: { focus: 'a/b c', nodes: [], truncated: false } },
    { status: 201, body: { uri: 'actor://sync/acme', created: true } },
    { status: 200, body: { type: { template: 't', hash: 'h', state: null, messages: {} }, params: {} } },
    { status: 200, body: page },
  );
  const api = createApi({ base: '/', fetch });
  await api.chain('a/b c');
  await api.createActor('actor://sync/acme', { source: 'x' });
  await api.typeOf('actor://sync/a b');
  await api.listPending({ prefix: 'actor://sync/', cursor: '17:3:actor://sync/a', limit: 2 });
  assert.equal(calls[0]!.url, '/messages/a%2Fb%20c/chain');
  assert.equal(header(calls[0]!, 'Authorization'), undefined);
  assert.equal(calls[1]!.method, 'PUT');
  assert.equal(calls[1]!.body, '{"state":{"source":"x"}}');
  assert.equal(calls[2]!.url, '/types?uri=actor%3A%2F%2Fsync%2Fa+b');
  assert.equal(calls[3]!.url, '/messages?prefix=actor%3A%2F%2Fsync%2F&cursor=17%3A3%3Aactor%3A%2F%2Fsync%2Fa&limit=2');
});

test('problems become ApiErrors that say whether retrying helps', async () => {
  const { fetch } = fakeFetch(
    { status: 409, body: { type: ProblemTypes.MESSAGE_RUNNING, title: 'Message is running', status: 409, detail: 'busy', instance: '/m' } },
    { status: 503, body: { type: ProblemTypes.STORE_UNAVAILABLE, title: 'Store unavailable', status: 503, detail: null, instance: null } },
    { status: 401, body: '' },
    new TypeError('Failed to fetch'),
  );
  const api = createApi({ base: '/', fetch });
  const conflict = await api.cancel('m1').catch((e: unknown) => e);
  assert.ok(conflict instanceof ApiError);
  assert.equal(conflict.type, ProblemTypes.MESSAGE_RUNNING);
  assert.equal(conflict.title, 'Message is running');
  assert.equal(conflict.detail, 'busy');
  assert.equal(conflict.code, '409 Conflict');
  assert.equal(conflict.temporary, false);
  const unavailable = (await api.health().catch((e: unknown) => e)) as ApiError;
  assert.equal(unavailable.temporary, true);
  const unauthorized = (await api.types().catch((e: unknown) => e)) as ApiError;
  assert.equal(unauthorized.status, 401);
  assert.equal(unauthorized.title, 'Unauthorized');
  const offline = (await api.types().catch((e: unknown) => e)) as ApiError;
  assert.equal(offline.status, 0);
  assert.equal(offline.temporary, true);
  assert.equal(offline.code, 'No response');
});

test('a response that does not match the API types is refused with the field that differs', async () => {
  const { fetch } = fakeFetch({ status: 200, body: { status: 'up', node: 'n1' } }, { status: 200, body: { items: [{ id: 1 }], next: null } });
  const api = createApi({ base: '/', fetch });
  const health = (await api.health().catch((e: unknown) => e)) as ContractError;
  assert.ok(health instanceof ContractError);
  assert.match(health.detail, /health.*state/);
  const failures = (await api.failures().catch((e: unknown) => e)) as ContractError;
  assert.ok(failures instanceof ContractError);
  assert.match(failures.detail, /items\.0\.id/);
});

test('paged listings are followed up to a limit', async () => {
  const pages: Record<string, { items: number[]; next: string | null }> = {
    '': { items: [1, 2], next: 'c1' }, c1: { items: [3], next: 'c2' }, c2: { items: [4], next: null },
  };
  const all = await fetchPages(async (cursor) => pages[cursor ?? '']!, { maxPages: 5 });
  assert.deepEqual(all, { items: [1, 2, 3, 4], truncated: false, next: null });
  const some = await fetchPages(async (cursor) => pages[cursor ?? '']!, { maxPages: 2 });
  assert.deepEqual(some, { items: [1, 2, 3], truncated: true, next: 'c2' });
});
