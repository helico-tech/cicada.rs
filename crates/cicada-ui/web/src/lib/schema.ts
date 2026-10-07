// Payload and state forms from the JSON Schemas served by GET /types (kotlinx-schema, draft 2020-12). The schemas are
// published for callers like this one; they do not validate anything. The node decodes with the serializer and is the
// only validator, so its 422 is the real answer and this module only catches what a form can see (a missing required
// field, a number that is not a number). Shapes a form cannot edit field by field (unions, maps, lists of objects,
// free JSON) get a JSON text box.

import { pointer } from './diff';

const MAX_DEPTH = 8;

type Schema = { [key: string]: unknown };

export interface Field {
  name: string;
  required: boolean;
  node: SchemaNode;
}

export type ScalarKind = 'string' | 'integer' | 'number' | 'boolean' | 'json';

export type SchemaNode =
  | { kind: 'object'; nullable: boolean; fields: Field[] }
  | { kind: ScalarKind; nullable: boolean }
  | { kind: 'enum'; nullable: boolean; options: string[] }
  | { kind: 'list'; nullable: boolean; item: SchemaNode };

export type ObjectNode = Extract<SchemaNode, { kind: 'object' }>;

/** What a form's inputs hold: text for scalars, a flag for booleans, lists and nested objects. */
export type Draft = string | boolean | Draft[] | { [key: string]: Draft };

export type DraftObject = { [key: string]: Draft };

const asSchema = (value: unknown): Schema => (value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Schema) : {});

function deref(schema: unknown, root: unknown): Schema {
  let s = asSchema(schema);
  for (let i = 0; i < MAX_DEPTH && typeof s.$ref === 'string'; i++) {
    const ref = s.$ref;
    if (!ref.startsWith('#/$defs/')) return {};
    const name = ref.slice('#/$defs/'.length).replace(/~1/g, '/').replace(/~0/g, '~');
    s = asSchema(asSchema(asSchema(root).$defs)[name]);
  }
  return s;
}

function typesOf(schema: Schema): { types: string[]; nullable: boolean } {
  const t = schema.type;
  const list = Array.isArray(t) ? t.filter((x): x is string => typeof x === 'string') : typeof t === 'string' ? [t] : [];
  return { types: list.filter((x) => x !== 'null'), nullable: list.includes('null') };
}

/** The editor for a schema, as a node. */
export function describe(schema: unknown, root: unknown = schema, depth = 0): SchemaNode {
  const s = deref(schema, root);
  const { types, nullable } = typesOf(s);
  if (Array.isArray(s.enum) && s.enum.length > 0 && s.enum.every((v) => typeof v === 'string')) {
    return { kind: 'enum', options: s.enum as string[], nullable };
  }
  const [type] = types;
  if (types.length === 1 && type !== undefined && depth < MAX_DEPTH) {
    if (type === 'string' || type === 'integer' || type === 'number' || type === 'boolean') return { kind: type, nullable };
    // A map has additionalProperties; a class has properties (or none, for an object without fields).
    if (type === 'object' && (s.additionalProperties === undefined || s.additionalProperties === false)) {
      const required = new Set(Array.isArray(s.required) ? s.required : []);
      const fields = Object.entries(asSchema(s.properties)).map(([name, p]) => ({
        name,
        required: required.has(name),
        node: describe(p, root, depth + 1),
      }));
      return { kind: 'object', fields, nullable };
    }
    if (type === 'array') {
      const item = describe(s.items ?? {}, root, depth + 1);
      if (['string', 'integer', 'number', 'enum'].includes(item.kind) && !item.nullable) return { kind: 'list', item, nullable };
    }
  }
  return { kind: 'json', nullable };
}

/** The type caption under a field: `string · enum`, `array<integer>`, `JSON · nullable`. */
export function hint(node: SchemaNode): string {
  const names: Record<string, string> = { object: 'object', string: 'string', integer: 'integer', number: 'number', boolean: 'boolean', enum: 'string · enum', json: 'JSON' };
  const base = node.kind === 'list' ? `array<${node.item.kind === 'enum' ? 'string' : node.item.kind}>` : names[node.kind] ?? node.kind;
  return node.nullable ? base + ' · nullable' : base;
}

/** A one-line summary of a message or state schema: `scope*, since, objects*` (`*` = required). */
export function summary(schema: unknown): string {
  const node = describe(schema);
  if (node.kind !== 'object') return hint(node);
  if (node.fields.length === 0) return '{} — no fields';
  return node.fields.map((f) => f.name + (f.required ? '*' : '')).join(', ');
}

/** The draft of an untouched form: what its inputs hold before anyone types. */
export function emptyDraft(node: SchemaNode): Draft {
  switch (node.kind) {
    case 'object':
      return Object.fromEntries(node.fields.map((f) => [f.name, emptyDraft(f.node)]));
    case 'boolean':
      return false;
    case 'list':
      return [];
    default:
      return '';
  }
}

const isPlainObject = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v);

/**
 * A draft holding [value], or null when the form cannot show it without losing something (an unknown key, a null
 * where the form has no null, a value of another shape). The caller then stays in raw JSON.
 */
export function draftOf(node: SchemaNode, value: unknown): Draft | null {
  if (value === undefined) return emptyDraft(node);
  switch (node.kind) {
    case 'object': {
      if (!isPlainObject(value)) return null;
      const known = new Set(node.fields.map((f) => f.name));
      if (Object.keys(value).some((k) => !known.has(k))) return null;
      const draft: DraftObject = {};
      for (const f of node.fields) {
        const d = draftOf(f.node, value[f.name]);
        if (d === null) return null;
        draft[f.name] = d;
      }
      return draft;
    }
    case 'string':
      if (value === null && node.nullable) return '';
      // An empty nullable string would come back as null.
      return typeof value === 'string' && !(value === '' && node.nullable) ? value : null;
    case 'integer':
      if (value === null && node.nullable) return '';
      return Number.isInteger(value) ? String(value) : null;
    case 'number':
      if (value === null && node.nullable) return '';
      return typeof value === 'number' ? String(value) : null;
    case 'boolean':
      return typeof value === 'boolean' ? value : null;
    case 'enum':
      if (value === null && node.nullable) return '';
      return typeof value === 'string' && node.options.includes(value) ? value : null;
    case 'list': {
      if (!Array.isArray(value)) return null;
      const items = value.map((v) => draftOf(node.item, v));
      return items.every((d) => d !== null && d !== '') ? (items as Draft[]) : null;
    }
    default:
      if (value === null && node.nullable) return '';
      return JSON.stringify(value, null, 2);
  }
}

const OMIT = Symbol('omit');

function isBlank(node: SchemaNode, draft: Draft | undefined): boolean {
  switch (node.kind) {
    case 'object':
      return node.fields.every((f) => isBlank(f.node, isPlainObject(draft) ? (draft as DraftObject)[f.name] : undefined));
    case 'boolean':
      return !draft;
    case 'list':
      return !Array.isArray(draft) || draft.length === 0;
    default:
      return draft === undefined || String(draft).trim() === '';
  }
}

export interface FieldError {
  path: string;
  message: string;
}

/**
 * Reads a draft into JSON: `{ value, errors }`, errors with a JSON Pointer. A blank optional field is left out (the
 * class's default applies); a blank required nullable field is null; a blank required string is the empty string.
 */
export function readDraft(node: SchemaNode, draft: Draft): { value: unknown; errors: FieldError[] } {
  const errors: FieldError[] = [];
  const value = read(node, draft, '', true, errors);
  return { value: value === OMIT ? {} : value, errors };
}

function read(node: SchemaNode, draft: Draft | undefined, path: string, required: boolean, errors: FieldError[]): unknown {
  const fail = (message: string): typeof OMIT => {
    errors.push({ path, message });
    return OMIT;
  };
  if (node.kind !== 'boolean' && node.kind !== 'string' && isBlank(node, draft) && !(node.kind === 'object' && required)) {
    if (!required) return OMIT;
    if (node.nullable) return null;
    if (node.kind === 'list') return [];
    return fail(node.kind === 'enum' ? 'Choose a value' : 'Required');
  }
  switch (node.kind) {
    case 'object': {
      const out: Record<string, unknown> = {};
      const fields = isPlainObject(draft) ? (draft as DraftObject) : {};
      for (const f of node.fields) {
        const v = read(f.node, fields[f.name], pointer(path, f.name), f.required, errors);
        if (v !== OMIT) out[f.name] = v;
      }
      return out;
    }
    case 'string':
      if (draft === undefined || draft === '') return !required ? OMIT : node.nullable ? null : '';
      return String(draft);
    case 'boolean':
      return Boolean(draft);
    case 'integer': {
      const text = String(draft).trim();
      if (!/^-?\d+$/.test(text)) return fail('Must be a whole number');
      const n = Number(text);
      return Number.isSafeInteger(n) ? n : fail('Too large for a JSON number');
    }
    case 'number': {
      const n = Number(String(draft).trim());
      return Number.isFinite(n) ? n : fail('Must be a number');
    }
    case 'enum':
      return typeof draft === 'string' && node.options.includes(draft) ? draft : fail('Choose one of the listed values');
    case 'list':
      return (Array.isArray(draft) ? draft : [])
        .map((item, i) => read(node.item, item, pointer(path, i), true, errors))
        .filter((v) => v !== OMIT);
    default:
      try {
        return JSON.parse(String(draft));
      } catch {
        return fail('Not valid JSON');
      }
  }
}

/** A starting value for raw JSON: the required fields with neutral values. */
export function skeleton(node: SchemaNode): unknown {
  if (node.nullable && node.kind !== 'object') return null;
  switch (node.kind) {
    case 'object':
      return Object.fromEntries(node.fields.filter((f) => f.required).map((f) => [f.name, skeleton(f.node)]));
    case 'string':
      return '';
    case 'integer':
    case 'number':
      return 0;
    case 'boolean':
      return false;
    case 'enum':
      return node.options[0];
    case 'list':
      return [];
    default:
      return null;
  }
}
