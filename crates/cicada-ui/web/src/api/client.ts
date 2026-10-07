// Client for the fourteen actorApi() endpoints (ARCHITECTURE.md "HTTP contract"). Every response body is parsed with
// the validator generated from the Kotlin wire classes (generated.ts), so the console never works on a shape the node
// did not send. Errors are thrown as ApiError, built from the problem+json body when there is one.

import type { z } from 'zod';
import { actorPath } from '../lib/address';
import {
  ActorPageSchema,
  ActorViewSchema,
  CreateActorResponseSchema,
  HealthViewSchema,
  MessageChainViewSchema,
  MessagePageSchema,
  MessageStatusViewSchema,
  OutcomePageSchema,
  ProblemSchema,
  SendResponseSchema,
  TypeListViewSchema,
  TypeMatchViewSchema,
  type CreateActorRequest,
  type Json,
  type SendRequest,
} from './generated';

const STATUS_TEXT: Record<number, string> = {
  400: 'Bad Request',
  401: 'Unauthorized',
  403: 'Forbidden',
  404: 'Not Found',
  405: 'Method Not Allowed',
  409: 'Conflict',
  413: 'Content Too Large',
  422: 'Unprocessable Content',
  500: 'Internal Server Error',
  502: 'Bad Gateway',
  503: 'Service Unavailable',
  504: 'Gateway Timeout',
};

interface ProblemLike {
  type?: string | null;
  title?: string | null;
  detail?: string | null;
}

/** A failed request: HTTP status (0 when no response came) and the problem's type, title and detail. */
export class ApiError extends Error {
  readonly status: number;
  readonly type: string;
  readonly title: string;
  readonly detail: string;

  constructor(status: number, problem: ProblemLike) {
    super(problem.detail || problem.title || `HTTP ${status}`);
    this.name = 'ApiError';
    this.status = status;
    this.type = problem.type ?? '';
    this.title = problem.title || STATUS_TEXT[status] || 'Request failed';
    this.detail = problem.detail ?? '';
  }

  /** Worth retrying as it is: the store or the network was unavailable, nothing was changed. */
  get temporary(): boolean {
    return this.status === 0 || this.status === 502 || this.status === 503 || this.status === 504;
  }

  /** `409 Conflict`; `No response` when the request did not reach the node. */
  get code(): string {
    return this.status === 0 ? 'No response' : `${this.status} ${STATUS_TEXT[this.status] ?? ''}`.trim();
  }
}

/** A response whose body does not match the generated validator: the node and the console disagree on the contract. */
export class ContractError extends ApiError {
  constructor(path: string, issues: string) {
    super(500, { title: 'Unexpected response', detail: `The response of ${path} does not match the console's API types: ${issues}` });
    this.name = 'ContractError';
  }
}

type Query = Record<string, string | number | null | undefined>;

export interface ApiOptions {
  /** The API's absolute base path, such as `/` or `/api`. */
  base: string;
  fetch?: typeof globalThis.fetch;
  /** The bearer token to send, if any. */
  token?: () => string | null;
}

/** A write whose status says what happened (created or existing, accepted or duplicate). */
export interface Written<T> {
  status: number;
  body: T;
}

export function createApi({ base, fetch: doFetch = (...args) => globalThis.fetch(...args), token = () => null }: ApiOptions) {
  const root = base.endsWith('/') ? base : base + '/';

  async function request(method: string, path: string, { query, body }: { query?: Query; body?: unknown } = {}): Promise<{ status: number; json: unknown }> {
    let url = root + path;
    if (query) {
      const params = new URLSearchParams();
      for (const [k, v] of Object.entries(query)) if (v !== undefined && v !== null && v !== '') params.set(k, String(v));
      const q = params.toString();
      if (q) url += '?' + q;
    }
    const headers: Record<string, string> = { Accept: 'application/json, application/problem+json' };
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    const bearer = token();
    if (bearer) headers.Authorization = 'Bearer ' + bearer;
    let response: Response;
    try {
      response = await doFetch(url, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        credentials: 'same-origin',
        cache: 'no-store',
      });
    } catch (e) {
      throw new ApiError(0, { title: 'Node unreachable', detail: `The request did not reach the node (${e instanceof Error ? e.message : String(e)}).` });
    }
    const text = await response.text();
    let json: unknown = null;
    if (text) {
      try {
        json = JSON.parse(text);
      } catch {
        json = null;
      }
    }
    if (!response.ok) {
      const problem = ProblemSchema.safeParse(json);
      throw new ApiError(response.status, problem.success ? problem.data : { detail: text.slice(0, 300) });
    }
    return { status: response.status, json };
  }

  function parse<S extends z.ZodType>(schema: S, path: string, json: unknown): z.infer<S> {
    const result = schema.safeParse(json);
    if (!result.success) {
      throw new ContractError(path, result.error.issues.map((i) => `${i.path.join('.') || '(body)'} ${i.message}`).join('; '));
    }
    return result.data;
  }

  const get = async <S extends z.ZodType>(schema: S, path: string, query?: Query): Promise<z.infer<S>> =>
    parse(schema, path, (await request('GET', path, { query })).json);

  const write = async <S extends z.ZodType>(schema: S, method: string, path: string, body?: unknown): Promise<Written<z.infer<S>>> => {
    const r = await request(method, path, { body });
    return { status: r.status, body: parse(schema, path, r.json) };
  };

  const messagePath = (id: string): string => 'messages/' + encodeURIComponent(id);

  return {
    listActors: ({ prefix, cursor, limit }: { prefix?: string; cursor?: string | null; limit?: number } = {}) =>
      get(ActorPageSchema, 'actors', { prefix, cursor, limit }),
    getActor: (uri: string) => get(ActorViewSchema, actorPath(uri)),
    createActor: (uri: string, state: Json | undefined) => {
      const body: CreateActorRequest = state === undefined ? {} : { state };
      return write(CreateActorResponseSchema, 'PUT', actorPath(uri), body);
    },
    deleteActor: (uri: string) => request('DELETE', actorPath(uri)).then((r) => r.status),
    pending: (uri: string, { cursor, limit }: { cursor?: string | null; limit?: number } = {}) =>
      get(MessagePageSchema, actorPath(uri) + '/messages', { cursor, limit }),
    send: (uri: string, message: SendRequest) => write(SendResponseSchema, 'POST', actorPath(uri) + '/messages', message),
    listPending: ({ prefix, cursor, limit }: { prefix?: string; cursor?: string | null; limit?: number } = {}) =>
      get(MessagePageSchema, 'messages', { prefix, cursor, limit }),
    message: (id: string) => get(MessageStatusViewSchema, messagePath(id)),
    chain: (id: string) => get(MessageChainViewSchema, messagePath(id) + '/chain'),
    cancel: (id: string) => request('DELETE', messagePath(id)).then((r) => r.status),
    failures: ({ prefix, since, cursor, limit }: { prefix?: string; since?: string; cursor?: string | null; limit?: number } = {}) =>
      get(OutcomePageSchema, 'failures', { prefix, since, cursor, limit }),
    types: () => get(TypeListViewSchema, 'types'),
    typeOf: (uri: string) => get(TypeMatchViewSchema, 'types', { uri }),
    health: () => get(HealthViewSchema, 'health'),
  };
}

export type Api = ReturnType<typeof createApi>;

/**
 * Follows a paged listing: calls [fetchPage] with each cursor until the last page or [maxPages]. Returns the items
 * and whether pages were left (`truncated`).
 */
export async function fetchPages<T>(
  fetchPage: (cursor: string | null) => Promise<{ items: T[]; next: string | null }>,
  { maxPages }: { maxPages: number },
): Promise<{ items: T[]; truncated: boolean; next: string | null }> {
  const items: T[] = [];
  let cursor: string | null = null;
  for (let page = 0; page < maxPages; page++) {
    const result = await fetchPage(cursor);
    items.push(...result.items);
    cursor = result.next;
    if (!cursor) return { items, truncated: false, next: null };
  }
  return { items, truncated: true, next: cursor };
}
