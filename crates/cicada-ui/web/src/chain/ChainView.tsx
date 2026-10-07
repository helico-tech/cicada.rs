// The message chain on the message page: what led to the message (left) and everything it caused (right), as a tree
// graph with pan and zoom, or as an indented list (the default on narrow screens, and for screen readers).

import { useMemo, useRef, useState } from 'react';
import {
  Background, BackgroundVariant, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow,
  type Edge, type Node, type NodeProps, type ReactFlowInstance,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import type { ChainNodeView, MessageChainView } from '../api/generated';
import { withoutScheme } from '../lib/address';
import { count, shortDateTime } from '../lib/format';
import { layoutChain, type ChainLayout } from '../lib/chain';
import { href } from '../lib/routes';
import { Badge, Card, cx, Icon, STATUS, StatusBadge, Tabs } from '../halo';

const NODE_WIDTH = 248;
const NODE_HEIGHT = 84;
const COLUMN = NODE_WIDTH + 64;
const ROW = NODE_HEIGHT + 20;
/** Below this zoom the cards are hard to read: the view then centers on the message itself instead of fitting all. */
const READABLE_ZOOM = 0.8;
const PADDING = 72; // leaves room for the zoom controls at the bottom left

type MessageNodeData = { view: ChainNodeView; focus: boolean };
type MessageNode = Node<MessageNodeData, 'message'>;

/** One message as a card: type and status, the actor it went to, when it finished or is due. */
function MessageCard({ data }: NodeProps<MessageNode>) {
  const { view: n, focus } = data;
  const tone = STATUS[n.status].tone;
  return (
    <div className={cx('cc-chain-node', 'cc-chain-' + tone, focus && 'cc-chain-focus', n.removed && 'cc-chain-removed')}>
      <Handle type="target" position={Position.Left} isConnectable={false} />
      <div className="cc-chain-node-head">
        <span className="mono cc-strong cc-trunc">{n.type}</span>
        <StatusBadge status={n.status} />
      </div>
      <div className="mono cc-xsmall cc-muted cc-trunc" title={n.to}>{withoutScheme(n.to)}</div>
      <div className="cc-caption cc-chain-node-foot">
        <span className="ha-num">{shortDateTime(n.finishedAt ?? n.deliverAt)}</span>
        {n.removed ? <span>record removed</span> : n.failures ? <span className="cc-critical">{count(n.failures, 'failure')}</span> : null}
        {focus ? <span className="cc-chain-here">this message</span> : null}
      </div>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </div>
  );
}

const nodeTypes = { message: MessageCard };

function toFlow(layout: ChainLayout, focus: string): { nodes: MessageNode[]; edges: Edge[] } {
  const nodes: MessageNode[] = layout.placed.map((p) => ({
    id: p.node.id,
    type: 'message',
    position: { x: p.depth * COLUMN, y: p.row * ROW },
    data: { view: p.node, focus: p.node.id === focus },
    draggable: false,
    connectable: false,
    width: NODE_WIDTH,
    height: NODE_HEIGHT,
  }));
  const edges: Edge[] = layout.edges.map((e) => ({
    id: e.from + '>' + e.to,
    source: e.from,
    target: e.to,
    type: 'smoothstep',
    markerEnd: { type: MarkerType.ArrowClosed, width: 16, height: 16 },
    className: 'cc-chain-edge',
  }));
  return { nodes, edges };
}

/** Fits the whole chain when it stays readable; otherwise shows the message itself at a readable zoom. */
function initialView(instance: ReactFlowInstance<MessageNode>, container: HTMLDivElement | null, layout: ChainLayout, focus: string): void {
  const width = (layout.depth + 1) * COLUMN - (COLUMN - NODE_WIDTH) + 2 * PADDING;
  const height = Math.max(1, layout.rows) * ROW - (ROW - NODE_HEIGHT) + 2 * PADDING;
  const box = container?.getBoundingClientRect();
  const fits = box ? Math.min(box.width / width, box.height / height) >= READABLE_ZOOM : true;
  const target = layout.placed.find((p) => p.node.id === focus);
  if (fits || !target || !box) {
    void instance.fitView({ padding: 0.15, maxZoom: 1 });
    return;
  }
  // The column of its cause at the left edge, the message itself centred vertically: its effects fill the rest.
  const zoom = 0.9;
  const left = Math.max(0, target.depth - 1) * COLUMN;
  void instance.setViewport({ x: PADDING - left * zoom, y: box.height / 2 - (target.row * ROW + NODE_HEIGHT / 2) * zoom, zoom });
}

function Graph({ layout, focus }: { layout: ChainLayout; focus: string }) {
  const { nodes, edges } = useMemo(() => toFlow(layout, focus), [layout, focus]);
  const container = useRef<HTMLDivElement>(null);
  // Tall enough for the tree, within limits; the graph pans and zooms beyond that.
  const height = Math.min(560, Math.max(220, layout.rows * ROW + 80));
  return (
    <div ref={container} className="cc-chain-graph" style={{ height }}>
      <ReactFlow
        nodes={nodes} edges={edges} nodeTypes={nodeTypes} onInit={(instance) => initialView(instance, container.current, layout, focus)}
        minZoom={0.2} maxZoom={1.5} nodesDraggable={false} nodesConnectable={false} elementsSelectable={false}
        proOptions={{ hideAttribution: true }} onNodeClick={(_, node) => { location.hash = href.message(node.id); }}
      >
        <Background variant={BackgroundVariant.Dots} gap={20} size={1} />
        <Controls showInteractive={false} />
        {layout.placed.length > 12 ? <MiniMap pannable zoomable nodeClassName={(n) => 'cc-chain-mini-' + STATUS[(n.data as MessageNodeData).view.status].tone} /> : null}
      </ReactFlow>
    </div>
  );
}

function List({ layout, focus }: { layout: ChainLayout; focus: string }) {
  return (
    <ol className="cc-chain-list" aria-label="Message chain">
      {layout.placed.map((p) => (
        <li key={p.node.id} className={cx('cc-chain-item', p.node.id === focus && 'cc-chain-item-focus')} style={{ paddingLeft: 12 + p.depth * 20 }}>
          {p.depth > 0 ? <span className="cc-faint" aria-hidden="true"><Icon name="chevron-right" size={14} /></span> : null}
          <a className="cc-link mono cc-strong" href={href.message(p.node.id)}>{p.node.type}</a>
          <StatusBadge status={p.node.status} />
          <span className="mono cc-xsmall cc-muted cc-trunc">{withoutScheme(p.node.to)}</span>
          <span className="cc-caption ha-num">{shortDateTime(p.node.finishedAt ?? p.node.deliverAt)}</span>
          {p.node.removed ? <Badge>Removed</Badge> : null}
          {p.node.id === focus ? <span className="cc-chain-here">this message</span> : null}
        </li>
      ))}
    </ol>
  );
}

export function ChainView({ chain }: { chain: MessageChainView }) {
  const layout = useMemo(() => layoutChain(chain.nodes), [chain.nodes]);
  const narrow = typeof matchMedia === 'function' && matchMedia('(max-width: 720px)').matches;
  const [mode, setMode] = useState<'graph' | 'list'>(narrow ? 'list' : 'graph');
  const actors = new Set(chain.nodes.map((n) => n.to)).size;
  const notes: string[] = [];
  if (layout.missingCause) notes.push('Its first cause is no longer kept (past retention, or removed).');
  if (chain.truncated) notes.push('The chain goes on beyond what is shown (50 steps either way, 500 caused messages).');

  if (chain.nodes.length <= 1 && !layout.missingCause) {
    return (
      <Card title="Message Chain" subtitle="Sent from outside an actor · caused no messages">
        <div className="cc-caption">This message was not sent by another message’s turn, and its own turn sent nothing (yet).</div>
      </Card>
    );
  }
  return (
    <Card
      title="Message Chain"
      subtitle={`${count(chain.nodes.length, 'message')} · ${count(actors, 'actor')} · cause on the left, effects on the right`}
      actions={<Tabs variant="segmented" label="Chain view" value={mode} onChange={setMode} items={[{ value: 'graph', label: 'Graph' }, { value: 'list', label: 'List' }]} />}
      flush
    >
      {notes.length ? <div className="cc-chain-notes cc-caption">{notes.join(' ')}</div> : null}
      {mode === 'graph' ? <Graph layout={layout} focus={chain.focus} /> : <List layout={layout} focus={chain.focus} />}
    </Card>
  );
}
