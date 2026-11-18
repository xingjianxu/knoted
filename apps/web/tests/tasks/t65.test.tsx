import { createApiClient, type components } from "@knoted/api-client";
import { QueryClient } from "@tanstack/react-query";
import { createMemoryHistory } from "@tanstack/react-router";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";

import App from "../../src/App";
import { createAppRouter } from "../../src/lib/router";
import { UiProvider } from "../../src/lib/ui-provider";

type Run = components["schemas"]["ChatRun"];
type Message = components["schemas"]["ChatMessage"];
type Session = components["schemas"]["ChatSessionDetail"];
type Event = components["schemas"]["ChatEvent"];
type Citation = components["schemas"]["Citation"];
type SearchHit = components["schemas"]["SearchHit"];

const sessionA = "76891566-0202-4033-88c7-0b88f225cded";
const sessionB = "5ac65ac9-5c1d-4b4e-8180-f9b5297a0d36";
const runID = "1d26efcf-58cc-42f3-9e6a-3a80be4b566c";
const newRunID = "9fa771ec-d182-4c12-a88e-f172ffa835bc";
const questionID = "9e3e841e-67de-43cd-8446-2ea89afa7a88";
const answerID = "9ad0e084-5816-47f4-8c3f-3ee93bf1bdc2";
const citationID = "781df448-05e8-4ccd-adaf-a4eb04329cb9";
const fixtureCsrf = "fixture-t65-csrf-value";
const timestamp = "2026-11-18T07:00:00Z";

const buildRun = (state: Run["state"], sessionId = sessionA) => ({
  id: runID,
  session_id: sessionId,
  message_id: questionID,
  state,
  partial: state === "failed" || state === "cancelled",
  error: state === "failed" ? "generation_interrupted" : null,
  created_at: timestamp,
  updated_at: timestamp,
}) satisfies Run;

const question: Message = {
  id: questionID,
  session_id: sessionA,
  role: "user",
  body: "这个项目有哪些风险？",
  run_id: null,
  incomplete: false,
  created_at: timestamp,
};
const answer: Message = {
  id: answerID,
  session_id: sessionA,
  role: "assistant",
  body: "历史回答正文",
  run_id: runID,
  incomplete: false,
  created_at: timestamp,
};
function session(id: string, messages: Message[] = []): Session {
  return { id, title: id === sessionA ? "甲会话" : "乙会话", provider: "local", version: 1, created_at: timestamp, updated_at: timestamp, messages };
}

function json(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
}

const encoder = new TextEncoder();
function sse(frame: Event) {
  return `event: ${frame.type}\r\ndata: ${JSON.stringify(frame)}\r\n\r\n`;
}
function responseStream(source: UnderlyingSource<Uint8Array>) {
  return new Response(new ReadableStream(source), { headers: { "content-type": "text/event-stream" } });
}
function closedStream(...frames: Event[]) {
  const raw = encoder.encode(frames.map(sse).join(""));
  return responseStream({
    start(controller) {
      for (let offset = 0; offset < raw.length; offset += 7) controller.enqueue(raw.slice(offset, offset + 7));
      controller.close();
    },
  });
}

async function openApp(
  handle: (path: string, request: Request) => Response | Promise<Response | undefined> | undefined,
  start = `/chat/${sessionA}`,
) {
  document.cookie = `knoted_csrf=${fixtureCsrf}; path=/`;
  const requests: Request[] = [];
  const fetcher: typeof globalThis.fetch = async (input) => {
    const request = input as Request;
    const path = new URL(request.url).pathname.replace(/^\/api\/v1/, "");
    if (path === "/auth/session") {
      return json({ session: { id: sessionA, user_id: sessionB, current: true }, csrf_token: fixtureCsrf });
    }
    requests.push(request);
    const response = await handle(path, request);
    if (response) return response;
    if (path === "/chat/sessions" && request.method === "GET") {
      return json({ items: [session(sessionA), session(sessionB)], next_cursor: null });
    }
    if (path === `/chat/sessions/${sessionA}`) return json(session(sessionA));
    if (path === `/chat/sessions/${sessionB}`) return json(session(sessionB));
    if (path === `/chat/sessions/${sessionA}/messages`) return json({ message: question, run: buildRun("queued") }, 201);
    if (path === "/tags" || path === "/projects") return json({ items: [], next_cursor: null });
    return json({ code: "not_found", message: "not found", request_id: sessionA }, 404);
  };
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
  const router = createAppRouter({
    history: createMemoryHistory({ initialEntries: [start] }),
    bundle: { queryClient, api: createApiClient({ baseUrl: "http://localhost/api/v1", fetch: fetcher }) },
  });
  const view = render(<UiProvider><App router={router} /></UiProvider>);
  await act(async () => router.load());
  return { requests, router, ...view };
}

async function sendQuestion() {
  fireEvent.change(await screen.findByRole("textbox", { name: "问题" }), { target: { value: question.body } });
  fireEvent.click(screen.getByRole("button", { name: "发送问题" }));
}

const toMessages = (request: Request) => new URL(request.url).pathname.endsWith("/messages");

afterEach(() => {
  document.cookie = "knoted_csrf=; Max-Age=0; path=/";
});

it("t65_1 retains partial text after disconnect and queries the run without resending", async () => {
  const view = await openApp((path) => {
    if (path === `/chat/runs/${runID}/events`) {
      return closedStream(
        { type: "queued", run_id: runID, seq: 1 },
        { type: "delta", run_id: runID, seq: 2, delta: "部分答复" },
      );
    }
    if (path === `/chat/runs/${runID}`) return json(buildRun("failed"));
    return undefined;
  });
  await sendQuestion();
  expect(await screen.findByText("部分答复")).toBeInTheDocument();
  expect(await screen.findByText("生成中断")).toBeInTheDocument();
  expect(screen.queryByText("已完成")).not.toBeInTheDocument();
  const stream = view.requests.find((request) => request.url.endsWith("/events"));
  expect(stream?.method).toBe("POST");
  expect(stream?.headers.get("x-csrf-token")).toBe(fixtureCsrf);
  expect(view.requests.some((request) => request.method === "GET" && request.url.endsWith(runID))).toBe(true);
  expect(view.requests.filter(toMessages)).toHaveLength(1);
});

it("keeps failure without an answer separate from partial output", async () => {
  await openApp((path) => {
    if (path !== `/chat/runs/${runID}/events`) return undefined;
    return closedStream({
      type: "failed",
      run_id: runID,
      seq: 1,
      error: { code: "context_too_large", message: "context too large", request_id: runID },
    });
  });
  await sendQuestion();
  expect(await screen.findByText("生成失败")).toBeInTheDocument();
  expect(screen.queryByText("未完成回答")).not.toBeInTheDocument();
});

it("shows the persisted answer when terminal validation changes the draft", async () => {
  let terminalSent = false;
  let publishHistory: ((response: Response) => void) | undefined;
  await openApp((path) => {
    if (path === `/chat/runs/${runID}/events`) {
      terminalSent = true;
      return closedStream(
        { type: "delta", run_id: runID, seq: 1, delta: "模型原始草稿 [cite:未知]" },
        { type: "completed", run_id: runID, seq: 2 },
      );
    }
    if (terminalSent && path === `/chat/sessions/${sessionA}`) {
      return new Promise<Response>((resolve) => { publishHistory = resolve; });
    }
    return undefined;
  });
  await sendQuestion();
  await screen.findByText("已完成");
  expect(screen.queryByText("模型原始草稿 [cite:未知]")).not.toBeInTheDocument();
  await waitFor(() => expect(publishHistory).toBeDefined());
  await act(async () => {
    publishHistory?.(json(session(sessionA, [question, { ...answer, body: "依据不足。" }])));
  });
  expect(await screen.findByText("依据不足。")).toBeInTheDocument();
});

it("t65_2 shows an immutable snapshot and deletion marker without linking to the source", async () => {
  const citation: Citation = {
    id: citationID,
    message_id: answerID,
    source_type: "note",
    source_id: questionID,
    title: "被删除的笔记",
    location: "第 2 段",
    snippet: "引用片段快照",
    source_state: "deleted",
  };
  const current: Citation = {
    ...citation,
    id: newRunID,
    title: "有效文件来源",
    source_type: "file",
    source_state: "current",
  };
  await openApp((path) => {
    if (path === `/chat/sessions/${sessionA}`) return json(session(sessionA, [question, answer]));
    if (path === `/chat/messages/${answerID}/citations`) return json({ items: [citation, current], next_cursor: null });
    return undefined;
  });
  fireEvent.click(await screen.findByRole("button", { name: "查看引用" }));
  expect(await screen.findByText("来源已删除")).toBeInTheDocument();
  expect(screen.getAllByText("引用片段快照")).toHaveLength(2);
  expect(screen.getAllByText("第 2 段")).toHaveLength(2);
  expect(screen.queryByRole("link", { name: "被删除的笔记" })).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "有效文件来源" })).toHaveAttribute("href", `/files/${questionID}`);
});

it("t65_3 aborts the old stream when navigating to a different session", async () => {
  let readerCancelled = false;
  const view = await openApp((path) => {
    if (path === `/chat/runs/${runID}/events`) {
      return responseStream({
        start(controller) {
          controller.enqueue(encoder.encode(sse({ type: "delta", run_id: runID, seq: 1, delta: "甲会话的流内容" })));
        },
        cancel() { readerCancelled = true; },
      });
    }
    return undefined;
  });
  await sendQuestion();
  await screen.findByText("甲会话的流内容");
  fireEvent.click(screen.getByRole("link", { name: "乙会话" }));
  expect(await screen.findByRole("heading", { name: "乙会话" })).toBeInTheDocument();
  await waitFor(() => expect(readerCancelled).toBe(true));
  const stream = view.requests.find((request) => request.url.endsWith("/events"));
  expect(stream?.signal.aborted).toBe(true);
  expect(screen.queryByText("甲会话的流内容")).not.toBeInTheDocument();
  expect(view.requests.filter(toMessages)).toHaveLength(1);
  expect(view.requests.some((request) => request.url.endsWith("/cancel"))).toBe(false);
});

it("shows a confirmed stop and retries the existing run only after explicit action", async () => {
  const view = await openApp((path) => {
    if (path === `/chat/runs/${runID}/events`) {
      return responseStream({
        start(controller) {
          controller.enqueue(encoder.encode(sse({ type: "delta", run_id: runID, seq: 1, delta: "可见草稿" })));
        },
      });
    }
    if (path === `/chat/runs/${runID}/cancel`) return json(buildRun("cancelled"));
    if (path === `/chat/runs/${runID}/retry`) return json({ ...buildRun("queued"), id: newRunID }, 201);
    if (path === `/chat/runs/${newRunID}/events`) return closedStream({ type: "completed", run_id: newRunID, seq: 1 });
    return undefined;
  });
  await sendQuestion();
  await screen.findByText("可见草稿");
  fireEvent.click(screen.getByRole("button", { name: "停止生成" }));
  expect(await screen.findByText("已停止")).toBeInTheDocument();
  expect(screen.getByText("可见草稿")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "重试生成" }));
  expect(await screen.findByText("已完成")).toBeInTheDocument();
  expect(view.requests.filter(toMessages)).toHaveLength(1);
  expect(view.requests.filter((request) => request.url.endsWith("/retry"))).toHaveLength(1);
});

it("opens a chat search hit at the message in its session", async () => {
  const view = await openApp((path) => {
    if (path === "/search") {
      return json({ items: [{
        id: answerID,
        entity_type: "chat",
        session_id: sessionA,
        title: "找到的聊天消息",
        snippet: "历史回答正文",
        updated_at: timestamp,
        path: null,
        score: null,
      } satisfies SearchHit], next_cursor: null });
    }
    if (path === `/chat/sessions/${sessionA}`) return json(session(sessionA, [question, answer]));
    return undefined;
  }, "/search?q=回答");
  fireEvent.click(await screen.findByRole("link", { name: "找到的聊天消息" }));
  await screen.findByText("历史回答正文");
  expect(view.router.state.location.pathname).toBe(`/chat/${sessionA}`);
  expect(view.router.state.location.search).toMatchObject({ message: answerID });
  const selected = view.container.querySelector(`[data-message-id="${answerID}"]`);
  expect(selected).toHaveFocus();
});

it("cancels the response body if an old session's stream headers arrive after navigation", async () => {
  let openStream: (() => void) | undefined;
  let released = false;
  const view = await openApp((path) => {
    if (path === `/chat/runs/${runID}/events`) {
      return new Promise((resolve) => {
        openStream = () => resolve(responseStream({ cancel() { released = true; } }));
      });
    }
    return undefined;
  });
  await sendQuestion();
  await waitFor(() => expect(openStream).toBeDefined());
  fireEvent.click(screen.getByRole("link", { name: "乙会话" }));
  await screen.findByRole("heading", { name: "乙会话" });
  await act(async () => openStream?.());
  await waitFor(() => expect(released).toBe(true));
  expect(view.requests.some((request) => request.method === "GET" && request.url.endsWith(runID))).toBe(false);
});

it("keeps a question draft and reuses its key only on explicit retry after a lost submit response", async () => {
  let submissions = 0;
  const view = await openApp((path) => {
    if (path === `/chat/sessions/${sessionA}/messages`) {
      submissions += 1;
      if (submissions === 1) return Promise.reject(new TypeError("fixture connection closed"));
      return json({ message: question, run: buildRun("queued") }, 201);
    }
    if (path === `/chat/runs/${runID}/events`) return closedStream({ type: "completed", run_id: runID, seq: 1 });
    return undefined;
  });
  await sendQuestion();
  expect(await screen.findByText("发送失败，请重试。")).toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "问题" })).toHaveValue(question.body);
  expect(view.requests.filter(toMessages)).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "发送问题" }));
  await screen.findByText("已完成");
  const writes = view.requests.filter(toMessages);
  expect(writes).toHaveLength(2);
  const key = writes[0]?.headers.get("idempotency-key");
  expect(key).toBeTruthy();
  expect(writes[1]?.headers.get("idempotency-key")).toBe(key);
});

it("creates a session with an explicit named external service and submits the selected source scope", async () => {
  const external = { ...session(sessionA), title: "外部会话", provider: "research" };
  const view = await openApp((path, request) => {
    if (path === "/chat/sessions" && request.method === "POST") return json(external, 201);
    if (path === `/chat/sessions/${sessionA}`) return json(external);
    if (path === `/chat/runs/${runID}/events`) return closedStream({ type: "completed", run_id: runID, seq: 1 });
    return undefined;
  }, "/chat");
  fireEvent.click(await screen.findByRole("button", { name: "新建会话" }));
  const dialog = await screen.findByRole("dialog", { name: "新建会话" });
  fireEvent.click(within(dialog).getByRole("combobox", { name: "生成服务" }));
  const externalOption = await screen.findByRole("option", { name: "外部服务" });
  fireEvent.pointerDown(externalOption, { pointerType: "mouse" });
  fireEvent.click(externalOption);
  fireEvent.change(screen.getByLabelText("会话名称"), { target: { value: external.title } });
  fireEvent.change(screen.getByLabelText("外部服务名"), { target: { value: external.provider } });
  fireEvent.click(screen.getByRole("button", { name: "创建会话" }));
  await screen.findByRole("heading", { name: external.title });
  const creation = view.requests.find((request) => request.method === "POST" && request.url.endsWith("/chat/sessions"));
  expect(await creation?.json()).toEqual({ title: external.title, provider: external.provider });
  expect(screen.getByText("问题和相关资料将发送给外部服务 research。")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("checkbox", { name: "笔记" }));
  await sendQuestion();
  await screen.findByText("已完成");
  const submission = view.requests.find(toMessages);
  expect(await submission?.json()).toEqual({ content: question.body, scope: { types: ["note"] } });
});

it("t66_5 saves only a complete answer and reuses the key on retry", async () => {
  const attempts: Array<{ key: string | null; body: unknown }> = [];
  const noteID = "d47ce8bd-1203-468f-832b-9e43509782d5";
  const partialID = "7af7728e-c8d1-4eb0-812d-72551251c552";
  const parentID = "738433a7-19bd-432a-8b16-699f2bde28a3";
  await openApp(async (path, request) => {
    if (path === `/chat/sessions/${sessionA}`) {
      return json(session(sessionA, [{ ...answer, id: partialID, body: "部分答复", incomplete: true }, question, answer]));
    }
    if (path === "/notes") return json({
      items: [{
        id: parentID, parent_id: null, title: "目标父页", document: { blocks: [] },
        format_version: 1, content_version: 1, position: 0, deleted_at: null,
        created_at: timestamp, updated_at: timestamp,
      }],
      next_cursor: null,
    });
    if (path === `/chat/messages/${answerID}/save-note`) {
      expect(request.headers.get("x-csrf-token")).toBe(fixtureCsrf);
      attempts.push({ key: request.headers.get("idempotency-key"), body: await request.json() });
      return attempts.length === 1
        ? json({ code: "dependency_unavailable", message: "fixture", request_id: runID }, 503)
        : json({ note_id: noteID }, 201);
    }
  });

  await screen.findByText(answer.body);
  const action = screen.getByRole("button", { name: "存为笔记" });
  expect(within(action.closest("li")!).getByText(answer.body)).toBeInTheDocument();
  fireEvent.click(action);
  const dialog = await screen.findByRole("dialog", { name: "存为笔记" });
  fireEvent.change(within(dialog).getByRole("textbox", { name: "标题" }), { target: { value: "项目风险" } });
  fireEvent.click(await within(dialog).findByRole("combobox", { name: "父页" }));
  const parentOption = await screen.findByRole("option", { name: "目标父页" });
  fireEvent.pointerDown(parentOption, { pointerType: "mouse" });
  fireEvent.click(parentOption);
  fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
  expect(await within(dialog).findByRole("alert")).toHaveTextContent("保存失败");
  fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
  expect(await screen.findByRole("link", { name: "打开笔记" })).toHaveAttribute("href", `/notes/${noteID}`);
  expect(attempts).toHaveLength(2);
  expect(attempts[0]?.key).toBeTruthy();
  expect(attempts[0]?.body).toEqual({ title: "项目风险", parent_id: parentID });
  expect(attempts[1]).toEqual(attempts[0]);
});

it("t66_6 can save to the root when the parent list cannot load", async () => {
  const noteID = "698c0bbc-e2df-4008-888f-d240ce890318";
  await openApp((path) => {
    if (path === `/chat/sessions/${sessionA}`) return json(session(sessionA, [question, answer]));
    if (path === "/notes") return json({ code: "dependency_unavailable", message: "fixture", request_id: runID }, 503);
    if (path === `/chat/messages/${answerID}/save-note`) return json({ note_id: noteID }, 201);
  });
  fireEvent.click(await screen.findByRole("button", { name: "存为笔记" }));
  const dialog = await screen.findByRole("dialog", { name: "存为笔记" });
  fireEvent.change(within(dialog).getByRole("textbox", { name: "标题" }), { target: { value: "主页笔记" } });
  expect(await within(dialog).findByRole("alert")).toHaveTextContent("无法读取笔记目录");
  fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
  expect(await screen.findByRole("link", { name: "打开笔记" })).toHaveAttribute("href", `/notes/${noteID}`);
});
