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
const otherTag = "00000000-0000-4000-8000-000000000003";
const timestamp = "2026-11-18T02:30:00.000Z";
const hit: SearchHit = {
  id,
  entity_type: "note",
  title: "会议纪要",
  snippet: "这份纪要记录了项目讨论。",
  updated_at: timestamp,
  score: null,
};

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

async function openSearch(
  url: string,
  search: (params: URLSearchParams) => Response | Promise<Response>,
  choices?: (path: string) => Response,
) {
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
      return choices?.(pathname) ?? json({ items: [], next_cursor: null });
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

it("t55_1 links search hits to the corresponding note, project and task", async () => {
  const projectId = "00000000-0000-4000-8000-000000000004";
  const taskId = "00000000-0000-4000-8000-000000000005";
  await openSearch("/search?q=纪要", () => page([
    hit,
    { ...hit, id: projectId, entity_type: "project", title: "会议项目" },
    { ...hit, id: taskId, entity_type: "task", title: "跟进会议" },
  ]));
  expect(await screen.findByRole("link", { name: "会议纪要" })).toHaveAttribute("href", `/notes/${id}`);
  expect(screen.getByRole("link", { name: "会议项目" })).toHaveAttribute("href", `/projects/${projectId}`);
  expect(screen.getByRole("link", { name: "跟进会议" })).toHaveAttribute("href", `/tasks/${taskId}`);
});

it("t55_2 restores every submitted filter when the URL is reopened", async () => {
  const params = new URLSearchParams({
    q: "纪要",
    type: "note",
    tags: JSON.stringify([tag, otherTag]),
    project: id,
    updated_from: timestamp,
    updated_to: "2026-11-19T03:00:00.000Z",
    archived: "false",
    sort: "updated_desc",
  });
  const original = await openSearch(`/search?${params}`, () => page([hit]));
  await screen.findByRole("link", { name: "会议纪要" });
  const updatedTo = "2026-11-20T10:30";
  const submittedEnd = new Date(updatedTo).toISOString();
  fireEvent.click(screen.getByRole("button", { name: "筛选条件" }));
  fireEvent.change(screen.getByLabelText("更新时间止"), { target: { value: updatedTo } });
  fireEvent.click(within(screen.getByRole("search")).getByRole("button", { name: "搜索" }));
  await waitFor(() => expect(original.requests.at(-1)?.get("updated_to")).toBe(submittedEnd));
  const url = original.router.state.location.href;
  original.unmount();
  const reopened = await openSearch(url, () => page([hit]));
  expect(await screen.findByRole("link", { name: "会议纪要" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "筛选条件" }));
  expect(screen.getByLabelText("更新时间止")).toHaveValue(updatedTo);
  params.set("updated_to", submittedEnd);
  for (const [key, value] of params) {
    if (key === "tags") expect(reopened.requests.at(-1)?.getAll(key)).toEqual([tag, otherTag]);
    else expect(reopened.requests.at(-1)?.get(key)).toBe(value);
  }
  expect(reopened.requests.at(-1)?.has("cursor")).toBe(false);
});

it("t55_3 highlights matching text without interpreting HTML", async () => {
  const snippet = '<img src=x onerror="alert(1)"> 查找 <em>纯文本</em>';
  await openSearch("/search?q=查找", () => page([{ ...hit, snippet }]));
  await screen.findByRole("link", { name: "会议纪要" });
  const results = screen.getByRole("list", { name: "匹配资料" });
  const item = within(results).getByRole("listitem");
  expect(item.textContent).toContain(snippet);
  expect(item.querySelector("mark")).toHaveTextContent("查找");
  expect(results.querySelector("img, em")).toBeNull();
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


it("keeps unapplied filters while collapsed and searches with them only on submission", async () => {
  const view = await openSearch(`/search?q=纪要&updated_from=${timestamp}`, () => page([hit]));
  await screen.findByRole("link", { name: "会议纪要" });
  const toggle = screen.getByRole("button", { name: "筛选条件" });
  const appliedStart = screen.getByText(/^已应用：更新时间起/).textContent ?? "";
  const draftStart = "2026-11-19T09:00";
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByLabelText("更新时间起")).not.toBeInTheDocument();
  fireEvent.click(toggle);
  fireEvent.change(screen.getByLabelText("更新时间起"), { target: { value: draftStart } });
  fireEvent.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.getByText(appliedStart)).toBeVisible();
  expect(screen.getByText("有待提交的修改")).toBeVisible();
  expect(view.requests).toHaveLength(1);
  fireEvent.click(toggle);
  expect(screen.getByLabelText("更新时间起")).toHaveValue(draftStart);
  fireEvent.click(toggle);
  fireEvent.click(within(screen.getByRole("search")).getByRole("button", { name: "搜索" }));
  await waitFor(() => expect(view.requests.at(-1)?.get("updated_from")).toBe(new Date(draftStart).toISOString()));
  expect(screen.getByText(/^已应用：更新时间起/)).not.toHaveTextContent(appliedStart);
  expect(screen.queryByText("有待提交的修改")).not.toBeInTheDocument();
});

it("opens the time filters and focuses the end field when a collapsed range is invalid", async () => {
  const view = await openSearch("/search?q=纪要", () => page([hit]));
  await screen.findByRole("link", { name: "会议纪要" });
  const toggle = screen.getByRole("button", { name: "筛选条件" });
  fireEvent.click(toggle);
  fireEvent.change(screen.getByLabelText("更新时间起"), { target: { value: "2026-11-19T09:00" } });
  fireEvent.change(screen.getByLabelText("更新时间止"), { target: { value: "2026-11-18T09:00" } });
  fireEvent.click(toggle);
  fireEvent.click(within(screen.getByRole("search")).getByRole("button", { name: "搜索" }));
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByLabelText("更新时间止")).toHaveFocus();
  expect(screen.getByText("更新时间起点须早于或等于终点。")).toBeVisible();
  expect(view.requests).toHaveLength(1);
});

it("keeps keyboard focus on the search input after changing the submitted URL", async () => {
  const view = await openSearch("/search?q=纪要", () => page([hit]));
  await screen.findByRole("link", { name: "会议纪要" });
  const keyword = screen.getByRole("searchbox");
  keyword.focus();
  fireEvent.change(keyword, { target: { value: "后续问题" } });
  fireEvent.submit(screen.getByRole("search"));
  await waitFor(() => expect(view.requests.at(-1)?.get("q")).toBe("后续问题"));
  expect(screen.getByRole("searchbox")).toHaveFocus();
});

it("keeps applied tag IDs available when tag choices fail and retries only that source", async () => {
  let failTags = true;
  const requestedOptions: string[] = [];
  const view = await openSearch(`/search?q=纪要&tags=${tag}`, () => page([hit]), (path) => {
    requestedOptions.push(path);
    if (path.endsWith("/tags") && failTags) return json({}, 503);
    return json({ items: path.endsWith("/tags") ? [{ id: tag, name: "会议资料" }] : [], next_cursor: null });
  });
  await screen.findByRole("link", { name: "会议纪要" });
  fireEvent.click(screen.getByRole("button", { name: "筛选条件" }));
  expect(await screen.findByText("无法读取标签选项。")).toBeVisible();
  expect(screen.getByText(`已应用：标签 ${tag}`)).toBeVisible();
  failTags = false;
  fireEvent.click(screen.getByRole("button", { name: "重试读取标签" }));
  expect(await screen.findByText("已应用：标签 会议资料")).toBeVisible();
  expect(requestedOptions.filter((path) => path.endsWith("/projects"))).toHaveLength(1);
  expect(view.requests.at(-1)?.get("tags")).toBe(tag);
});
