import { test } from 'vitest';
import assert from 'node:assert/strict';
import type { ChainNodeView } from '../src/api/wire';
import { chainTree, layoutChain } from '../src/lib/chain';

let clock = 0;
function node(id: string, causedBy: string | null, extra: Partial<ChainNodeView> = {}): ChainNodeView {
  clock += 1;
  return {
    id, causedBy, to: 'actor://x/' + id, from: null, type: 'T', status: 'done',
    deliverAt: new Date(Date.UTC(2026, 9, 4, 12, 0, clock)).toISOString(), finishedAt: null, error: null, attempt: 1, failures: 0, removed: false,
    ...extra,
  };
}

test('nodes link into a tree by causedBy, children in delivery order', () => {
  const nodes = [node('root', null), node('b', 'root'), node('a', 'root'), node('a1', 'a')];
  const [root, ...others] = chainTree(nodes);
  assert.equal(others.length, 0);
  assert.equal(root!.node.id, 'root');
  assert.deepEqual(root!.children.map((c) => c.node.id), ['b', 'a'], 'b was delivered before a');
  assert.deepEqual(root!.children[1]!.children.map((c) => [c.node.id, c.depth]), [['a1', 2]]);
});

test('leaves take one row each and a parent sits between its first and last child', () => {
  const layout = layoutChain([node('r', null), node('x', 'r'), node('y', 'r'), node('y1', 'y'), node('y2', 'y')]);
  const at = Object.fromEntries(layout.placed.map((p) => [p.node.id, [p.depth, p.row]]));
  assert.deepEqual(at, { r: [0, 0.75], x: [1, 0], y: [1, 1.5], y1: [2, 1], y2: [2, 2] });
  assert.equal(layout.rows, 3);
  assert.equal(layout.depth, 2);
  assert.deepEqual(layout.placed.map((p) => p.node.id), ['r', 'x', 'y', 'y1', 'y2'], 'depth first, for the list');
  assert.deepEqual(layout.edges, [{ from: 'r', to: 'x' }, { from: 'r', to: 'y' }, { from: 'y', to: 'y1' }, { from: 'y', to: 'y2' }]);
  assert.equal(layout.missingCause, null);
});

test('a chain whose first cause is gone says so', () => {
  const layout = layoutChain([node('child', 'expired-parent'), node('grandchild', 'child')]);
  assert.equal(layout.missingCause, 'expired-parent');
  assert.deepEqual(layout.placed.map((p) => [p.node.id, p.depth]), [['child', 0], ['grandchild', 1]]);
});

test('a single message is one row', () => {
  const layout = layoutChain([node('only', null)]);
  assert.deepEqual(layout.placed.map((p) => p.row), [0]);
  assert.equal(layout.rows, 1);
  assert.deepEqual(layout.edges, []);
});
