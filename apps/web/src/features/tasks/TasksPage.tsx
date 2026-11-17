import { useReducer, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert, Button, ConfirmDialog, Empty, FormField, Select, Spinner } from "@bixlab/mtone";
import { ApiRequestError, type components } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { projectQuery } from "../projects/queries";
import { projectChoicesQuery, taskChoicesQuery, taskQuery, tasksQuery, type Project, type Task } from "./queries";
import { TaskForm } from "./TaskForm";
import { TaskList } from "./TaskList";
import { SubtaskList } from "./SubtaskList";
const message = (cause: unknown) => cause instanceof Error ? cause.message : "操作失败，请重试";
export function TasksPage({ bundle, taskId, onSelectTask }: {
  bundle: QueryClientBundle; taskId?: string; onSelectTask: (id: string | null) => void;
}) {
  const queryClient = useQueryClient();
  const [archived, setArchived] = useState(false); const [projectId, setProjectId] = useState("");
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState("active"));
  const projects = useQuery(projectChoicesQuery(bundle)); const parents = useQuery(taskChoicesQuery(bundle));
  const list = useQuery(tasksQuery(bundle, { archived, project_id: projectId || undefined, cursor: page.cursor ?? undefined }));
  async function invalidate() {
    await Promise.all([queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() }), queryClient.invalidateQueries({ queryKey: queryKeys.projects.all() })]);
  }
  async function create(input: components["schemas"]["TaskCreate"]) {
    const { data } = await bundle.api.raw.POST("/tasks", { body: input });
    if (!data) throw new Error("无法创建事项");
    await invalidate();
  }
  return <div className="space-y-6">
    <section aria-label="创建事项" className="space-y-3"><h2>创建事项</h2>
      {projects.error && <Alert tone="error">{projects.error.message}<Button onClick={() => void projects.refetch()}>重试项目选项</Button></Alert>}
      {parents.error && <Alert tone="error">{parents.error.message}<Button onClick={() => void parents.refetch()}>重试父事项选项</Button></Alert>}
      <TaskForm projects={projects.data ?? []} parents={parents.data ?? []} disabled={!projects.data || !parents.data} onSubmit={create} submitLabel="创建事项" />
    </section>
    <section aria-label="事项列表" className="space-y-3"><h2>事项</h2>
      <div className="flex flex-wrap gap-3">
        <FormField label="事项归档筛选"><Select value={archived ? "archived" : "active"} onValueChange={(value) => { setArchived(value === "archived"); dispatch({ type: "filter", filterKey: `${value}:${projectId}` }); }} options={[{ value: "active", label: "未归档" }, { value: "archived", label: "已归档" }]} /></FormField>
        <FormField label="项目筛选"><Select value={projectId} onValueChange={(value) => { setProjectId(value ?? ""); dispatch({ type: "filter", filterKey: `${archived}:${value}` }); }} options={[{ value: "", label: "全部项目与独立事项" }, ...(projects.data ?? []).map((row) => ({ value: row.id, label: row.title }))]} /></FormField>
        <Button disabled={list.isFetching || !page.previous.length} onClick={() => dispatch({ type: "previous" })}>上一页事项</Button>
        <Button disabled={list.isFetching || !list.data?.next_cursor} onClick={() => dispatch({ type: "next", nextCursor: list.data?.next_cursor ?? null })}>下一页事项</Button>
        <Button disabled={list.isFetching} onClick={() => void invalidate()}>刷新事项列表</Button>
      </div>
      {list.isFetching && <Spinner label="正在读取事项" />}
      {list.error && <Alert tone="error">{list.error.message}<Button onClick={() => void list.refetch()}>重试事项列表</Button></Alert>}
      <TaskList items={list.data?.items ?? []} projects={projects.data ?? []} onSelectTask={onSelectTask} />
    </section>
    {taskId ? <TaskDetail key={taskId} bundle={bundle} taskId={taskId} projects={projects.data ?? []} parents={parents.data ?? []} choicesReady={!!projects.data && !!parents.data} onSelectTask={onSelectTask} invalidate={invalidate} create={create} /> : <Empty title="未选择事项" />}
  </div>;
}
function TaskDetail({ bundle, taskId, projects, parents, choicesReady, onSelectTask, invalidate, create }: {
  bundle: QueryClientBundle; taskId: string; projects: Project[]; parents: Task[]; choicesReady: boolean;
  onSelectTask: (id: string | null) => void; invalidate: () => Promise<void>; create: (input: components["schemas"]["TaskCreate"]) => Promise<void>;
}) {
  const query = useQuery(taskQuery(bundle, taskId)); const task = query.data;
  const parent = useQuery({ ...taskQuery(bundle, task?.parent_id ?? ""), enabled: !!task?.parent_id });
  const project = useQuery({ ...projectQuery(bundle, task?.project_id ?? ""), enabled: !!task?.project_id });
  const [conflict, setConflict] = useState(""); const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState(""); const [actionError, setActionError] = useState("");
  const [busy, setBusy] = useState(false);
  const [rebaseTask, setRebaseTask] = useState<Task>();
  const ancestorBlocked = !!task && (
    !!task.parent_id && (!parent.data || parent.isError || parent.data.archived_at !== null || parent.data.deleted_at !== null)
    || !!task.project_id && (!project.data || project.isError || project.data.archived_at !== null || project.data.deleted_at !== null)
  );
  function recordConflict(cause: unknown) {
    if (cause instanceof ApiRequestError && cause.status === 409) {
      setConflict(`事项版本或状态已变化，草稿已保留${cause.currentVersion === undefined ? "" : `（当前版本 ${cause.currentVersion}）`}。刷新服务端版本后核对并重试。`);
    }
  }
  async function refresh() {
    setRefreshing(true); setRefreshError("");
    try {
      const latest = await query.refetch(); if (latest.error || !latest.data) throw latest.error ?? new Error("事项不可用");
      // Recheck writable relationships before allowing a retained draft to retry.
      if (latest.data.parent_id) await bundle.queryClient.fetchQuery({ ...taskQuery(bundle, latest.data.parent_id), staleTime: 0 });
      if (latest.data.project_id) await bundle.queryClient.fetchQuery({ ...projectQuery(bundle, latest.data.project_id), staleTime: 0 });
      await invalidate(); setRebaseTask(latest.data); setConflict("");
    } catch (cause) { setRefreshError(`刷新失败：${message(cause)}`); }
    finally { setRefreshing(false); }
  }
  if (!task && query.isPending) return <Spinner label="正在读取事项详情" />;
  if (!task) return <Alert tone="error">{query.error?.message ?? "事项不可用"}<Button onClick={() => void query.refetch()}>重试事项详情</Button></Alert>;
  const readOnly = ancestorBlocked || task.archived_at !== null || task.deleted_at !== null;
  async function save(input: components["schemas"]["TaskCreate"], expectedVersion?: number) {
    setBusy(true);
    try {
      if (expectedVersion === undefined) throw new Error("草稿基础版本不可用，请重新打开事项");
      const { data } = await bundle.api.raw.PATCH("/tasks/{taskId}", { params: { path: { taskId } }, body: { ...input, expected_version: expectedVersion } });
      if (!data) throw new Error("无法读取保存后的事项");
      bundle.queryClient.setQueryData(queryKeys.tasks.detail(taskId), data);
      await invalidate();
      return data;
    } catch (cause) { recordConflict(cause); throw cause; }
    finally { setBusy(false); }
  }
  async function lifecycle(action: "archive" | "unarchive") {
    if (busy) return; setBusy(true); setActionError("");
    try {
      const options = { params: { path: { taskId } }, body: { expected_version: task!.version } };
      if (action === "archive") await bundle.api.raw.POST("/tasks/{taskId}/archive", options);
      else await bundle.api.raw.POST("/tasks/{taskId}/unarchive", options);
      await invalidate();
    } catch (cause) { recordConflict(cause); setActionError(message(cause)); }
    finally { setBusy(false); }
  }
  return <><section aria-label="事项详情" className="space-y-4">
    <h2>{task.title} <span className="text-sm text-muted">版本 {task.version}</span></h2>
    {readOnly && <Alert tone="info">{ancestorBlocked ? "父事项或项目不可写，当前事项只读；请先恢复父事项或项目。" : "事项已归档，为只读；取消归档后可编辑。"}</Alert>}
    {query.error && <Alert tone="error">读取事项失败：{query.error.message}</Alert>}
    {conflict && <Alert tone="warning">{conflict}<Button disabled={refreshing} onClick={() => void refresh()}>刷新服务端版本</Button></Alert>}
    {refreshError && <Alert tone="error">{refreshError}</Alert>}
    <TaskForm task={task} projects={projects} parents={parent.data && !parents.some((row) => row.id === parent.data.id) ? [...parents, parent.data] : parents} disabled={readOnly || !choicesReady || refreshing || busy} conflict={!!conflict} rebaseTask={rebaseTask} onSubmit={save} submitLabel="保存事项" />
    <div className="flex gap-2">
      <Button disabled={ancestorBlocked || busy || refreshing || !!conflict} onClick={() => void lifecycle(task.archived_at ? "unarchive" : "archive")}>{task.archived_at ? "取消事项归档" : "归档事项"}</Button>
      <ConfirmDialog title="删除事项？" description="事项进入回收站，30天内可恢复。存在子事项时须先处理子事项。" confirmText="确认删除事项" cancelText="取消" danger trigger={<Button danger disabled={readOnly || busy || refreshing || !!conflict}>删除事项</Button>} onConfirm={async () => {
        setBusy(true);
        try { await bundle.api.raw.DELETE("/tasks/{taskId}", { params: { path: { taskId }, query: { expected_version: task.version } } }); await invalidate(); onSelectTask(null); }
        catch (cause) { recordConflict(cause); setActionError(message(cause)); throw cause; }
        finally { setBusy(false); }
      }} />
    </div>
    {actionError && <Alert tone="error">{actionError}</Alert>}
  </section>
    {task.parent_id === null && <SubtaskList bundle={bundle} parent={task} projects={projects} disabled={readOnly || !choicesReady || busy || !!conflict} onCreate={create} onSelectTask={onSelectTask} />}
  </>;
}
