// Actor URIs and URI templates, as in ARCHITECTURE.md "Addressing": `actor://seg/seg`, segments percent-encoded
// outside A-Z a-z 0-9 . _ ~ -, templates with literal and {param} segments. Templates never overlap, so at most one
// matches a URI and the console can resolve types without asking the node.

export const SCHEME = 'actor://';

const UNRESERVED = /^[A-Za-z0-9._~-]$/;

/** Percent-encodes a raw segment value into its canonical form (as PercentEncoding.encode). */
export function encodeSegment(value: string): string {
  let out = '';
  for (const byte of new TextEncoder().encode(value)) {
    const c = String.fromCharCode(byte);
    out += byte < 0x80 && UNRESERVED.test(c) ? c : '%' + byte.toString(16).toUpperCase().padStart(2, '0');
  }
  return out;
}

/** Decodes a percent-encoded segment; a malformed one is shown as it is. */
export function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

export function isActorUri(text: unknown): text is string {
  return typeof text === 'string' && text.startsWith(SCHEME) && text.length > SCHEME.length;
}

/** The encoded segments of a URI or template. */
export function segmentsOf(uri: string): string[] {
  return uri.slice(SCHEME.length).split('/');
}

/** A URI or template without its scheme, for compact labels: `sync/{tenant}`. */
export function withoutScheme(uri: string): string {
  return uri.startsWith(SCHEME) ? uri.slice(SCHEME.length) : uri;
}

/**
 * The API path of an instance, relative to the API root (as ActorAddress.toHttpPath): `actors/a/b`. A last segment
 * that is literally `messages` is escaped, so it names the instance and not its mailbox.
 */
export function actorPath(uri: string): string {
  const segments = segmentsOf(uri).map((s) => (s === '.' ? '%2E' : s === '..' ? '%2E%2E' : s));
  if (segments[segments.length - 1] === 'messages') segments[segments.length - 1] = '%6Dessages';
  return 'actors/' + segments.join('/');
}

export type TemplatePart = { literal: string; param?: undefined } | { param: string; literal?: undefined };

/** A template's segments: `{ literal }` or `{ param }`. */
export function parseTemplate(template: string): TemplatePart[] {
  return segmentsOf(template).map((s) =>
    s.length > 2 && s.startsWith('{') && s.endsWith('}') ? { param: s.slice(1, -1) } : { literal: s });
}

export function templateParams(template: string): string[] {
  return parseTemplate(template).flatMap((s) => (s.param !== undefined ? [s.param] : []));
}

/** The decoded parameters of [uri] under [template], or null when it does not match. */
export function matchTemplate(template: string, uri: string): Record<string, string> | null {
  if (!isActorUri(uri)) return null;
  const parts = parseTemplate(template);
  const segments = segmentsOf(uri);
  if (parts.length !== segments.length) return null;
  const params: Record<string, string> = {};
  for (const [i, part] of parts.entries()) {
    const segment = segments[i] ?? '';
    if (part.param !== undefined) {
      if (segment === '') return null;
      params[part.param] = decodeSegment(segment);
    } else if (part.literal !== segment) {
      return null;
    }
  }
  return params;
}

/** The type (from GET /types) a URI belongs to, with its parameters; null when none matches. */
export function resolveType<T extends { template: string }>(types: readonly T[], uri: string): { type: T; params: Record<string, string> } | null {
  for (const type of types) {
    const params = matchTemplate(type.template, uri);
    if (params) return { type, params };
  }
  return null;
}

/**
 * The longest URI prefix every instance of [template] shares: its literal segments up to the first parameter, so
 * `actor://sync/{tenant}` gives `actor://sync/`. A template without parameters is its own prefix.
 */
export function templatePrefix(template: string): string {
  const literals: string[] = [];
  for (const part of parseTemplate(template)) {
    if (part.param !== undefined) return SCHEME + literals.map((l) => l + '/').join('');
    literals.push(part.literal);
  }
  return template;
}

/** Normalises a prefix filter as the API does: the scheme may be left out. */
export function normalizePrefix(prefix: string): string {
  const p = prefix.trim();
  return p === '' || p.startsWith(SCHEME) ? p : SCHEME + p;
}

/**
 * Combines two prefix filters into the one listing both select: the longer one when one extends the other, or
 * null when no URI can start with both.
 */
export function combinePrefixes(a: string, b: string): string | null {
  if (a.startsWith(b)) return a;
  if (b.startsWith(a)) return b;
  return null;
}

export interface UriPart {
  text: string;
  param: boolean;
  filled: boolean;
}

/**
 * Builds a URI from a template and raw parameter values. Returns the parts for display, the URI, and whether every
 * parameter has a value.
 */
export function buildUri(template: string, values: Record<string, string>): { parts: UriPart[]; uri: string; complete: boolean } {
  const parts = parseTemplate(template).map((part): UriPart => {
    if (part.param === undefined) return { text: part.literal, param: false, filled: true };
    const value = values[part.param] ?? '';
    return value === ''
      ? { text: '{' + part.param + '}', param: true, filled: false }
      : { text: encodeSegment(value), param: true, filled: true };
  });
  return {
    parts,
    uri: SCHEME + parts.map((p) => p.text).join('/'),
    complete: parts.every((p) => p.filled),
  };
}
