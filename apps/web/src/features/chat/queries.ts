import { ApiRequestError, type components } from "@knoted/api-client";
import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query";
import { queryKeys, type QueryClientBundle } from "../../lib/query";

type Schemas = components["schemas"];
export type ChatSession = Schemas["ChatSession"];
export type ChatSessionDetail = Schemas["ChatSessionDetail"];
export type ChatMessage = Schemas["ChatMessage"];
export type ChatRun = Schemas["ChatRun"];
export type ChatScope = Schemas["ChatScope"];
export type ChatRequest = Schemas["ChatPostMessageRequest"];
export type Citation = Schemas["Citation"];

function result<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("聊天响应不完整。");
  return value;
}

export function sessionsQuery(bundle: QueryClientBundle) {
  return infiniteQueryOptions({
    queryKey: queryKeys.chat.sessions(),
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) => result((await bundle.api.raw.GET("/chat/sessions", {
      params: { query: { limit: 50, cursor: pageParam ?? undefined } },
      signal,
    })).data),
    getNextPageParam: (page) => page.next_cursor ?? undefined,
  });
}

export function sessionQuery(bundle: QueryClientBundle, sessionId: string) {
  return queryOptions({
    queryKey: queryKeys.chat.session(sessionId),
    staleTime: 0,
    queryFn: async ({ signal }) => {
      const session = result((await bundle.api.raw.GET("/chat/sessions/{sessionId}", {
        params: { path: { sessionId } },
        signal,
      })).data);
      if (session.id !== sessionId) throw new Error("会话身份不匹配。");
      return session;
    },
  });
}

export async function createSession(
  bundle: QueryClientBundle,
  body: Schemas["ChatSessionCreate"],
  signal: AbortSignal,
) {
  return result((await bundle.api.raw.POST("/chat/sessions", { body, signal })).data);
}

export async function postQuestion(
  bundle: QueryClientBundle,
  sessionId: string,
  body: ChatRequest,
  key: string,
  signal: AbortSignal,
) {
  const response = result((await bundle.api.raw.POST("/chat/sessions/{sessionId}/messages", {
    params: { path: { sessionId }, header: { "Idempotency-Key": key } },
    body,
    signal,
  })).data);
  if (response.message.session_id !== sessionId || response.run.session_id !== sessionId) {
    throw new Error("会话身份不匹配。");
  }
  return response;
}

export async function readRun(bundle: QueryClientBundle, sessionId: string, runId: string, signal: AbortSignal) {
  const run = result((await bundle.api.raw.GET("/chat/runs/{runId}", {
    params: { path: { runId } },
    signal,
  })).data);
  if (run.id !== runId || run.session_id !== sessionId) throw new Error("生成运行身份不匹配。");
  return run;
}

export function runQuery(bundle: QueryClientBundle, sessionId: string, runId: string | null) {
  return queryOptions({
    queryKey: ["chat", "run", sessionId, runId] as const,
    enabled: runId !== null,
    retry: false,
    queryFn: ({ signal }) => readRun(bundle, sessionId, result(runId ?? undefined), signal),
  });
}

export async function changeRun(
  bundle: QueryClientBundle,
  run: ChatRun,
  operation: "cancel" | "retry",
  signal: AbortSignal,
) {
  const options = { params: { path: { runId: run.id } }, signal };
  const response = operation === "cancel"
    ? await bundle.api.raw.POST("/chat/runs/{runId}/cancel", options)
    : await bundle.api.raw.POST("/chat/runs/{runId}/retry", options);
  const data = result<ChatRun>(response.data);
  if (data.session_id !== run.session_id || (operation === "cancel") !== (data.id === run.id)) {
    throw new Error("生成运行身份不匹配。");
  }
  return data;
}

export function citationsQuery(bundle: QueryClientBundle, messageId: string) {
  return infiniteQueryOptions({
    queryKey: ["chat", "citations", messageId] as const,
    staleTime: 0,
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) => {
      const page = result((await bundle.api.raw.GET("/chat/messages/{messageId}/citations", {
        params: { path: { messageId }, query: { limit: 50, cursor: pageParam ?? undefined } },
        signal,
      })).data);
      return { ...page, items: page.items.filter((citation) => citation.message_id === messageId) };
    },
    getNextPageParam: (page) => page.next_cursor ?? undefined,
  });
}

export function chatError(error: unknown, fallback: string) {
  if (error instanceof ApiRequestError) {
    if (error.status === 401) return "会话已失效，请重新登录。";
    const messages: Record<string, string> = {
      context_too_large: "问题或资料超过当前服务的上下文上限。",
      provider_unavailable: "所选服务暂时不可用。",
      validation_failed: "请检查问题或会话配置。",
      not_found: "会话或消息已不存在。",
      state_conflict: "运行状态已变化，请查询最新状态。",
    };
    if (Object.hasOwn(messages, error.code)) return messages[error.code]!;
  }
  return fallback;
}
