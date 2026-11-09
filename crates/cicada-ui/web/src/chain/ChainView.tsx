// The message chain on the message page: what led to the message (top) and everything it caused (below), as a tree
// of cards joined by connector lines with pan and zoom, or as an indented list (the default on narrow screens, and for
// screen readers).

import { useMemo, useRef, useState } from 'react';
import {
  Background, BackgroundVariant, Controls, Handle, MiniMap, Position, ReactFlow,
  type Edge, type Node, type NodeProps, type ReactFlowInstance,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import type { ChainNodeView, MessageChainView } from '../api/wire';
import { withoutScheme } from '../lib/address';
import { count, shortDateTime } from '../lib/format';
import { layoutChain, type ChainLayout } from '../lib/chain';
import { href } from '../lib/routes';
import { Badge, cx, Icon, Segmented, STATUS, StatusBadge, Widget } from '../sx';

const NODE_WIDTH = 240;
const NODE_HEIGHT = 76;
/** A sibling's slot across, and a generation's slot down. */
const COLUMN = NODE_WIDTH + 32;
const ROW = NODE_HEIGHT + 40;
/** Below this zoom the cards are hard to read: the view then centers on the message itself instead of fitting all. */
const READABLE_ZOOM = 0.8;
const PADDING = 48;

type MessageNodeData = { view: ChainNodeView; focus: boolean };
type MessageNode = Node<MessageNodeData, 'message'>;

/** What a card says under the actor: this message, a removed record, failures, or when it finished or is due. */
function note(n: ChainNodeView, focus: boolean): { text: string; className?: string } {
  if (focus) return { text: 'this message', className: 'cc-chain-here' };
  if (n.removed) return { text: 'record removed by its retention policy' };
  const time = shortDateTime(n.finishedAt ?? n.deliverAt);
  if (n.failures) return { text: `${time} · ${count(n.failures, 'failure')}`, className: 'cc-warning' };
  return { text: (n.finishedAt ? '' : 'due ') + time };
}

/** One message as a card: type and status, the actor it went to, and a note. */
function MessageCard({ data }: NodeProps<MessageNode>) {
  const { view: n, focus } = data;
  const line = note(n, focus);
  return (
    <div className={cx('cc-chain-card', focus && 'is-focus', n.removed && 'is-removed')}>
      <Handle type="target" position={Position.Top} isConnectable={false} />
      <span className="cc-row cc-gap-6">
        <b className="shape-title cc-trunc cc-grow" title={n.type}>{n.type}</b>
        {n.removed ? <Badge tone="outline" size="sm">Removed</Badge> : <StatusBadge status={n.status} size="sm" />}
      </span>
      <span className="mono-sm cc-muted cc-trunc" title={n.to}>{n.to}</span>
      <span className={cx('caption sx-num cc-trunc', line.className ?? 'cc-subtle')}>{line.text}</span>
      <Handle type="source" position={Position.Bottom} isConnectable={false} />
    </div>
  );
}

const nodeTypes = { message: MessageCard };

function toFlow(layout: ChainLayout, focus: string): { nodes: MessageNode[]; edges: Edge[] } {
  // The layout's depth goes down the page, its rows across.
  const nodes: MessageNode[] = layout.placed.map((p) => ({
    id: p.node.id,
    type: 'message',
    position: { x: p.row * COLUMN, y: p.depth * ROW },
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
    className: 'cc-chain-edge',
  }));
  return { nodes, edges };
}

/** Fits the whole chain when it stays readable; otherwise shows the message itself at a readable zoom. */
function initialView(instance: ReactFlowInstance<MessageNode>, container: HTMLDivElement | null, layout: ChainLayout, focus: string): void {
  const width = Math.max(1, layout.rows) * COLUMN - (COLUMN - NODE_WIDTH) + 2 * PADDING;
  const height = (layout.depth + 1) * ROW - (ROW - NODE_HEIGHT) + 2 * PADDING;
  const box = container?.getBoundingClientRect();
  const fits = box ? Math.min(box.width / width, box.height / height) >= READABLE_ZOOM : true;
  const target = layout.placed.find((p) => p.node.id === focus);
  if (fits || !target || !box) {
    void instance.fitView({ padding: 0.12, maxZoom: 1 });
    return;
  }
  // The generation of its cause at the top edge, the message itself centred across: its effects fill the rest.
  const zoom = 0.9;
  const top = Math.max(0, target.depth - 1) * ROW;
  void instance.setViewport({ x: box.width / 2 - (target.row * COLUMN + NODE_WIDTH / 2) * zoom, y: PADDING - top * zoom, zoom });
}

function Graph({ layout, focus }: { layout: ChainLayout; focus: string }) {
  const { nodes, edges } = useMemo(() => toFlow(layout, focus), [layout, focus]);
  const container = useRef<HTMLDivElement>(null);
  // Tall enough for the tree, within limits; the graph pans and zooms beyond that.
  const height = Math.min(640, Math.max(240, (layout.depth + 1) * ROW + 2 * PADDING));
  return (
    <>
      <div ref={container} className="cc-chain-graph" style={{ height }}>
        <ReactFlow
          nodes={nodes} edges={edges} nodeTypes={nodeTypes} onInit={(instance) => initialView(instance, container.current, layout, focus)}
          minZoom={0.2} maxZoom={1.5} nodesDraggable={false} nodesConnectable={false} elementsSelectable={false}
          proOptions={{ hideAttribution: true }} onNodeClick={(_, node) => { location.hash = href.message(node.id); }}
        >
          <Background variant={BackgroundVariant.Dots} gap={16} size={1} />
          <Controls showInteractive={false} />
          {layout.placed.length > 12 ? <MiniMap pannable zoomable nodeClassName={(n) => 'cc-chain-mini-' + STATUS[(n.data as MessageNodeData).view.status].tone} /> : null}
        </ReactFlow>
      </div>
      <div className="cc-chain-legend caption cc-subtle">
        <span>Click a card to open that message</span>
        <span className="sx-vr" aria-hidden="true" />
        <span>Thick outline: this message</span>
        <span className="sx-vr" aria-hidden="true" />
        <span>Dashed: removed record</span>
      </div>
    </>
  );
}

function List({ layout, focus }: { layout: ChainLayout; focus: string }) {
  return (
    <ol className="cc-chain-list" aria-label="Message chain">
      {layout.placed.map((p) => (
        <li key={p.node.id} className={cx('cc-chain-item', p.node.id === focus && 'is-focus')} style={{ paddingLeft: 16 + p.depth * 20 }}>
          {p.depth > 0 ? <span className="cc-subtle" aria-hidden="true"><Icon name="chevron-right" size={14} /></span> : null}
          <a className="label" href={href.message(p.node.id)}>{p.node.type}</a>
          <StatusBadge status={p.node.status} size="sm" />
          <span className="mono-sm cc-muted cc-trunc">{withoutScheme(p.node.to)}</span>
          <span className="caption sx-num cc-subtle">{shortDateTime(p.node.finishedAt ?? p.node.deliverAt)}</span>
          {p.node.removed ? <Badge tone="outline" size="sm">Removed</Badge> : null}
          {p.node.id === focus ? <span className="caption cc-chain-here">this message</span> : null}
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
      <Widget title="Message chain" subtitle="Sent from outside an actor · caused no messages">
        <p className="caption cc-muted cc-flat">This message was not sent by another message’s turn, and its own turn sent nothing (yet).</p>
      </Widget>
    );
  }
  return (
    <Widget
      title="Message chain"
      subtitle={`${count(chain.nodes.length, 'message')} · ${count(actors, 'actor')} · cause at the top, effects below`}
      actions={<Segmented label="Chain view" value={mode} onChange={setMode} icons={{ graph: 'flow-v', list: 'list' }}
        items={[{ value: 'graph', label: 'Graph' }, { value: 'list', label: 'List' }]} />}
      flush
    >
      {notes.length ? <p className="caption cc-muted cc-chain-notes">{notes.join(' ')}</p> : null}
      {mode === 'graph' ? <Graph layout={layout} focus={chain.focus} /> : <List layout={layout} focus={chain.focus} />}
    </Widget>
  );
}
