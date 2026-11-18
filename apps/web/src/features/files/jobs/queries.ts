import { ApiRequestError, type components } from "@knoted/api-client";
import { queryOptions } from "@tanstack/react-query";
import { queryKeys, type QueryClientBundle } from "../../../lib/query";

export type Job = components["schemas"]["Job"];
type JobPage = components["schemas"]["JobPage"];
export type JobState = components["schemas"]["JobState"];
export type JobAction = { type: "scan"; key: string } | { type: "retry" | "cancel"; jobId: string };

const activeStates = new Set<JobState>(["queued", "running", "retry_wait", "cancel_requested"]);

export function jobsQuery(bundle: QueryClientBundle, state: JobState | undefined, cursor: string | null) {
  const query = { state, cursor: cursor ?? undefined, limit: 50 };
  return queryOptions<JobPage>({
    queryKey: queryKeys.jobs.list(query),
    retry: false,
    staleTime: 0,
    refetchInterval: ({ state }) => state.data?.items.some((job) => activeStates.has(job.state)) ? 2_000 : false,
    queryFn: async ({ signal }) => {
      const { data } = await bundle.api.raw.GET("/jobs", { params: { query }, signal });
      if (!data) throw new Error("无法读取任务记录。");
      return data;
    },
  });
}

export async function actOnJob(bundle: QueryClientBundle, action: JobAction) {
  const { raw } = bundle.api;
  if (action.type === "scan") {
    const { data } = await raw.POST("/jobs/scan", {
      params: { header: { "Idempotency-Key": action.key } },
      body: { force: true },
    });
    if (!data) throw new Error("扫描任务响应不完整。");
    return data;
  }
  const { data } = await raw.POST(action.type === "cancel" ? "/jobs/{jobId}/cancel" : "/jobs/{jobId}/retry", {
    params: { path: { jobId: action.jobId } },
  });
  if (!data) throw new Error("任务响应不完整。");
  return data;
}

export function actionProblem(error: unknown): string {
  if (error instanceof ApiRequestError) {
    switch (error.code) {
      case "scan_pending": return "已有文件扫描任务，请等待其结束。";
      case "file_scanning_disabled": return "文件扫描未启用。";
      case "state_conflict": return "任务状态已变化，请刷新后重试。";
      case "not_found": return "任务已不存在，请刷新列表。";
      case "unauthenticated": return "会话已失效，请重新登录。";
    }
  }
  return "任务操作失败，请重试。";
}
