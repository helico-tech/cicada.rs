// Real responses of every endpoint, recorded from a running node by crates/cicada-ui/tests/api_fixtures.rs (which then
// runs this file), parsed with the validators generated from the wire classes: the console can read whatever the node
// sends.

import { test } from 'vitest';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import type { z } from 'zod';
import * as generated from '../src/api/generated';

const dir = process.env.CICADA_API_FIXTURES;

test('every recorded response matches its generated validator', () => {
  assert.ok(dir, 'CICADA_API_FIXTURES is not set: run `cargo test -p cicada-ui --test api_fixtures`, which records the fixtures first');
  assert.ok(existsSync(join(dir, 'manifest.json')), `No fixtures in ${dir}: run cargo test -p cicada-ui --test api_fixtures`);
  const manifest = JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8')) as Record<string, string>;
  assert.ok(Object.keys(manifest).length >= 20, 'every endpoint is recorded');
  const schemas = generated as unknown as Record<string, z.ZodType | undefined>;
  for (const [file, name] of Object.entries(manifest)) {
    const schema = schemas[name + 'Schema'];
    assert.ok(schema, `${file}: no generated schema ${name}Schema`);
    const result = schema.safeParse(JSON.parse(readFileSync(join(dir, file), 'utf8')));
    assert.ok(result.success, `${file} does not match ${name}: ${result.success ? '' : JSON.stringify(result.error.issues)}`);
  }
});

test('the recorded removed message shows as removed in its chain', () => {
  if (!dir) return;
  const chain = generated.MessageChainViewSchema.parse(JSON.parse(readFileSync(join(dir, 'chain-removed.json'), 'utf8')));
  assert.equal(chain.nodes.find((n) => n.id === chain.focus)?.removed, true);
});
