import { queryOptions } from "@tanstack/react-query";
import type { components } from "@knoted/api-client";
import type { QueryClientBundle } from "../../lib/query";
import { queryKeys } from "../../lib/query";

export type Project = components["schemas"]["Project"];
export type ProjectPage = components["schemas"]["ProjectPage"];
export type Task = components["schemas"]["Task"];
export type TaskStatus = components["schemas"]["TaskStatus"];

/** Statuses the archive precondition treats as unfinished (server: NOT IN done/cancelled). */
const UNFINISHED_STATUSES = ["todo", "in_progress"] as const satisfies readonly TaskStatus[];

export function projectsQuery(
  bundle: QueryClientBundle,
  filters: { archived: boolean; cursor: string | null },
) {
  return queryOptions({
    queryKey: queryKeys.projects.list({
      archived: filters.archived,
      cursor: filters.cursor ?? undefined,
    }),
    retry: false,
    queryFn: async ({ signal }) => {
      const { data }: { data?: ProjectPage } = await bundle.api.raw.GET("/projects", {
        signal,
        params: {
          query: {
            archived: filters.archived,
            limit: 50,
            cursor: filters.cursor ?? undefined,
          },
        },
      });
      if (!data) throw new Error("无法读取项目列表");
      return data;
    },
  });
}

export function projectQuery(bundle: QueryClientBundle, projectId: string) {
  return queryOptions({
    queryKey: queryKeys.projects.detail(projectId),
    retry: false,
    queryFn: async ({ signal }) => {
      const { data }: { data?: Project } = await bundle.api.raw.GET("/projects/{projectId}", {
        signal,
        params: { path: { projectId } },
      });
      if (!data) throw new Error("无法读取项目");
      return data;
    },
  });
}

async function tasksWithStatus(
  bundle: QueryClientBundle,
  signal: AbortSignal,
  projectId: string,
  status: TaskStatus,
): Promise<Task[]> {
  const items: Task[] = [];
  const seen = new Set<string>();
  let cursor: string | null = null;
  do {
    if (cursor !== null) {
      if (seen.has(cursor)) throw new Error("事项游标重复，请重试读取未完成事项");
      seen.add(cursor);
    }
    const { data }: { data?: components["schemas"]["TaskPage"] } = await bundle.api.raw.GET("/tasks", {
      signal,
      params: {
        query: { project_id: projectId, status, limit: 100, cursor: cursor ?? undefined },
      },
    });
    if (!data) throw new Error("无法读取项目事项");
    items.push(...data.items);
    cursor = data.next_cursor;
  } while (cursor !== null);
  return items;
}

/**
 * The archive precondition only reports a 409, so the page asks the task list
 * which unfinished tasks block it. Pages through both unfinished statuses.
 */
export function unfinishedTasksQuery(bundle: QueryClientBundle, projectId: string) {
  return queryOptions({
    queryKey: queryKeys.tasks.list({ project_id: projectId, status: "unfinished" }),
    retry: false,
    queryFn: async ({ signal }) => {
      const groups = await Promise.all(
        UNFINISHED_STATUSES.map((status) => tasksWithStatus(bundle, signal, projectId, status)),
      );
      return groups.flat();
    },
  });
}
