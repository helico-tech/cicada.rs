// The layout of a message chain (GET /messages/{id}/chain): the flat list of nodes linked by `causedBy`, laid out as
// a tree from cause (left) to effects (right). Leaves take one row each; a parent sits in the middle of its children.

import type { ChainNodeView } from '../api/wire';

export interface TreeNode {
  node: ChainNodeView;
  depth: number;
  children: TreeNode[];
}

export interface PlacedNode {
  node: ChainNodeView;
  depth: number;
  /** Row position: leaves are whole rows, parents sit between their first and last child. */
  row: number;
}

export interface ChainLayout {
  /** Top-down, depth first: the order of the compact list. */
  placed: PlacedNode[];
  edges: Array<{ from: string; to: string }>;
  /** The root's own cause is not in the chain: sent from outside, or past retention / removed. */
  missingCause: string | null;
  rows: number;
  depth: number;
}

const byDelivery = (a: TreeNode, b: TreeNode): number =>
  a.node.deliverAt.localeCompare(b.node.deliverAt) || a.node.id.localeCompare(b.node.id);

/** Builds the tree. A node whose cause is not in the list is a root; normally there is exactly one. */
export function chainTree(nodes: readonly ChainNodeView[]): TreeNode[] {
  const byId = new Map<string, TreeNode>();
  for (const node of nodes) byId.set(node.id, { node, depth: 0, children: [] });
  const roots: TreeNode[] = [];
  for (const tree of byId.values()) {
    const parent = tree.node.causedBy ? byId.get(tree.node.causedBy) : undefined;
    if (parent && parent !== tree) parent.children.push(tree);
    else roots.push(tree);
  }
  const setDepth = (t: TreeNode, depth: number): void => {
    t.depth = depth;
    t.children.sort(byDelivery);
    for (const c of t.children) setDepth(c, depth + 1);
  };
  roots.sort(byDelivery);
  for (const r of roots) setDepth(r, 0);
  return roots;
}

export function layoutChain(nodes: readonly ChainNodeView[]): ChainLayout {
  const roots = chainTree(nodes);
  const placed: PlacedNode[] = [];
  const edges: Array<{ from: string; to: string }> = [];
  let nextRow = 0;
  let depth = 0;
  const place = (t: TreeNode): number => {
    const entry: PlacedNode = { node: t.node, depth: t.depth, row: 0 };
    placed.push(entry);
    depth = Math.max(depth, t.depth);
    if (t.children.length === 0) {
      entry.row = nextRow++;
      return entry.row;
    }
    const rows = t.children.map((c) => {
      edges.push({ from: t.node.id, to: c.node.id });
      return place(c);
    });
    entry.row = ((rows[0] ?? 0) + (rows[rows.length - 1] ?? 0)) / 2;
    return entry.row;
  };
  for (const r of roots) place(r);
  const first = roots[0]?.node;
  return {
    placed,
    edges,
    missingCause: first?.causedBy ?? null,
    rows: nextRow,
    depth,
  };
}
