import { Alert, Badge, Button, ConfirmDialog, Empty, FormField, List, ListItem, Select, Spinner } from "@bixlab/mtone";
import { ApiRequestError } from "@knoted/api-client";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useReducer, useRef, useState } from "react";
import { createCursorState, cursorReducer } from "../../../lib/cursor-state";
import { queryKeys, type QueryClientBundle } from "../../../lib/query";
import { jobName, jobProblem, jobStage, jobStages, jobStates } from "./job-status";
import { actOnJob, actionProblem, jobsQuery, type Job, type JobAction, type JobState } from "./queries";

const stageKeys = ["extraction", "fulltext", "vector", "other"] as const;
const stateOptions = [
  { value: "", label: "全部任务" },
  ...Object.entries(jobStates).map(([value, label]) => ({ value, label })),
];
const timeFormat = new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" });

function kindActions(job: Job, pending: boolean, onAction: (action: JobAction) => Promise<void>) {
  if (job.state === "failed" || job.state === "cancelled") {
    return (
      <ConfirmDialog
        title="重试任务？"
        description="重试会创建一次新的执行，并保留这条记录。"
        confirmText="确认重试"
        cancelText="返回"
        trigger={<Button disabled={pending}>重试任务</Button>}
        onConfirm={() => onAction({ type: "retry", jobId: job.id })}
      />
    );
  }
  if (job.state === "queued" || job.state === "running" || job.state === "retry_wait") {
    return (
      <ConfirmDialog
        title="取消任务？"
        description="正在运行的任务需要等待执行退出确认。"
        confirmText="确认取消"
        cancelText="返回"
        danger
        trigger={<Button disabled={pending}>取消任务</Button>}
        onConfirm={() => onAction({ type: "cancel", jobId: job.id })}
      />
    );
  }
  return undefined;
}

function JobRow({ job, pending, onAction }: {
  job: Job;
  pending: boolean;
  onAction: (action: JobAction) => Promise<void>;
}) {
  const problem = job.error ? jobProblem(job.error) : undefined;
  const updated = new Date(job.updated_at);
  return (
    <ListItem
      actions={kindActions(job, pending, onAction)}
      description={(
        <span className="block space-y-1">
          <span className="flex flex-wrap gap-x-3 text-sm text-muted">
            <span>尝试 {job.attempts}/{job.max_attempts}</span>
            {Number.isFinite(updated.getTime()) && <time dateTime={job.updated_at}>{timeFormat.format(updated)}</time>}
          </span>
          {problem && <span className="block wrap-anywhere"><code>{problem.code}</code> {problem.message}</span>}
        </span>
      )}
    >
      <span className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{jobName(job.kind)}</span>
        <Badge tone={job.state === "failed" ? "red" : "neutral"}>{jobStates[job.state]}</Badge>
      </span>
    </ListItem>
  );
}

export function JobsPage({ bundle }: { bundle: QueryClientBundle }) {
  const [state, setState] = useState<JobState>();
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const scanKey = useRef<string | null>(null);
  const activeAction = useRef(false);
  const [notice, setNotice] = useState("");
  const list = useQuery(jobsQuery(bundle, state, page.cursor));
  const operation = useMutation({
    retry: false,
    mutationFn: async (action: JobAction) => {
      try {
        return await actOnJob(bundle, action);
      } catch (error) {
        if (action.type === "scan" && error instanceof ApiRequestError && error.status < 500) {
          scanKey.current = null;
        }
        throw new Error(actionProblem(error), { cause: error });
      }
    },
    onSuccess: async (job, action) => {
      if (action.type === "scan") scanKey.current = null;
      setNotice({
        scan: "已提交文件扫描。",
        retry: "已创建新的重试任务。",
        cancel: job.state === "cancel_requested" ? "已提交取消请求。" : "任务已取消。",
      }[action.type]);
      await bundle.queryClient.invalidateQueries({ queryKey: queryKeys.jobs.all() });
    },
  });

  async function run(action: JobAction) {
    if (activeAction.current) return;
    activeAction.current = true;
    setNotice("");
    try {
      await operation.mutateAsync(action);
    } finally {
      activeAction.current = false;
    }
  }

  const items = list.data?.items ?? [];
  const busy = operation.isPending || list.isFetching;
  return (
    <div className="mx-auto max-w-5xl space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-2xl font-semibold">索引任务</h2>
        <div className="flex flex-wrap gap-2">
          <Button disabled={operation.isPending} variant="solid" onClick={() => {
            scanKey.current ??= crypto.randomUUID();
            void run({ type: "scan", key: scanKey.current }).catch(() => {});
          }}>扫描文件</Button>
          <Button disabled={busy} onClick={() => void list.refetch()}>刷新任务</Button>
        </div>
      </div>
      <div className="max-w-xs">
        <FormField label="任务状态">
          <Select
            options={stateOptions}
            value={state ?? ""}
            disabled={operation.isPending}
            onValueChange={(value) => {
              setState(value && Object.hasOwn(jobStates, value) ? value as JobState : undefined);
              dispatch({ type: "filter", filterKey: value ?? "" });
            }}
          />
        </FormField>
      </div>
      {notice && <Alert tone="success">{notice}</Alert>}
      {operation.error && operation.variables?.type === "scan" && <Alert tone="error">{operation.error.message}</Alert>}
      {list.isPending && <Spinner label="正在读取任务" />}
      {list.isError && <Alert tone="error">无法读取任务记录。请刷新后重试。</Alert>}

      <div className="space-y-5" aria-busy={list.isFetching}>
        {stageKeys.map((stage) => {
          const jobs = items.filter((job) => jobStage(job.kind) === stage);
          if (stage === "other" && jobs.length === 0) return null;
          const title = jobStages[stage];
          return (
            <section aria-label={title} className="space-y-2" key={stage}>
              <div className="flex items-center gap-2">
                <h3 className="font-semibold">{title}</h3>
                {stage === "vector" && jobs.length === 0 && <Badge>未启用</Badge>}
              </div>
              {jobs.length > 0 && <List aria-label={`${title}任务`}>
                {jobs.map((job) => <JobRow key={job.id} job={job} pending={operation.isPending} onAction={run} />)}
              </List>}
            </section>
          );
        })}
        {list.data && items.length === 0 && <Empty title="没有匹配的任务记录" />}
      </div>
      {(list.data || page.previous.length > 0) && (
        <nav aria-label="任务翻页" className="flex gap-2">
          <Button disabled={busy || page.previous.length === 0} onClick={() => dispatch({ type: "previous" })}>上一页</Button>
          <Button disabled={busy || list.isError || !list.data?.next_cursor}
            onClick={() => dispatch({ type: "next", nextCursor: list.data?.next_cursor ?? null })}>下一页</Button>
        </nav>
      )}
    </div>
  );
}
