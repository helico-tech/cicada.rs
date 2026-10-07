// The console's pages live in the URL fragment, so the server serves one page and links survive reloads:
//   #/actors?type=&prefix=   #/actor/<uri>   #/message/<id>   #/pending?type=&prefix=   #/failures?prefix=
//   #/types?template=

export type Query = Record<string, string>;

export type Route =
  | { name: 'actor'; uri: string }
  | { name: 'message'; id: string }
  | { name: 'actors' | 'pending' | 'failures' | 'types'; query: Query };

/** The route of a location hash. Unknown hashes go to the actor list. */
export function parseRoute(hash: string | null | undefined): Route {
  const raw = (hash ?? '').replace(/^#\/?/, '');
  const [path = '', queryString = ''] = raw.split('?', 2);
  const query: Query = Object.fromEntries(new URLSearchParams(queryString));
  const [name, ...rest] = path.split('/');
  const arg = rest.length ? safeDecode(rest.join('/')) : '';
  switch (name) {
    case 'actor':
      return arg ? { name: 'actor', uri: arg } : { name: 'actors', query: {} };
    case 'message':
      return arg ? { name: 'message', id: arg } : { name: 'actors', query: {} };
    case 'pending':
    case 'failures':
    case 'types':
    case 'actors':
      return { name, query };
    default:
      return { name: 'actors', query: {} };
  }
}

function safeDecode(text: string): string {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

function withQuery(path: string, query: Record<string, string | null | undefined> | undefined): string {
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(query ?? {})) if (v) params.set(k, v);
  const q = params.toString();
  return q ? `${path}?${q}` : path;
}

type QueryArg = Record<string, string | null | undefined>;

export const href = {
  actors: (query?: QueryArg): string => withQuery('#/actors', query),
  actor: (uri: string): string => '#/actor/' + encodeURIComponent(uri),
  message: (id: string): string => '#/message/' + encodeURIComponent(id),
  pending: (query?: QueryArg): string => withQuery('#/pending', query),
  failures: (query?: QueryArg): string => withQuery('#/failures', query),
  types: (query?: QueryArg): string => withQuery('#/types', query),
};

/** Changes the URL's query to match a page's filters without opening the page again. */
export function replaceHash(target: string): void {
  history.replaceState(null, '', target);
}
