import { type components, createApiClient } from "@knoted/api-client";
import { QueryClient } from "@tanstack/react-query";
import { createMemoryHistory } from "@tanstack/react-router";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it } from "vitest";

import App from "../../src/App";
import { createAppRouter } from "../../src/lib/router";
import { UiProvider } from "../../src/lib/ui-provider";

type SearchHit = components["schemas"]["SearchHit"];
type SearchResponse = components["schemas"]["SearchResponse"];

const id = "00000000-0000-4000-8000-000000000001";
const tag = "00000000-0000-4000-8000-000000000002";
const timestamp = "2026-11-18T02:30:00.000Z";
const hit: SearchHit = {
  id,
  entity_type: "file",
  title: "会议纪要",
  snippet: "这份纪要记录了项目讨论。",
  path: "工作/会议.txt",
  updated_at: timestamp,
  score: null,
};

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

async function openSearch(url: string, search: (params: URLSearchParams) => Response | Promise<Response>) {
  const requests: URLSearchParams[] = [];
  const fetcher: typeof globalThis.fetch = async (input) => {
    const { pathname, searchParams } = new URL((input as Request).url);
    if (pathname.endsWith("/auth/session")) {
      return json({ session: { id, user_id: id, current: true }, csrf_token: "test-csrf-value-long" });
    }
    if (pathname.endsWith("/search")) {
      requests.push(searchParams);
      return search(searchParams);
    }
    if (pathname.endsWith("/projects") || pathname.endsWith("/tags")) {
      return json({ items: [], next_cursor: null });
    }
    return json({}, 404);
  };
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
  const router = createAppRouter({
    history: createMemoryHistory({ initialEntries: [url] }),
    bundle: { queryClient, api: createApiClient({ baseUrl: "http://localhost/api/v1", fetch: fetcher }) },
  });
  const view = render(<UiProvider><App router={router} /></UiProvider>);
  await act(async () => router.load());
  return { requests, router, ...view };
}

const page = (items: SearchHit[], next_cursor: string | null = null) =>
  json({ items, next_cursor } satisfies SearchResponse);

it("t55_1 labels a chat hit and shows when its exact location is unavailable", async () => {
  await openSearch("/search?q=纪要", () => page([{ ...hit, entity_type: "chat", path: null }]));
  expect(await screen.findByText("聊天消息", { exact: true })).toBeInTheDocument();
  expect(screen.getByText("无法定位到原文位置。")).toBeInTheDocument();
});

it("t55_2 restores every submitted filter when the URL is reopened", async () => {
  const params = new URLSearchParams({
    q: "纪要",
    type: "file",
    tags: tag,
    project: id,
    updated_from: timestamp,
    updated_to: "2026-11-19T03:00:00.000Z",
    archived: "false",
    path: "工作/会议",
    format: "txt",
    sort: "updated_desc",
  });
  const original = await openSearch(`/search?${params}`, () => page([hit]));
  expect(await screen.findByRole("link", { name: "会议纪要" })).toHaveAttribute("href", `/files/${id}`);
  fireEvent.change(screen.getByLabelText("路径"), { target: { value: "项目/文件" } });
  fireEvent.click(within(screen.getByRole("search")).getByRole("button", { name: "搜索" }));
  await waitFor(() => expect(original.requests.at(-1)?.get("path")).toBe("项目/文件"));
  const url = original.router.state.location.href;
  original.unmount();
  const reopened = await openSearch(url, () => page([hit]));
  expect(await screen.findByRole("link", { name: "会议纪要" })).toBeInTheDocument();
  expect(screen.getByLabelText("路径")).toHaveValue("项目/文件");
  expect(screen.getByLabelText("格式")).toHaveValue("txt");
  params.set("path", "项目/文件");
  for (const [key, value] of params) expect(reopened.requests.at(-1)?.get(key)).toBe(value);
  expect(reopened.requests.at(-1)?.has("cursor")).toBe(false);
});

it("t55_3 highlights matching text without interpreting HTML", async () => {
  const snippet = '<img src=x onerror="alert(1)"> 查找 <em>纯文本</em>';
  const view = await openSearch("/search?q=查找", () => page([{ ...hit, snippet }]));
  await screen.findByRole("link", { name: "会议纪要" });
  const item = screen.getByRole("listitem");
  expect(item.textContent).toContain(snippet);
  expect(item.querySelector("mark")).toHaveTextContent("查找");
  expect(view.container.querySelector("img, em")).toBeNull();
});

it("disables cursor controls while loading and resets them when the URL query changes", async () => {
  let resolvePage: (response: Response) => void = () => {};
  const view = await openSearch("/search?q=纪要", (params) => {
    if (params.get("cursor") === "p2") return new Promise((resolve) => { resolvePage = resolve; });
    return page([hit], "p2");
  });
  await screen.findByRole("link", { name: "会议纪要" });
  fireEvent.click(screen.getByRole("button", { name: "下一页" }));
  await waitFor(() => expect(view.requests.at(-1)?.get("cursor")).toBe("p2"));
  expect(screen.getByRole("button", { name: "上一页" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "下一页" })).toBeDisabled();
  await act(async () => resolvePage(page([{ ...hit, title: "下一页结果" }])));
  await screen.findByRole("link", { name: "下一页结果" });
  fireEvent.click(screen.getByRole("button", { name: "上一页" }));
  await screen.findByRole("link", { name: "会议纪要" });
  fireEvent.click(screen.getByRole("button", { name: "下一页" }));
  await waitFor(() => expect(view.requests.filter((params) => params.get("cursor") === "p2")).toHaveLength(2));
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "新问题" } });
  fireEvent.click(within(screen.getByRole("search")).getByRole("button", { name: "搜索" }));
  await waitFor(() => expect(view.requests.at(-1)?.get("q")).toBe("新问题"));
  expect(view.requests.at(-1)?.has("cursor")).toBe(false);
  await screen.findByRole("link", { name: "会议纪要" });
  expect(screen.getByRole("button", { name: "上一页" })).toBeDisabled();
  await act(async () => resolvePage(page([{ ...hit, title: "下一页结果" }])));
  expect(screen.queryByRole("link", { name: "下一页结果" })).not.toBeInTheDocument();
});

it("can retry a failed result request", async () => {
  let unavailable = true;
  await openSearch("/search?q=纪要", () => unavailable
    ? json({ code: "dependency_unavailable", message: "unavailable", request_id: id }, 503)
    : page([hit]));
  const retry = await screen.findByRole("button", { name: "重试搜索" });
  unavailable = false;
  fireEvent.click(retry);
  expect(await screen.findByRole("link", { name: "会议纪要" })).toBeInTheDocument();
});

it("submitting the same search refreshes the first page", async () => {
  let revision = "会议纪要";
  await openSearch("/search?q=纪要", (params) => params.has("cursor")
    ? page([{ ...hit, title: "下一页结果" }])
    : page([{ ...hit, title: revision }], "p2"));
  await screen.findByRole("link", { name: "会议纪要" });
  fireEvent.click(screen.getByRole("button", { name: "下一页" }));
  await screen.findByRole("link", { name: "下一页结果" });
  revision = "更新后的会议纪要";
  fireEvent.click(within(screen.getByRole("search")).getByRole("button", { name: "搜索" }));
  expect(await screen.findByRole("link", { name: "更新后的会议纪要" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "上一页" })).toBeDisabled();
});
