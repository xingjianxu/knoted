import { useReducer, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert, Badge, Button, ConfirmDialog, Dialog, FormField, Link, Menu,
  PropertyItem, PropertyList, Segmented, Select, Spinner,
} from "@bixlab/mtone";
import { ApiRequestError, type components } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { projectQuery } from "../projects/queries";
import { projectChoicesQuery, taskChoicesQuery, taskQuery, tasksQuery, type Project, type Task } from "./queries";
import { priorityOptions, statusOptions, TaskForm } from "./TaskForm";
import { formatTaskDate, TaskLink, TaskList } from "./TaskList";
import { SubtaskList } from "./SubtaskList";
import { TaskBoard } from "./board/TaskBoard";
import { ActivityTimeline } from "./timeline/ActivityTimeline";

const message = (cause: unknown) => cause instanceof Error ? cause.message : "操作失败，请重试";
type OnSelectTask = (id: string | null) => void;
type CreateTask = (input: components["schemas"]["TaskCreate"]) => Promise<void>;

export function TasksPage({ bundle, taskId, onSelectTask, onSelectProject }: {
  bundle: QueryClientBundle;
  taskId?: string;
  onSelectTask: OnSelectTask;
  onSelectProject?: (id: string) => void;
}) {
  const queryClient = useQueryClient();
  const [view, setView] = useState<"list" | "board">("list");
  const [boardBusy, setBoardBusy] = useState(false);
  const [archived, setArchived] = useState(false);
  const [projectId, setProjectId] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState("active:"));

  const projects = useQuery(projectChoicesQuery(bundle));
  const parents = useQuery({ ...taskChoicesQuery(bundle), enabled: !!taskId || createOpen });
  const list = useQuery({
    ...tasksQuery(bundle, { archived, project_id: projectId || undefined, cursor: page.cursor ?? undefined }),
    enabled: !taskId,
  });
  const choicesReady = !!projects.data && !!parents.data && !projects.isError && !parents.isError;
  const choicesErrors = <>
    {projects.isError && <Alert tone="error">
      {projects.error.message}
      <Button disabled={projects.isFetching} onClick={() => void projects.refetch()}>重试项目选项</Button>
    </Alert>}
    {parents.isError && <Alert tone="error">
      {parents.error.message}
      <Button disabled={parents.isFetching} onClick={() => void parents.refetch()}>重试父事项选项</Button>
    </Alert>}
  </>;

  async function invalidate() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() }),
      queryClient.invalidateQueries({ queryKey: queryKeys.projects.all() }),
    ]);
  }
  async function create(input: components["schemas"]["TaskCreate"]) {
    const { data } = await bundle.api.raw.POST("/tasks", { body: input });
    if (!data) throw new Error("无法创建事项");
    await invalidate();
  }
  function changeFilter(nextArchived: boolean, nextProject: string) {
    setArchived(nextArchived);
    setProjectId(nextProject);
    dispatch({ type: "filter", filterKey: `${nextArchived ? "archived" : "active"}:${nextProject}` });
  }

  if (taskId) return <TaskDetail key={taskId} bundle={bundle} taskId={taskId}
    projects={projects.data ?? []} parents={parents.data ?? []} choicesReady={choicesReady}
    choicesErrors={choicesErrors} onSelectTask={onSelectTask} onSelectProject={onSelectProject}
    invalidate={invalidate} create={create} />;

  const nextCursor = list.data?.next_cursor ?? null;
  const listBusy = boardBusy || list.isFetching;
  return <section aria-label="事项列表" className="space-y-4">
    <header className="flex flex-wrap items-center justify-between gap-3">
      <h1 className="page-title">事项</h1>
      <Dialog title="新建事项" open={createOpen} onOpenChange={(next) => {
        if (!creating) setCreateOpen(next);
      }} trigger={<Button variant="blue" disabled={boardBusy}>新建事项</Button>}>
        {createOpen && <div className="space-y-3">
          {choicesErrors}
          {!choicesReady && (projects.isFetching || parents.isFetching) && <Spinner label="正在读取事项选项" />}
          <TaskForm projects={projects.data ?? []} parents={parents.data ?? []} disabled={!choicesReady}
            onCancel={() => setCreateOpen(false)} onSubmit={async (input) => {
              setCreating(true);
              try {
                await create(input);
                setCreateOpen(false);
              } finally {
                setCreating(false);
              }
            }} submitLabel="创建事项" />
        </div>}
      </Dialog>
    </header>
    <Segmented label="事项视图" value={view} disabled={boardBusy}
      onValueChange={(value) => setView(value === "board" ? "board" : "list")}
      options={[{ value: "list", label: "列表视图" }, { value: "board", label: "看板视图" }]} />
    <div className="flex flex-wrap items-end gap-3">
      <FormField label="事项归档筛选">
        <Select disabled={boardBusy} value={archived ? "archived" : "active"}
          onValueChange={(value) => changeFilter(value === "archived", projectId)}
          options={[{ value: "active", label: "未归档" }, { value: "archived", label: "已归档" }]} />
      </FormField>
      <FormField label="项目筛选">
        <Select disabled={boardBusy || !projects.data} value={projectId}
          onValueChange={(value) => changeFilter(archived, value ?? "")}
          options={[{ value: "", label: "全部项目与独立事项" },
            ...(projects.data ?? []).map((row) => ({ value: row.id, label: row.title }))]} />
      </FormField>
      <Button disabled={listBusy} onClick={() => void invalidate()}>刷新事项列表</Button>
    </div>
    {!createOpen && projects.isError && choicesErrors}
    {list.isFetching && <Spinner label="正在读取事项" />}
    {list.error && <Alert tone="error">{list.error.message}
      <Button disabled={listBusy} onClick={() => void list.refetch()}>重试事项列表</Button>
    </Alert>}
    {list.data && (list.data.items.length > 0 || list.isSuccess) && (view === "list"
      ? <TaskList items={list.data.items} projects={projects.data ?? []} onSelectTask={onSelectTask} />
      : <TaskBoard bundle={bundle} items={list.data.items} projects={projects.data ?? []}
          disabled={list.isFetching || !!list.error} onSelectTask={onSelectTask}
          onBusyChange={setBoardBusy} invalidate={invalidate} />)}
    {(list.data || page.previous.length > 0) && <nav aria-label="事项分页" className="flex flex-wrap gap-2">
      <Button disabled={listBusy || !page.previous.length}
        onClick={() => dispatch({ type: "previous" })}>上一页事项</Button>
      <Button disabled={listBusy || nextCursor === null || nextCursor === page.cursor}
        onClick={() => dispatch({ type: "next", nextCursor })}>下一页事项</Button>
    </nav>}
  </section>;
}

function TaskDetail({
  bundle, taskId, projects, parents, choicesReady, choicesErrors, onSelectTask, onSelectProject, invalidate, create,
}: {
  bundle: QueryClientBundle;
  taskId: string;
  projects: Project[];
  parents: Task[];
  choicesReady: boolean;
  choicesErrors: React.ReactNode;
  onSelectTask: OnSelectTask;
  onSelectProject?: (id: string) => void;
  invalidate: () => Promise<void>;
  create: CreateTask;
}) {
  const query = useQuery(taskQuery(bundle, taskId));
  const task = query.data;
  const parent = useQuery({ ...taskQuery(bundle, task?.parent_id ?? ""), enabled: !!task?.parent_id });
  const project = useQuery({ ...projectQuery(bundle, task?.project_id ?? ""), enabled: !!task?.project_id });
  const [conflict, setConflict] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState("");
  const [actionError, setActionError] = useState("");
  const [deleteError, setDeleteError] = useState("");
  const [busy, setBusy] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [rebaseTask, setRebaseTask] = useState<Task>();
  const menuRef = useRef<HTMLButtonElement>(null);
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
    setRefreshing(true);
    setRefreshError("");
    try {
      const latest = await query.refetch();
      if (latest.error || !latest.data) throw latest.error ?? new Error("事项不可用");
      if (latest.data.parent_id) await bundle.queryClient.fetchQuery({ ...taskQuery(bundle, latest.data.parent_id), staleTime: 0 });
      if (latest.data.project_id) await bundle.queryClient.fetchQuery({ ...projectQuery(bundle, latest.data.project_id), staleTime: 0 });
      await invalidate();
      setRebaseTask(latest.data);
      setConflict("");
    } catch (cause) {
      setRefreshError(`刷新失败：${message(cause)}`);
    } finally {
      setRefreshing(false);
    }
  }

  if (!task && query.isPending) return <Spinner label="正在读取事项详情" />;
  if (!task) return <Alert tone="error">{query.error?.message ?? "事项不可用"}
    <Button disabled={query.isFetching} onClick={() => void query.refetch()}>重试事项详情</Button>
  </Alert>;

  const readOnly = ancestorBlocked || task.archived_at !== null || task.deleted_at !== null;
  const detailWritesLocked = readOnly || query.isError || busy || refreshing || !!conflict;
  const lifecycleLocked = ancestorBlocked || busy || refreshing || !!conflict || query.isError || !!task.deleted_at;

  async function save(input: components["schemas"]["TaskCreate"], expectedVersion?: number) {
    setBusy(true);
    try {
      if (expectedVersion === undefined) throw new Error("草稿基础版本不可用，请重新打开事项");
      const { data } = await bundle.api.raw.PATCH("/tasks/{taskId}", {
        params: { path: { taskId } }, body: { ...input, expected_version: expectedVersion },
      });
      if (!data) throw new Error("无法读取保存后的事项");
      bundle.queryClient.setQueryData(queryKeys.tasks.detail(taskId), data);
      await invalidate();
      setEditOpen(false);
      return data;
    } catch (cause) {
      recordConflict(cause);
      throw cause;
    } finally {
      setBusy(false);
    }
  }

  async function lifecycle(action: "archive" | "unarchive") {
    if (lifecycleLocked || !task) return;
    setBusy(true);
    setActionError("");
    try {
      const options = { params: { path: { taskId } }, body: { expected_version: task.version } };
      const { data } = action === "archive"
        ? await bundle.api.raw.POST("/tasks/{taskId}/archive", options)
        : await bundle.api.raw.POST("/tasks/{taskId}/unarchive", options);
      if (data) bundle.queryClient.setQueryData(queryKeys.tasks.detail(taskId), data);
      await invalidate();
    } catch (cause) {
      recordConflict(cause);
      setActionError(message(cause));
    } finally {
      setBusy(false);
    }
  }

  function changeEdit(next: boolean) {
    if (busy || refreshing) return;
    if (!next) {
      setConflict("");
      setRefreshError("");
      setRebaseTask(undefined);
    }
    setEditOpen(next);
  }

  const conflictControl = <>
    {conflict && <Alert tone="warning">{conflict}
      <Button disabled={refreshing} onClick={() => void refresh()}>刷新服务端版本</Button>
    </Alert>}
    {refreshError && <Alert tone="error">{refreshError}</Alert>}
  </>;
  const parentOptions = parent.data && !parents.some((row) => row.id === parent.data!.id)
    ? [...parents, parent.data] : parents;

  return <section aria-label="事项详情" className="mx-auto min-w-0 max-w-[70rem] space-y-5">
    <header className="flex flex-wrap items-start justify-between gap-3">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h1 className="page-title wrap-anywhere">{task.title}</h1>
        <Badge tone={task.status === "done" ? "green" : "neutral"}>
          {statusOptions.find(({ value }) => value === task.status)?.label}
        </Badge>
        {task.archived_at && <Badge>已归档</Badge>}
      </div>
      <div className="flex flex-wrap gap-2">
        <Dialog title="编辑事项" open={editOpen} onOpenChange={changeEdit}
          trigger={<Button variant="blue" disabled={readOnly || lifecycleLocked || !choicesReady}>编辑事项</Button>}>
          {editOpen && <div className="space-y-3">
            {choicesErrors}
            {conflictControl}
            {rebaseTask && !conflict && <Alert tone="info" title={`服务端版本 ${rebaseTask.version}`}>
              <p className="wrap-anywhere">{rebaseTask.title}</p>
              {rebaseTask.description && <p className="wrap-anywhere whitespace-pre-wrap">{rebaseTask.description}</p>}
            </Alert>}
            <TaskForm task={task} projects={projects} parents={parentOptions}
              disabled={readOnly || query.isError || !choicesReady || refreshing || busy} conflict={!!conflict}
              rebaseTask={rebaseTask} onSubmit={save} submitLabel="保存事项" onCancel={() => changeEdit(false)} />
          </div>}
        </Dialog>
        {task.archived_at
          ? <Button disabled={lifecycleLocked} onClick={() => void lifecycle("unarchive")}>取消事项归档</Button>
          : <Menu trigger={<Button ref={menuRef} disabled={lifecycleLocked}>更多事项操作</Button>} items={[
              { key: "archive", label: "归档事项", onSelect: () => void lifecycle("archive") },
              { key: "delete", label: "删除事项", danger: true, onSelect: () => setDeleteOpen(true) },
            ]} />}
      </div>
    </header>
    {readOnly && <Alert tone="info">{ancestorBlocked
      ? "父事项或项目不可写，当前事项只读；请先恢复父事项或项目。"
      : task.deleted_at ? "事项已删除，为只读。" : "事项已归档，为只读；取消归档后可编辑。"}</Alert>}
    {!editOpen && choicesErrors}
    {query.error && <Alert tone="error">读取事项失败：{query.error.message}
      <Button disabled={query.isFetching} onClick={() => void query.refetch()}>重试事项详情</Button>
    </Alert>}
    {(parent.isError || project.isError) && <Alert tone="error">无法读取所属项目或父事项。
      <Button disabled={refreshing} onClick={() => void refresh()}>重试读取关联</Button>
    </Alert>}
    {!editOpen && conflictControl}
    {actionError && <Alert tone="error">{actionError}</Alert>}
    <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(13rem,18rem)]">
      <aside aria-label="事项属性" className="min-w-0 wrap-anywhere lg:col-start-2 lg:row-start-1">
        <PropertyList>
          <PropertyItem label="所属项目">{task.project_id
            ? <Link href={`/projects/${task.project_id}`} className="page-link wrap-anywhere" onClick={(event) => {
                if (!onSelectProject || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
                event.preventDefault();
                onSelectProject(task.project_id!);
              }}>{project.data?.title ?? task.project_id}</Link>
            : "独立事项"}</PropertyItem>
          {task.parent_id && <PropertyItem label="父事项"><TaskLink
            task={parent.data ?? { id: task.parent_id, title: task.parent_id }} onSelectTask={onSelectTask} /></PropertyItem>}
          <PropertyItem label="优先级">{priorityOptions.find(({ value }) => value === task.priority)?.label}</PropertyItem>
          <PropertyItem label="开始（上海）">{formatTaskDate(task.start)}</PropertyItem>
          <PropertyItem label="截止（上海）">{formatTaskDate(task.due)}</PropertyItem>
          <PropertyItem label="版本">{task.version}</PropertyItem>
        </PropertyList>
      </aside>
      <div className="min-w-0 space-y-6 lg:col-start-1 lg:row-start-1">
        {task.description && <p className="reading-measure wrap-anywhere whitespace-pre-wrap">{task.description}</p>}
        {task.parent_id === null && <SubtaskList bundle={bundle} parent={task} projects={projects}
          disabled={detailWritesLocked || !choicesReady}
          onCreate={create} onSelectTask={onSelectTask} />}
        <ActivityTimeline bundle={bundle} taskId={taskId} disabled={detailWritesLocked} />
      </div>
    </div>
    <ConfirmDialog title="删除事项？"
      description="事项进入回收站，删除满 30 天后具备清理资格。存在子事项时须先处理子事项。"
      confirmText="确认删除事项" cancelText="取消" danger open={deleteOpen} onOpenChange={(next) => {
        if (busy) return;
        setDeleteOpen(next);
        if (!next) {
          setDeleteError("");
          requestAnimationFrame(() => menuRef.current?.focus());
        }
      }} onConfirm={async () => {
        if (readOnly || lifecycleLocked) throw new Error("当前事项不可删除");
        setBusy(true);
        setDeleteError("");
        try {
          await bundle.api.raw.DELETE("/tasks/{taskId}", {
            params: { path: { taskId }, query: { expected_version: task.version } },
          });
          await invalidate();
          onSelectTask(null);
        } catch (cause) {
          recordConflict(cause);
          setDeleteError(message(cause));
          throw cause;
        } finally {
          setBusy(false);
        }
      }}>
      {deleteError && <Alert tone="error">{deleteError}
        {conflict && <Button disabled={refreshing} onClick={() => void refresh()}>刷新删除版本</Button>}
      </Alert>}
    </ConfirmDialog>
  </section>;
}
