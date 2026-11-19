import { Alert, Button, ConfirmDialog, Empty, FilterBar, FormField, List, ListItem, Select, Spinner, Status } from "@bixlab/mtone";
import { ApiRequestError } from "@knoted/api-client";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useReducer, useRef, useState, type ReactNode } from "react";
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

function kindActions(job: Job, pending: boolean, error: string | undefined, onAction: (action: JobAction) => Promise<void>) {
  if (job.state === "failed" || job.state === "cancelled") {
    return (
      <ConfirmDialog
        title="重试任务？"
        description="重试会创建一次新的执行，并保留这条记录。"
        confirmText="确认重试"
        cancelText="返回"
        trigger={<Button disabled={pending}>重试任务</Button>}
        onConfirm={() => onAction({ type: "retry", jobId: job.id })}
      >{error && <Alert tone="error">{error}</Alert>}</ConfirmDialog>
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
      >{error && <Alert tone="error">{error}</Alert>}</ConfirmDialog>
    );
  }
  return undefined;
}

function JobRow({ job, pending, error, onAction }: {
  job: Job;
  pending: boolean;
  error?: string;
  onAction: (action: JobAction) => Promise<void>;
}) {
  const problem = job.error ? jobProblem(job.error) : undefined;
  const updated = new Date(job.updated_at);
  return (
    <ListItem>
      <div className="grid min-w-0 gap-2 @min-[48rem]/workspace:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)_minmax(0,1.5fr)_minmax(0,auto)] @min-[48rem]/workspace:items-center">
        <span className="font-medium wrap-anywhere">{jobName(job.kind)}</span>
        <span><Status tone={job.state === "failed" ? "red" : job.state === "succeeded" ? "green" : "neutral"}>{jobStates[job.state]}</Status></span>
        <span className="text-sm text-muted">尝试 {job.attempts}/{job.max_attempts}</span>
        {Number.isFinite(updated.getTime()) && <time dateTime={job.updated_at} className="text-sm text-muted"><span className="sr-only">更新时间 </span>{timeFormat.format(updated)}</time>}
        <div>{kindActions(job, pending, error, onAction)}</div>
        {problem && <p className="wrap-anywhere @min-[48rem]/workspace:col-span-full"><code>{problem.code}</code> {problem.message}</p>}
      </div>
    </ListItem>
  );
}

export function JobsPage({ bundle, filesLink }: { bundle: QueryClientBundle; filesLink?: ReactNode }) {
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
    <div className="space-y-5">
      {filesLink}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1>索引任务</h1>
        <Button loading={operation.isPending && operation.variables?.type === "scan"} disabled={operation.isPending} variant="solid" onClick={() => {
          scanKey.current ??= crypto.randomUUID();
          void run({ type: "scan", key: scanKey.current }).catch(() => {});
        }}>扫描文件</Button>
      </div>
      <FilterBar className="flex flex-wrap items-end gap-3">
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
        <Button disabled={busy} onClick={() => void list.refetch()}>刷新任务</Button>
      </FilterBar>
      {notice && <Alert tone="success">{notice}</Alert>}
      {operation.error && operation.variables?.type === "scan" && <Alert tone="error">{operation.error.message}</Alert>}
      {list.isPending && <Spinner label="正在读取任务" />}
      {list.isError && <Alert tone="error">无法读取任务记录。请刷新后重试。</Alert>}

      <div className="space-y-5" aria-busy={list.isFetching}>
        {list.data && items.length > 0 && stageKeys.map((stage) => {
          const jobs = items.filter((job) => jobStage(job.kind) === stage);
          if (stage === "other" && jobs.length === 0) return null;
          const title = jobStages[stage];
          return (
            <section aria-label={title} className="space-y-2" key={stage}>
              <h2 className="section-title">{title}</h2>
              {jobs.length === 0 ? <p className="text-sm text-muted">本页没有{title}任务</p> : <List aria-label={`${title}任务`}>
                {jobs.map((job) => <JobRow key={job.id} job={job} pending={operation.isPending}
                  error={operation.variables && "jobId" in operation.variables && operation.variables.jobId === job.id
                    ? operation.error?.message : undefined}
                  onAction={run} />)}
              </List>}
            </section>
          );
        })}
        {list.isSuccess && !list.isFetching && items.length === 0 && <Empty title="没有匹配的任务记录" />}
      </div>
      {(list.data || page.previous.length > 0) && (
        <nav aria-label="任务翻页" className="flex gap-2">
          <Button disabled={busy || page.previous.length === 0} onClick={() => dispatch({ type: "previous" })}>上一页</Button>
          <Button disabled={busy || list.isError || !list.data?.next_cursor || list.data.next_cursor === page.cursor}
            onClick={() => dispatch({ type: "next", nextCursor: list.data?.next_cursor ?? null })}>下一页</Button>
        </nav>
      )}
    </div>
  );
}
