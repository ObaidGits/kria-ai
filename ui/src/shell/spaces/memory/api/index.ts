/**
 * memory/api — public barrel for Memory API v2 client types.
 *
 * Import from this module to access the client, DTOs, error types, and
 * request options without depending on file-internal paths.
 *
 * Usage:
 *   import { MemoryApiClient, UnsupportedCapabilityError } from "../api";
 *   import type { GraphResponseV2, RequestOptions } from "../api";
 */

export {
  DEFAULT_DEADLINE_MS,
  UnsupportedCapabilityError,
  MemoryApiClient,
} from "./client";

export type {
  TotalSemantics,
  ApiWarning,
  DegradationInfo,
  GraphResponseV2,
  RequestOptions,
} from "./client";

export { parseKnowledgeProjectionResponse } from "./knowledgeProjection";
export type {
  KnowledgeProjectionAuthority,
  KnowledgeProjectionDirection,
  KnowledgeProjectionItem,
  KnowledgeProjectionItemKind,
  KnowledgeProjectionParseResult,
  KnowledgeProjectionResponse,
} from "./knowledgeProjection";
