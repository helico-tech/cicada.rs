// Example 20's browser client: the <script> of examples/examples/websocket_session.html, run unchanged in a node:vm
// context with controlled fakes for WebSocket, localStorage, setTimeout, location and the page. No real browser runs.
// Run with `node --test examples/tests/websocket_session_client.test.mjs` (Node 26 or later, no packages).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const html = readFileSync(new URL('../examples/websocket_session.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*)<\/script>/)[1];
const STREAM = 's-1';
const KEY = `cicada-session-cursor:${STREAM}`;

/** Runs the client against fakes; `stored` is the localStorage content before the page loads. */
function load(stored = {}) {
  const sockets = [];
  const timers = [];
  const storage = new Map(Object.entries(stored));
  const log = { textContent: '' };
  class FakeWebSocket {
    constructor(url) {
      this.url = url;
      this.sent = [];
      this.closed = false;
      sockets.push(this);
    }
    send(text) {
      assert.equal(this.closed, false, 'no send on a closed socket');
      this.sent.push(JSON.parse(text));
    }
    close() {
      this.closed = true;
    }
    /** Delivers a server message, as the browser calls onmessage. */
    deliver(message) {
      this.onmessage({ data: JSON.stringify(message) });
    }
    /** The browser's close event, after a close by either side. */
    fireClose() {
      this.closed = true;
      this.onclose({});
    }
  }
  const context = {
    WebSocket: FakeWebSocket,
    localStorage: {
      getItem: (key) => (storage.has(key) ? storage.get(key) : null),
      setItem: (key, value) => storage.set(key, String(value)),
    },
    setTimeout: (callback, delay) => timers.push({ callback, delay }),
    location: { search: `?stream=${STREAM}`, host: 'localhost:3000' },
    document: { getElementById: (id) => (assert.equal(id, 'log'), log) },
    encodeURIComponent,
    URLSearchParams,
    JSON,
    Number,
    String,
  };
  vm.runInNewContext(script, context);
  return {
    sockets,
    timers,
    storage,
    log,
    /** The newest socket. */
    get socket() {
      return sockets.at(-1);
    },
    /** Runs the timers that are due, as the browser would after their delay. */
    runTimers() {
      const due = timers.splice(0);
      for (const { callback } of due) callback();
      return due.length;
    },
  };
}

const hello = (acked, stream = STREAM) => ({ type: 'Hello', stream, acked });
const frame = (seq, stream = STREAM) => ({ type: 'Frame', stream, seq, body: `f${seq}` });
const ack = (through) => ({ type: 'Ack', stream: STREAM, through });
const applied = (page) => page.log.textContent.split('\n').filter((line) => /^\d+: /.test(line));

test('it connects to the session route of the stream', () => {
  const page = load();
  assert.equal(page.sockets.length, 1);
  assert.equal(page.socket.url, `ws://localhost:3000/sessions/${STREAM}/socket`);
});

test('only the next contiguous frame is applied, and the cursor is kept per stream', () => {
  const page = load({ 'cicada-session-cursor:other': '9' });
  page.socket.deliver(hello(0));
  assert.deepEqual(page.socket.sent, [], 'nothing applied yet, nothing to acknowledge');
  page.socket.deliver(frame(1));
  page.socket.deliver(frame(2));
  assert.deepEqual(applied(page), ['1: f1', '2: f2']);
  assert.deepEqual(page.socket.sent, [ack(1), ack(2)]);
  assert.equal(page.storage.get(KEY), '2');
  assert.equal(page.storage.get('cicada-session-cursor:other'), '9', "another stream's cursor is untouched");

  const reloaded = load(Object.fromEntries(page.storage));
  reloaded.socket.deliver(hello(2));
  reloaded.socket.deliver(frame(3));
  assert.deepEqual(applied(reloaded), ['3: f3'], 'a reload continues from the stored cursor');
});

test('a duplicate frame is not applied again and is acknowledged again', () => {
  const page = load({ [KEY]: '2' });
  page.socket.deliver(hello(1));
  page.socket.deliver(frame(2));
  page.socket.deliver(frame(1));
  assert.deepEqual(applied(page), []);
  assert.deepEqual(page.socket.sent, [ack(2), ack(2), ack(2)], 'Hello, then each replay, re-acknowledge the cursor');
  assert.equal(page.storage.get(KEY), '2');
});

test('a gap closes the socket and reconnects, never skipping ahead', () => {
  const page = load({ [KEY]: '1' });
  const first = page.socket;
  first.deliver(hello(1));
  first.deliver(frame(3));
  assert.deepEqual(applied(page), []);
  assert.equal(page.storage.get(KEY), '1');
  assert.equal(first.closed, true);
  first.fireClose();
  assert.equal(page.timers.length, 1, 'one reconnect, not a second one from the close event');
  assert.equal(page.timers[0].delay, 1000);
  page.runTimers();
  assert.equal(page.sockets.length, 2);
  page.socket.deliver(hello(1));
  page.socket.deliver(frame(2));
  assert.deepEqual(applied(page), ['2: f2']);
});

test('callbacks of an old socket are ignored after a reconnect', () => {
  const page = load();
  const old = page.socket;
  old.deliver(hello(0));
  old.deliver(frame(2)); // a gap: reconnect
  page.runTimers();
  const current = page.socket;
  assert.notEqual(current, old);
  old.deliver(frame(1));
  old.fireClose();
  assert.deepEqual(applied(page), []);
  assert.deepEqual(old.sent, []);
  assert.equal(page.timers.length, 0, "the old socket's close schedules nothing");
  current.deliver(hello(0));
  current.deliver(frame(1));
  assert.deepEqual(applied(page), ['1: f1']);
  assert.deepEqual(current.sent, [ack(1)]);
});

test('a Hello ahead of a lost cursor stops with the recovery message, without acknowledging', () => {
  const page = load({ [KEY]: '2' });
  page.socket.deliver(hello(5));
  assert.deepEqual(page.socket.sent, []);
  assert.equal(page.socket.closed, true);
  assert.match(page.log.textContent, /cursor 2 is behind the acknowledged 5: recover, or start a new session/);
  page.socket.fireClose();
  assert.equal(page.timers.length, 0, 'no reconnect');
  assert.equal(page.storage.get(KEY), '2');
});

test('on Hello, a cursor at or above the acknowledgement is acknowledged again', () => {
  for (const [cursor, acked] of [['3', 3], ['4', 3]]) {
    const page = load({ [KEY]: cursor });
    page.socket.deliver(hello(acked));
    assert.deepEqual(page.socket.sent, [ack(Number(cursor))], `cursor ${cursor}, acked ${acked}`);
    assert.equal(page.socket.closed, false);
  }
  const fresh = load();
  fresh.socket.deliver(hello(0));
  assert.deepEqual(fresh.socket.sent, [], 'a fresh client has nothing to acknowledge');
});

test('messages for another stream are ignored', () => {
  const page = load();
  page.socket.deliver(hello(7, 'other'));
  page.socket.deliver(frame(1, 'other'));
  assert.deepEqual(applied(page), []);
  assert.deepEqual(page.socket.sent, []);
  assert.equal(page.socket.closed, false);
  assert.equal(page.storage.get(KEY), undefined);
});

test('a lost connection reconnects after a second', () => {
  const page = load();
  page.socket.deliver(hello(0));
  page.socket.fireClose();
  assert.equal(page.timers.length, 1);
  assert.equal(page.timers[0].delay, 1000);
  page.runTimers();
  assert.equal(page.sockets.length, 2);
});
