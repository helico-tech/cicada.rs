import { test } from 'vitest';
import assert from 'node:assert/strict';
import {
  describe, draftOf, emptyDraft, hint, readDraft, skeleton, summary, type DraftObject, type SchemaNode,
} from '../src/lib/schema';

// A JSON Schema as served by GET /types.
const ORDER = {
  $defs: {
    circle: { properties: { radius: { type: 'number' }, type: { const: 'circle', type: 'string' } }, required: ['type', 'radius'], type: 'object' },
    'Priority': { enum: ['LOW', 'HIGH'], type: 'string' },
    'Shape': { oneOf: [{ $ref: '#/$defs/circle' }, { $ref: '#/$defs/square' }] },
    square: { properties: { side: { type: 'number' }, type: { const: 'square', type: 'string' } }, required: ['type', 'side'], type: 'object' },
  },
  $schema: 'https://json-schema.org/draft/2020-12/schema',
  properties: {
    at: { type: 'string' },
    extra: {},
    note: { type: ['string', 'null'] },
    prices: { additionalProperties: { type: 'integer' }, type: 'object' },
    priority: { $ref: '#/$defs/Priority' },
    quantity: { type: 'integer' },
    replyTo: { type: ['string', 'null'] },
    shape: { $ref: '#/$defs/Shape' },
    sku: { type: 'string' },
    tags: { items: { type: 'string' }, type: 'array' },
  },
  required: ['sku', 'quantity', 'note', 'prices', 'shape', 'at'],
  title: 'Order',
  type: 'object',
};

const fieldsOf = (node: SchemaNode) => (node.kind === 'object' ? node.fields : []);
const kinds = (node: SchemaNode) => Object.fromEntries(fieldsOf(node).map((f) => [f.name, f.node.kind]));

test('fields get an editor by kind; unions, maps and free JSON get a JSON box', () => {
  const node = describe(ORDER);
  assert.equal(node.kind, 'object');
  assert.deepEqual(kinds(node), {
    at: 'string', extra: 'json', note: 'string', prices: 'json', priority: 'enum', quantity: 'integer',
    replyTo: 'string', shape: 'json', sku: 'string', tags: 'list',
  });
  const note = fieldsOf(node).find((f) => f.name === 'note')!;
  assert.equal(note.required, true);
  assert.equal(note.node.nullable, true);
  assert.equal(hint(note.node), 'string · nullable');
  assert.equal(hint(fieldsOf(node).find((f) => f.name === 'tags')!.node), 'array<string>');
  assert.equal(hint(fieldsOf(node).find((f) => f.name === 'priority')!.node), 'string · enum');
});

test('an object without fields, like a data object message, is an empty form', () => {
  const tick = { $schema: 'https://json-schema.org/draft/2020-12/schema', title: 'Tick', type: 'object' };
  assert.deepEqual(describe(tick), { kind: 'object', fields: [], nullable: false });
  assert.equal(summary(tick), '{} — no fields');
  assert.deepEqual(readDraft(describe(tick), emptyDraft(describe(tick))), { value: {}, errors: [] });
});

test('a summary marks required fields', () => {
  assert.equal(summary(ORDER), 'at*, extra, note*, prices*, priority, quantity*, replyTo, shape*, sku*, tags');
  assert.equal(summary({ type: 'string' }), 'string');
});

test('an untouched form reports the required fields it cannot fill and leaves optional ones out', () => {
  const node = describe(ORDER);
  const { value, errors } = readDraft(node, emptyDraft(node));
  assert.deepEqual(errors.map((e) => e.path).sort(), ['/prices', '/quantity', '/shape']);
  // A required string may be empty; a required nullable field is null; optional fields are left out.
  assert.deepEqual(value, { at: '', note: null, sku: '' });
});

test('a filled form reads into the JSON the serializer expects', () => {
  const node = describe(ORDER);
  const draft = {
    ...(emptyDraft(node) as DraftObject),
    sku: 'S-1', quantity: ' 3 ', note: 'hi', prices: '{"EUR": 100}', priority: 'HIGH', tags: ['a', 'b'],
    shape: '{"type": "circle", "radius": 1.5}', at: '2026-10-03T12:00:00Z', extra: '', replyTo: '',
  };
  assert.deepEqual(readDraft(node, draft), {
    value: {
      at: '2026-10-03T12:00:00Z', note: 'hi', prices: { EUR: 100 }, priority: 'HIGH', quantity: 3,
      shape: { type: 'circle', radius: 1.5 }, sku: 'S-1', tags: ['a', 'b'],
    },
    errors: [],
  });
});

test('numbers and JSON are checked as far as the form can see', () => {
  const node = describe({
    type: 'object',
    properties: { n: { type: 'integer' }, x: { type: 'number' }, j: {}, l: { type: 'array', items: { type: 'integer' } } },
    required: ['n', 'x', 'j', 'l'],
  });
  const { errors } = readDraft(node, { n: '1.5', x: 'abc', j: '{', l: ['1', 'two'] });
  assert.deepEqual(errors, [
    { path: '/n', message: 'Must be a whole number' },
    { path: '/x', message: 'Must be a number' },
    { path: '/j', message: 'Not valid JSON' },
    { path: '/l/1', message: 'Must be a whole number' },
  ]);
});

test('nested objects are fields of their own; an optional one left blank is left out', () => {
  const node = describe({
    type: 'object',
    properties: {
      source: { type: 'object', properties: { kind: { type: 'string' }, region: { type: 'string' } }, required: ['kind'] },
      paused: { type: 'boolean' },
    },
  });
  assert.deepEqual(readDraft(node, emptyDraft(node)), { value: { paused: false }, errors: [] });
  const draft = { source: { kind: 'sf', region: '' }, paused: true };
  assert.deepEqual(readDraft(node, draft).value, { source: { kind: 'sf' }, paused: true });
});

test('a value round-trips through a draft, unless the form would lose something', () => {
  const node = describe(ORDER);
  const value = { sku: 'S-1', quantity: 3, note: null, prices: { EUR: 1 }, shape: { type: 'square', side: 2 }, at: 'x', tags: ['t'] };
  const draft = draftOf(node, value)!;
  assert.deepEqual(readDraft(node, draft).value, value);
  assert.equal(draftOf(node, { ...value, unknown: 1 }), null);
  assert.equal(draftOf(node, { ...value, quantity: 'three' }), null);
  assert.equal(draftOf(node, [1]), null);
});

test('a skeleton holds the required fields with neutral values', () => {
  assert.deepEqual(skeleton(describe(ORDER)), { sku: '', quantity: 0, note: null, prices: null, shape: null, at: '' });
});
