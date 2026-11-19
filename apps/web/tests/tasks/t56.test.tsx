import { createApiClient, type components } from "@knoted/api-client";
import { QueryClient } from "@tanstack/react-query";
import { createMemoryHistory } from "@tanstack/react-router";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it } from "vitest";

import App from "../../src/App";
import { createAppRouter } from "../../src/lib/router";
import { UiProvider } from "../../src/lib/ui-provider";

type Job = components["schemas"]["Job"];
const id = "00000000-0000-4000-8000-000000000001";
const nextId = "00000000-0000-4000-8000-000000000002";
const time = "2026-11-18T00:00:00Z";
const failed: Job = {
  id,
  kind: "file.extract",
  state: "failed",
  attempts: 1,
  max_attempts: 4,
  error: "unsupported_format",
  created_at: time,
  updated_at: time,
  started_at: time,
  finished_at: time,
};

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

async function openJobs(
  requests: Request[],
  handle: (path: string, request: Request) => Response | Promise<Response>,
  start = "/files/jobs",
) {
  const fetcher: typeof globalThis.fetch = async (input) => {
    const request = input as Request;
    const path = new URL(request.url).pathname.replace("/api/v1", "");
    if (path === "/auth/session") {
      return json({ session: { id, user_id: id, current: true }, csrf_token: "fixture-csrf-value-long" });
    }
    requests.push(request);
    return handle(path, request);
  };
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
  const router = createAppRouter({
    history: createMemoryHistory({ initialEntries: [start] }),
    bundle: { queryClient, api: createApiClient({ baseUrl: "http://localhost/api/v1", fetch: fetcher }) },
  });
  const view = render(<UiProvider><App router={router} /></UiProvider>);
  await act(async () => router.load());
  return { ...view, router };
}

it("t56_1 retains the permanent failure and only creates a retry on explicit action", async () => {
  const requests: Request[] = [];
  const items = [failed];
  await openJobs(requests, (path) => {
    if (path === `/jobs/${id}/retry`) {
      const queued: Job = { ...failed, id: nextId, state: "queued", error: null, attempts: 0, started_at: null, finished_at: null };
      items.unshift(queued);
      return json(queued, 201);
    }
    return json({ items, next_cursor: null });
  });
  expect(await screen.findByText("unsupported_format")).toBeInTheDocument();
  expect(screen.getByText("确认文件格式受支持后再重试。")).toBeInTheDocument();
  expect(requests.filter((request) => request.method === "POST")).toHaveLength(0);

  fireEvent.click(screen.getByRole("button", { name: "重试任务" }));
  const dialog = await screen.findByRole("alertdialog", { name: "重试任务？" });
  fireEvent.click(within(dialog).getByRole("button", { name: "确认重试" }));
  await screen.findByText("排队中");
  expect(screen.getByText("失败")).toBeInTheDocument();
  const writes = requests.filter((request) => request.method === "POST");
  expect(writes).toHaveLength(1);
  expect(new URL(writes[0]!.url).pathname).toBe(`/api/v1/jobs/${id}/retry`);
});

it("t56_2 renders the cancellation as pending until the worker confirms", async () => {
  let row: Job = { ...failed, state: "running", error: null, finished_at: null };
  await openJobs([], (path) => {
    if (path === `/jobs/${id}/cancel`) {
      row = { ...row, state: "cancel_requested" };
      return json(row);
    }
    return json({ items: [row], next_cursor: null });
  });
  await screen.findByText("运行中");
  fireEvent.click(screen.getByRole("button", { name: "取消任务" }));
  const dialog = await screen.findByRole("alertdialog", { name: "取消任务？" });
  fireEvent.click(within(dialog).getByRole("button", { name: "确认取消" }));
  await screen.findByText("正在取消");
  expect(screen.queryByText("已取消")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "重试任务" })).not.toBeInTheDocument();

  row = { ...row, state: "cancelled", finished_at: time };
  fireEvent.click(screen.getByRole("button", { name: "刷新任务" }));
  expect(await screen.findByText("已取消")).toBeInTheDocument();
});

it("t56_3 reads persisted failures again after remount and reports an empty vector stage for the current page", async () => {
  const handler = () => json({ items: [failed], next_cursor: null });
  const first = await openJobs([], handler);
  await screen.findByText("unsupported_format");
  first.unmount();

  const reopened = await openJobs([], handler);
  expect(await screen.findByText("unsupported_format")).toBeInTheDocument();
  expect(screen.getByText("本页没有向量索引任务")).toBeInTheDocument();
  expect(screen.queryByText("未启用")).not.toBeInTheDocument();
  expect(reopened.router.state.location.pathname).toBe("/files/jobs");
});

it("sends one idempotent scan intent per explicit success and reuses its key after a lost response", async () => {
  const requests: Request[] = [];
  let fail = true;
  const job: Job = { ...failed, kind: "file.scan", state: "queued", error: null, started_at: null, finished_at: null };
  let visible = false;
  await openJobs(requests, (path) => {
    if (path === "/jobs/scan") {
      if (fail) {
        fail = false;
        throw new TypeError("connection lost");
      }
      visible = true;
      return json(job, 201);
    }
    return json({ items: visible ? [job] : [], next_cursor: null });
  });
  const scan = await screen.findByRole("button", { name: "扫描文件" });
  fireEvent.click(scan);
  await screen.findByText("任务操作失败，请重试。");
  fireEvent.click(scan);
  await screen.findByText("扫描文件", { selector: "span" });
  const submissions = requests.filter((request) => request.method === "POST");
  expect(submissions).toHaveLength(2);
  expect(submissions[0]!.headers.get("idempotency-key")).toBeTruthy();
  expect(submissions[1]!.headers.get("idempotency-key")).toBe(submissions[0]!.headers.get("idempotency-key"));
  expect(await submissions[1]!.json()).toEqual({ force: true });
});

it("keeps unsafe error text out of the document", async () => {
  await openJobs([], () => json({ items: [{ ...failed, error: "<svg onload='private fixture'>" }], next_cursor: null }));
  expect(await screen.findByText("job_failed")).toBeInTheDocument();
  expect(screen.queryByText(/private fixture/)).not.toBeInTheDocument();
});

it("opens the task page from the file browser", async () => {
  const view = await openJobs([], () => json({ items: [], next_cursor: null }), "/files");
  fireEvent.click(await screen.findByRole("link", { name: "索引任务" }));
  await waitFor(() => expect(view.router.state.location.pathname).toBe("/files/jobs"));
  expect(await screen.findByRole("heading", { name: "索引任务" })).toBeInTheDocument();
});

it("shows the vector stage with its own jobs when embedding is used", async () => {
  const vector: Job = { ...failed, kind: "rag_vectors.embed", state: "succeeded", error: null };
  await openJobs([], () => json({ items: [vector], next_cursor: null }));
  const stage = await screen.findByRole("region", { name: "向量索引" });
  expect(await within(stage).findByText("嵌入与发布向量")).toBeInTheDocument();
  expect(within(stage).queryByText("未启用")).not.toBeInTheDocument();
});
