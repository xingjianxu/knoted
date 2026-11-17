import { queryOptions } from "@tanstack/react-query";
import type { components, operations } from "@knoted/api-client";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
export type Task = components["schemas"]["Task"];
export type Project = components["schemas"]["Project"];
export type TaskFilters = NonNullable<operations["listTasks"]["parameters"]["query"]>;
export function tasksQuery(bundle: QueryClientBundle, filters: TaskFilters) {
  return queryOptions({ queryKey: queryKeys.tasks.list(filters), retry: false, queryFn: async ({ signal }) => {
    const { data } = await bundle.api.raw.GET("/tasks", { signal, params: { query: { limit: 50, ...filters } } });
    if (!data) throw new Error("无法读取事项列表");
    return data;
  } });
}
export function taskQuery(bundle: QueryClientBundle, taskId: string) {
  return queryOptions({ queryKey: queryKeys.tasks.detail(taskId), retry: false, queryFn: async ({ signal }) => {
    const { data } = await bundle.api.raw.GET("/tasks/{taskId}", { signal, params: { path: { taskId } } });
    if (!data) throw new Error("无法读取事项");
    return data;
  } });
}
/** Form choices must include later pages, rather than silently limiting relationships to 50. */
export function taskChoicesQuery(bundle: QueryClientBundle) {
  return queryOptions({ queryKey: queryKeys.tasks.list({ purpose: "parent-choices" }), retry: false, queryFn: async ({ signal }) => {
    const items: Task[] = []; const seen = new Set<string>(); let cursor: string | undefined;
    do {
      const { data } = await bundle.api.raw.GET("/tasks", { signal, params: { query: { archived: false, limit: 100, cursor } } });
      if (!data) throw new Error("无法读取父事项选项");
      items.push(...data.items); cursor = data.next_cursor ?? undefined;
      if (cursor && seen.has(cursor)) throw new Error("事项游标重复");
      if (cursor) seen.add(cursor);
    } while (cursor);
    return items.filter((item) => item.parent_id === null);
  } });
}
export function projectChoicesQuery(bundle: QueryClientBundle) {
  return queryOptions({ queryKey: queryKeys.projects.list({ purpose: "task-project-choices" }), retry: false, queryFn: async ({ signal }) => {
    const items: Project[] = []; const seen = new Set<string>(); let cursor: string | undefined;
    do {
      const { data } = await bundle.api.raw.GET("/projects", { signal, params: { query: { limit: 100, cursor } } });
      if (!data) throw new Error("无法读取项目选项");
      items.push(...data.items); cursor = data.next_cursor ?? undefined;
      if (cursor && seen.has(cursor)) throw new Error("项目游标重复");
      if (cursor) seen.add(cursor);
    } while (cursor);
    return items;
  } });
}
