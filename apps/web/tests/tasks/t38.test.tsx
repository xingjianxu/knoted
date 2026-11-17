import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory } from "@tanstack/react-router";
import { createApiClient } from "@knoted/api-client";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import App from "../../src/App";
import { ProjectsPage } from "../../src/features/projects/ProjectsPage";
import { createAppRouter } from "../../src/lib/router";
import { queryKeys, type QueryClientBundle } from "../../src/lib/query";
import { UiProvider } from "../../src/lib/ui-provider";

const projectA = "00000000-0000-4000-8000-00000000000a";
const projectB = "00000000-0000-4000-8000-00000000000b";
const tagA = "00000000-0000-4000-8000-0000000000a1";
const tagB = "00000000-0000-4000-8000-0000000000b1";
const stamp = "2026-11-18T00:00:00Z";

interface ProjectRow {
  id: string;
  title: string;
  description: string | null;
  version: number;
  archived_at: string | null;
  deleted_at: string | null;
  created_at: string;
  updated_at: string;
  archive_batch_id?: string | null;
}

interface TaskRow {
  id: string;
  project_id: string | null;
  parent_id: string | null;
  title: string;
  description: string | null;
  status: "todo" | "in_progress" | "done" | "cancelled";
  priority: "none" | "low" | "medium" | "high" | "urgent";
  start: null;
  due: null;
  version: number;
  archived_at: string | null;
  deleted_at: string | null;
  created_at: string;
  updated_at: string;
}

function project(id: string, title: string): ProjectRow {
  return {
    id,
    title,
    description: `${title}的说明`,
    version: 1,
    archived_at: null,
    deleted_at: null,
    created_at: stamp,
    updated_at: stamp,
  };
}

function task(id: string, title: string, status: TaskRow["status"]): TaskRow {
  return {
    id,
    project_id: projectA,
    parent_id: null,
    title,
    description: null,
    status,
    priority: "none",
    start: null,
    due: null,
    version: 1,
    archived_at: null,
    deleted_at: null,
    created_at: stamp,
    updated_at: stamp,
  };
}

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function apiError(status: number, code: string, currentVersion?: number): Response {
  return json(
    {
      code,
      message: code,
      request_id: "00000000-0000-4000-8000-0000000000ff",
      ...(currentVersion === undefined ? {} : { current_version: currentVersion }),
    },
    status,
  );
}

/** Minimal in-memory implementation of the /projects, /tasks, tags and links contract. */
function harness(options: { tasks?: TaskRow[] } = {}) {
  const projects: ProjectRow[] = [project(projectA, "项目甲"), project(projectB, "项目乙")];
  const tasks: TaskRow[] = options.tasks ?? [];
  const tags: Record<string, { id: string; name: string; version: number; created_at: string; updated_at: string }[]> = {
    [projectA]: [{ id: tagA, name: "甲标签", version: 1, created_at: stamp, updated_at: stamp }],
    [projectB]: [{ id: tagB, name: "乙标签", version: 1, created_at: stamp, updated_at: stamp }],
  };
  const requests: Request[] = [];

  const fetch: typeof globalThis.fetch = async (input) => {
    const request = input as Request;
    requests.push(request.clone());
    const path = new URL(request.url).pathname;
    const method = request.method;
    const body = method === "GET" || method === "DELETE" ? undefined : await request.json();

    if (method === "GET" && path === "/api/v1/auth/session") {
      return json({
        session: {
          id: "s1",
          user_id: "u1",
          current: true,
          created_at: stamp,
          last_seen_at: stamp,
          idle_expires_at: stamp,
          absolute_expires_at: stamp,
        },
        csrf_token: "test-csrf-value-long",
      });
    }
    if (method === "GET" && path === "/api/v1/projects") {
      const archived = new URL(request.url).searchParams.get("archived") === "true";
      const items = projects.filter(
        (row) => row.deleted_at === null && (archived ? row.archived_at !== null : row.archived_at === null),
      );
      return json({ items, next_cursor: null });
    }
    if (method === "POST" && path === "/api/v1/projects") {
      const input = body as { title: string; description?: string | null };
      const created = project(`00000000-0000-4000-8000-0000000000${projects.length + 1}c`, input.title);
      created.description = input.description ?? null;
      projects.push(created);
      return json(created, 201);
    }
    const detail = path.match(/^\/api\/v1\/projects\/([0-9a-f-]+)$/);
    if (detail) {
      const row = projects.find((entry) => entry.id === detail[1]);
      if (!row) return apiError(404, "not_found");
      if (method === "GET") return json(row);
      if (method === "PATCH") {
        const patch = body as { expected_version: number; title?: string; description?: string | null };
        if (patch.expected_version !== row.version) return apiError(409, "version_conflict", row.version);
        if (patch.title !== undefined) row.title = patch.title;
        if (patch.description !== undefined) row.description = patch.description;
        row.version += 1;
        return json(row);
      }
      if (method === "DELETE") {
        const expected = Number(new URL(request.url).searchParams.get("expected_version"));
        if (expected !== row.version) return apiError(409, "version_conflict", row.version);
        row.deleted_at = stamp;
        return new Response(null, { status: 204 });
      }
    }
    const action = path.match(/^\/api\/v1\/projects\/([0-9a-f-]+)\/(archive|unarchive|restore)$/);
    if (method === "POST" && action) {
      const row = projects.find((entry) => entry.id === action[1]);
      if (!row) return apiError(404, "not_found");
      const expectVersion = (body as { expected_version: number }).expected_version;
      if (expectVersion !== row.version) return apiError(409, "version_conflict", row.version);
      if (action[2] === "archive") {
        const unfinished = tasks.filter(
          (entry) => entry.project_id === row.id && entry.deleted_at === null
            && (entry.status === "todo" || entry.status === "in_progress"),
        );
        if (unfinished.length > 0) return apiError(409, "version_conflict", row.version);
        row.archived_at = stamp;
        row.archive_batch_id = "batch-1";
        for (const entry of tasks) {
          if (entry.project_id === row.id && entry.archived_at === null) entry.archived_at = stamp;
        }
      } else {
        row.archived_at = null;
        row.archive_batch_id = null;
      }
      row.version += 1;
      return json(row);
    }
    if (method === "GET" && path === "/api/v1/tasks") {
      const query = new URL(request.url).searchParams;
      const items = tasks.filter(
        (entry) => entry.deleted_at === null
          && (query.get("project_id") === null || entry.project_id === query.get("project_id"))
          && (query.get("status") === null || entry.status === query.get("status"))
          && (query.get("archived") === null || String(entry.archived_at !== null) === query.get("archived")),
      );
      return json({ items, next_cursor: null });
    }
    const entityTags = path.match(/^\/api\/v1\/entities\/project\/([0-9a-f-]+)\/tags$/);
    if (method === "GET" && entityTags) {
      return json({ items: tags[entityTags[1]!] ?? [], next_cursor: null });
    }
    if (method === "GET" && path === "/api/v1/tags") {
      return json({ items: Object.values(tags).flat(), next_cursor: null });
    }
    if (method === "GET" && path === "/api/v1/links") {
      return json({ items: [], next_cursor: null });
    }
    throw new Error(`未处理的请求：${method} ${path}`);
  };

  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const api = createApiClient({ baseUrl: "http://localhost/api/v1", fetch });
  return { bundle: { queryClient, api } satisfies QueryClientBundle, requests, projects, tasks };
}

/** Stands in for the route adapter: the page owns no URL knowledge. */
function Page({ bundle }: { bundle: QueryClientBundle }) {
  const [projectId, setProjectId] = useState<string | undefined>(undefined);
  return <ProjectsPage bundle={bundle} projectId={projectId} onSelectProject={(id) => setProjectId(id ?? undefined)} />;
}

function renderPage(bundle: QueryClientBundle) {
  return render(
    <UiProvider>
      <QueryClientProvider client={bundle.queryClient}>
        <Page bundle={bundle} />
      </QueryClientProvider>
    </UiProvider>,
  );
}

function requestPath(request: Request): string {
  return new URL(request.url).pathname + new URL(request.url).search;
}

describe("T38 project page", () => {
  it("t38_1 切换项目后不残留上一个项目的标签与资料关联", async () => {
    const { bundle } = harness();
    renderPage(bundle);

    fireEvent.click(await screen.findByRole("button", { name: "打开项目 项目甲" }));
    expect(await screen.findByText("甲标签")).toBeInTheDocument();
    expect(await screen.findByText("暂无资料关联")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "打开项目 项目乙" }));
    expect(await screen.findByText("乙标签")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("甲标签")).not.toBeInTheDocument());
    expect(screen.getByRole("region", { name: "项目标签与资料关联" })).not.toHaveTextContent("甲标签");
    // 详情草稿属于当前项目，不能继承上一个项目的标题。
    expect(await screen.findByLabelText("项目名称")).toHaveValue("项目乙");
    expect(screen.queryByDisplayValue("项目甲")).not.toBeInTheDocument();
  });

  it("t38_2 归档失败时列出未完成事项", async () => {
    const { bundle } = harness({
      tasks: [
        task("00000000-0000-4000-8000-0000000000t1", "写文档", "todo"),
        task("00000000-0000-4000-8000-0000000000t2", "审阅初稿", "in_progress"),
        task("00000000-0000-4000-8000-0000000000t3", "已完成的事", "done"),
      ],
    });
    renderPage(bundle);

    fireEvent.click(await screen.findByRole("button", { name: "打开项目 项目甲" }));
    fireEvent.click(await screen.findByRole("button", { name: "归档项目" }));

    expect(await screen.findByText("无法归档：仍有未完成事项")).toBeInTheDocument();
    const list = screen.getByRole("list", { name: "未完成事项" });
    expect(within(list).getByText("写文档")).toBeInTheDocument();
    expect(within(list).getByText("审阅初稿")).toBeInTheDocument();
    expect(within(list).queryByText("已完成的事")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "归档项目" })).toBeEnabled();
  });

  it("t38_3 删除确认不暗示会删除笔记", async () => {
    const { bundle, requests } = harness();
    renderPage(bundle);

    fireEvent.click(await screen.findByRole("button", { name: "打开项目 项目甲" }));
    fireEvent.click(await screen.findByRole("button", { name: "删除项目" }));

    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("笔记");
    expect(dialog).toHaveTextContent("不会被删除");
    fireEvent.click(within(dialog).getByRole("button", { name: "确认删除项目" }));

    await waitFor(() => {
      expect(requests.some((entry) => entry.method === "DELETE" && requestPath(entry) === `/api/v1/projects/${projectA}?expected_version=1`)).toBe(true);
    });
    // The project leaves the active list and the detail closes; other projects stay.
    await waitFor(() => expect(screen.queryByRole("button", { name: "打开项目 项目甲" })).not.toBeInTheDocument());
    expect(screen.getByRole("button", { name: "打开项目 项目乙" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "项目详情" })).not.toBeInTheDocument();
  });

  it("创建与修改项目后刷新列表，归档成功失效事项查询", async () => {
    const { bundle, requests } = harness();
    renderPage(bundle);

    fireEvent.change(await screen.findByLabelText("新项目名称"), { target: { value: "项目丙" } });
    fireEvent.change(screen.getByLabelText("新项目说明"), { target: { value: "新的说明" } });
    fireEvent.click(screen.getByRole("button", { name: "创建项目" }));

    expect(await screen.findByRole("button", { name: "打开项目 项目丙" })).toBeInTheDocument();
    const created = requests.find((entry) => entry.method === "POST" && new URL(entry.url).pathname === "/api/v1/projects");
    expect(await created?.json()).toEqual({ title: "项目丙", description: "新的说明" });

    fireEvent.click(screen.getByRole("button", { name: "打开项目 项目甲" }));
    fireEvent.change(await screen.findByLabelText("项目名称"), { target: { value: "项目甲改名" } });
    fireEvent.click(screen.getByRole("button", { name: "保存修改" }));
    await waitFor(() => {
      const patch = requests.find((entry) => entry.method === "PATCH" && new URL(entry.url).pathname === `/api/v1/projects/${projectA}`);
      expect(patch).toBeDefined();
    });
    const patch = requests.find((entry) => entry.method === "PATCH" && new URL(entry.url).pathname === `/api/v1/projects/${projectA}`);
    expect(await patch?.json()).toEqual({ expected_version: 1, title: "项目甲改名" });
    expect(await screen.findByRole("button", { name: "打开项目 项目甲改名" })).toBeInTheDocument();

    bundle.queryClient.setQueryData(queryKeys.tasks.list({ project_id: projectA }), { items: [], next_cursor: null });
    fireEvent.click(await screen.findByRole("button", { name: "归档项目" }));
    expect(await screen.findByRole("button", { name: "取消归档" })).toBeInTheDocument();
    // 归档后只读：编辑与关联入口禁用。
    expect(screen.getByRole("button", { name: "保存修改" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "移除标签 甲标签" })).toBeDisabled();
    await waitFor(() => {
      expect(bundle.queryClient.getQueryState(queryKeys.tasks.list({ project_id: projectA }))?.isInvalidated).toBe(true);
    });

    // 撤销路径：取消归档后恢复可写与列表可见。
    fireEvent.click(screen.getByRole("button", { name: "取消归档" }));
    expect(await screen.findByRole("button", { name: "归档项目" })).toBeInTheDocument();
    expect(screen.queryByText("已归档")).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "保存修改" })).toBeEnabled());
    await waitFor(() => expect(screen.getByRole("button", { name: "移除标签 甲标签" })).toBeEnabled());
    await waitFor(() => expect(screen.getByRole("button", { name: "打开项目 项目甲改名" })).toBeInTheDocument());
  });

  it("保存冲突时保留输入、提示当前版本，并允许用刷新后的版本重试", async () => {
    const { bundle, projects } = harness();
    renderPage(bundle);

    fireEvent.click(await screen.findByRole("button", { name: "打开项目 项目甲" }));
    const title = await screen.findByLabelText("项目名称");
    projects[0]!.version = 7;
    fireEvent.change(title, { target: { value: "冲突后保留的标题" } });
    fireEvent.click(screen.getByRole("button", { name: "保存修改" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("7");
    expect(screen.getByLabelText("项目名称")).toHaveValue("冲突后保留的标题");

    // 重试必须用刷新后的版本，而不是拿着陈旧版本反复撞 409。
    fireEvent.click(screen.getByRole("button", { name: "保存修改" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(projects[0]!.version).toBe(8);
    expect(projects[0]!.title).toBe("冲突后保留的标题");
    expect(screen.getByLabelText("项目名称")).toHaveValue("冲突后保留的标题");
  });

  it("深层链接 /projects/$projectId 直接打开对应项目详情", async () => {
    const { bundle } = harness();
    const router = createAppRouter({
      history: createMemoryHistory({ initialEntries: ["/projects"] }),
      bundle,
    });
    render(
      <UiProvider>
        <App router={router} />
      </UiProvider>,
    );
    await act(async () => {
      await router.load();
    });
    expect(await screen.findByRole("button", { name: "打开项目 项目甲" })).toBeInTheDocument();

    await act(async () => {
      await router.navigate({ to: "/projects/$projectId", params: { projectId: projectA } });
    });
    expect(await screen.findByText("甲标签")).toBeInTheDocument();
    expect(router.state.location.pathname).toBe(`/projects/${projectA}`);
  });
});
