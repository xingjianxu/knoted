import { queryOptions } from "@tanstack/react-query";
import type { components } from "@knoted/api-client";
import { queryKeys, type QueryClientBundle } from "../../../lib/query";
export type Activity = components["schemas"]["Activity"];
export function activitiesQuery(bundle: QueryClientBundle, taskId: string, cursor: string | null) {
  return queryOptions({ queryKey: [...queryKeys.tasks.activities(taskId), cursor], retry: false, queryFn: async ({ signal }) => {
    const { data } = await bundle.api.raw.GET("/tasks/{taskId}/activities", { signal, params: { path: { taskId }, query: { cursor: cursor ?? undefined, limit: 50 } } });
    if (!data || !Array.isArray(data.items)) throw new Error("无法读取跟进记录");
    return data;
  } });
}
export function revisionsQuery(bundle: QueryClientBundle, taskId: string, activity: Activity, cursor: string | null) {
  return queryOptions({ queryKey: [...queryKeys.tasks.activities(taskId), "revisions", activity.id, activity.version, cursor], retry: false, queryFn: async ({ signal }) => {
    const { data } = await bundle.api.raw.GET("/tasks/{taskId}/activities/{activityId}/revisions", { signal, params: { path: { taskId, activityId: activity.id }, query: { cursor: cursor ?? undefined, limit: 50 } } });
    if (!data || !Array.isArray(data.items)) throw new Error("无法读取跟进修订");
    return data;
  } });
}

/** Apply a confirmed write before refetch, without touching revision pages or other tasks. */
export function replaceCachedActivity(bundle: QueryClientBundle, taskId: string, activity: Activity) {
  const prefix = queryKeys.tasks.activities(taskId);
  bundle.queryClient.setQueriesData<components["schemas"]["ActivityPage"]>(
    { queryKey: prefix, predicate: (query) => query.queryKey.length === prefix.length + 1 },
    (previous) => previous ? { ...previous, items: previous.items.map((item) => item.id === activity.id ? activity : item) } : previous,
  );
}
