// The HTTP contract's wire types, kept by hand next to `cicada::api`. crates/cicada-ui/tests/api_fixtures.rs checks the
// API against them (CICADA_API_FIXTURES), so a change to the wire model lands here too.

import { z } from 'zod';

/** Any JSON value: payloads and states are opaque to the API. */
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
const json = z.json();

export const JsonElementSchema = json;
export type JsonElement = z.infer<typeof JsonElementSchema>;

export const CreateActorRequestSchema = z.object({
  state: JsonElementSchema.nullable().optional(),
});
export type CreateActorRequest = z.infer<typeof CreateActorRequestSchema>;

export const CreateActorResponseSchema = z.object({
  uri: z.string(),
  created: z.boolean(),
});
export type CreateActorResponse = z.infer<typeof CreateActorResponseSchema>;

export const ActorViewSchema = z.object({
  uri: z.string(),
  type: z.string(),
  params: z.record(z.string(), z.string()),
  state: JsonElementSchema.nullable(),
  version: z.number().int(),
  createdAt: z.string(),
});
export type ActorView = z.infer<typeof ActorViewSchema>;

export const ActorPageSchema = z.object({
  items: z.array(z.string()),
  next: z.string().nullable(),
});
export type ActorPage = z.infer<typeof ActorPageSchema>;

export const MessageIdSchema = z.string();
export type MessageId = z.infer<typeof MessageIdSchema>;

export const BackoffSchema = z.discriminatedUnion('kind', [
  z.object({
    kind: z.literal('exponential'),
    initial: z.union([z.string(), z.number()]),
    factor: z.number().optional(),
    max: z.union([z.string(), z.number()]),
  }),
  z.object({
    kind: z.literal('fixed'),
    delay: z.union([z.string(), z.number()]),
  }),
]);
export type Backoff = z.infer<typeof BackoffSchema>;

export const RetryPolicySchema = z.object({
  maxAttempts: z.number().int(),
  backoff: BackoffSchema,
});
export type RetryPolicy = z.infer<typeof RetryPolicySchema>;

export const KeepSchema = z.discriminatedUnion('kind', [
  z.object({
    kind: z.literal('for'),
    age: z.union([z.string(), z.number()]),
  }),
  z.object({
    kind: z.literal('last'),
    count: z.number().int(),
  }),
  z.object({
    kind: z.literal('remove'),
  }),
]);
export type Keep = z.infer<typeof KeepSchema>;

export const RetentionSchema = z.object({
  done: KeepSchema.nullable().optional(),
  failed: KeepSchema.nullable().optional(),
});
export type Retention = z.infer<typeof RetentionSchema>;

export const SendRequestSchema = z.object({
  id: MessageIdSchema.nullable().optional(),
  type: z.string(),
  payload: JsonElementSchema.optional(),
  deliverAt: z.string().nullable().optional(),
  delay: z.union([z.string(), z.number()]).nullable().optional(),
  correlationId: z.string().nullable().optional(),
  headers: z.record(z.string(), z.string()).optional(),
  retry: RetryPolicySchema.nullable().optional(),
  retention: RetentionSchema.nullable().optional(),
});
export type SendRequest = z.infer<typeof SendRequestSchema>;

export const SendResponseSchema = z.object({
  id: MessageIdSchema,
  deliverAt: z.string().nullable(),
});
export type SendResponse = z.infer<typeof SendResponseSchema>;

export const MessageViewSchema = z.object({
  id: MessageIdSchema,
  to: z.string(),
  from: z.string().nullable(),
  type: z.string(),
  payload: JsonElementSchema,
  deliverAt: z.string(),
  correlationId: z.string().nullable(),
  headers: z.record(z.string(), z.string()),
  seq: z.number().int(),
  attempt: z.number().int(),
  failures: z.number().int(),
  lastError: z.string().nullable(),
  retry: RetryPolicySchema.nullable(),
  causedBy: MessageIdSchema.nullable(),
  retention: RetentionSchema.nullable(),
});
export type MessageView = z.infer<typeof MessageViewSchema>;

export const MessagePageSchema = z.object({
  items: z.array(MessageViewSchema),
  next: z.string().nullable(),
});
export type MessagePage = z.infer<typeof MessagePageSchema>;

export const MessageStateSchema = z.enum(['pending', 'running', 'done', 'failed', 'cancelled', 'purged']);
export type MessageState = z.infer<typeof MessageStateSchema>;

export const MessageStatusViewSchema = z.object({
  id: MessageIdSchema,
  status: MessageStateSchema,
  error: z.string().nullable(),
  failures: z.number().int(),
  lastError: z.string().nullable(),
  finishedAt: z.string().nullable(),
  node: z.string().nullable(),
  leaseUntil: z.string().nullable(),
  message: MessageViewSchema,
});
export type MessageStatusView = z.infer<typeof MessageStatusViewSchema>;

export const OutcomeViewSchema = z.object({
  id: MessageIdSchema,
  actor: z.string(),
  status: MessageStateSchema,
  error: z.string().nullable(),
  finishedAt: z.string(),
  message: MessageViewSchema,
});
export type OutcomeView = z.infer<typeof OutcomeViewSchema>;

export const OutcomePageSchema = z.object({
  items: z.array(OutcomeViewSchema),
  next: z.string().nullable(),
});
export type OutcomePage = z.infer<typeof OutcomePageSchema>;

export const ChainNodeViewSchema = z.object({
  id: MessageIdSchema,
  causedBy: MessageIdSchema.nullable(),
  to: z.string(),
  from: z.string().nullable(),
  type: z.string(),
  status: MessageStateSchema,
  deliverAt: z.string(),
  finishedAt: z.string().nullable(),
  error: z.string().nullable(),
  attempt: z.number().int(),
  failures: z.number().int(),
  removed: z.boolean(),
});
export type ChainNodeView = z.infer<typeof ChainNodeViewSchema>;

export const MessageChainViewSchema = z.object({
  focus: MessageIdSchema,
  nodes: z.array(ChainNodeViewSchema),
  truncated: z.boolean(),
});
export type MessageChainView = z.infer<typeof MessageChainViewSchema>;

export const TypeViewSchema = z.object({
  template: z.string(),
  hash: z.string(),
  state: z.record(z.string(), json).nullable(),
  messages: z.record(z.string(), z.record(z.string(), json)),
});
export type TypeView = z.infer<typeof TypeViewSchema>;

export const TypeListViewSchema = z.object({
  items: z.array(TypeViewSchema),
});
export type TypeListView = z.infer<typeof TypeListViewSchema>;

export const TypeMatchViewSchema = z.object({
  type: TypeViewSchema,
  params: z.record(z.string(), z.string()),
});
export type TypeMatchView = z.infer<typeof TypeMatchViewSchema>;

export const HealthViewSchema = z.object({
  status: z.string(),
  node: z.string(),
  state: z.string(),
});
export type HealthView = z.infer<typeof HealthViewSchema>;

export const ProblemSchema = z.object({
  type: z.string(),
  title: z.string(),
  status: z.number().int(),
  detail: z.string().nullable(),
  instance: z.string().nullable(),
});
export type Problem = z.infer<typeof ProblemSchema>;

/** The problem `type` URIs of the API (`ProblemTypes`). */
export const ProblemTypes = {
  BAD_REQUEST: 'urn:cicada:problem:bad-request',
  INTERNAL_ERROR: 'urn:cicada:problem:internal-error',
  INVALID_ADDRESS: 'urn:cicada:problem:invalid-address',
  INVALID_HEADERS: 'urn:cicada:problem:invalid-headers',
  INVALID_PAYLOAD: 'urn:cicada:problem:invalid-payload',
  INVALID_STATE: 'urn:cicada:problem:invalid-state',
  MESSAGE_FINISHED: 'urn:cicada:problem:message-finished',
  MESSAGE_ID_CONFLICT: 'urn:cicada:problem:message-id-conflict',
  MESSAGE_RUNNING: 'urn:cicada:problem:message-running',
  METHOD_NOT_ALLOWED: 'urn:cicada:problem:method-not-allowed',
  NOT_FOUND: 'urn:cicada:problem:not-found',
  NO_SUCH_INSTANCE: 'urn:cicada:problem:no-such-instance',
  NO_SUCH_MESSAGE: 'urn:cicada:problem:no-such-message',
  NO_SUCH_TYPE: 'urn:cicada:problem:no-such-type',
  REQUEST_TOO_LARGE: 'urn:cicada:problem:request-too-large',
  STORE_UNAVAILABLE: 'urn:cicada:problem:store-unavailable',
  UNHEALTHY: 'urn:cicada:problem:unhealthy',
  UNKNOWN_MESSAGE_TYPE: 'urn:cicada:problem:unknown-message-type',
} as const;
