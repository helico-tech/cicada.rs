// What changed between two refreshes. The console polls; these diffs drive the "new", "changed" and "left"
// highlights. On the first load there is nothing to compare with, so nothing is highlighted.

/** Keys in [keys] that were not in [previous] (null on the first load). */
export function addedKeys(previous: ReadonlySet<string> | null, keys: readonly string[]): Set<string> {
  if (!previous) return new Set();
  return new Set(keys.filter((k) => !previous.has(k)));
}

/** Keys in [previous] that are no longer in [keys]. */
export function removedKeys(previous: ReadonlySet<string> | null, keys: readonly string[]): string[] {
  if (!previous) return [];
  const now = new Set(keys);
  return [...previous].filter((k) => !now.has(k));
}

/** A JSON Pointer (RFC 6901) for [key] below [path]. */
export function pointer(path: string, key: string | number): string {
  return path + '/' + String(key).replace(/~/g, '~0').replace(/\//g, '~1');
}

type Container = Record<string, unknown> | unknown[];

const isContainer = (v: unknown): v is Container => v !== null && typeof v === 'object';

const keysOf = (v: Container): string[] => Object.keys(v);
const member = (v: Container, key: string): unknown => (v as Record<string, unknown>)[key];

function sameJson(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!isContainer(a) || !isContainer(b) || Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = keysOf(a);
  const kb = keysOf(b);
  return ka.length === kb.length && ka.every((k) => Object.hasOwn(b, k) && sameJson(member(a, k), member(b, k)));
}

/** A changed place: `before` is undefined for an added key. */
export interface Change {
  before: unknown;
}

export interface JsonChanges {
  changed: Map<string, Change>;
  removed: number;
}

/**
 * The changed places between two JSON values: `changed` maps a JSON Pointer to `{ before }` and `removed` counts
 * keys that are gone. Containers are compared member by member, so a changed leaf is reported at the leaf. Null on
 * the first load ([before] undefined).
 */
export function jsonChanges(before: unknown, after: unknown): JsonChanges | null {
  if (before === undefined) return null;
  const changed = new Map<string, Change>();
  let removed = 0;
  const walk = (b: unknown, a: unknown, path: string): void => {
    if (isContainer(b) && isContainer(a) && Array.isArray(b) === Array.isArray(a)) {
      for (const key of keysOf(a)) {
        const p = pointer(path, key);
        if (!Object.hasOwn(b, key)) changed.set(p, { before: undefined });
        else walk(member(b, key), member(a, key), p);
      }
      removed += keysOf(b).filter((k) => !Object.hasOwn(a, k)).length;
    } else if (!sameJson(b, a)) {
      changed.set(path, { before: b });
    }
  };
  walk(before, after, '');
  return { changed, removed };
}
