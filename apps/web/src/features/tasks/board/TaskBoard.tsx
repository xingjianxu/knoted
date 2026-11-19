import { useLayoutEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, Empty, List, ListItem, Menu } from "@bixlab/mtone";
import { queryKeys, type QueryClientBundle } from "../../../lib/query";
import { projectQuery } from "../../projects/queries";
import { formatTaskDate, TaskLink } from "../TaskList";
import { taskQuery, type Project, type Task } from "../queries";

const columns = [
  { status: "todo", label: "待办" }, { status: "in_progress", label: "进行中" },
  { status: "done", label: "已完成" }, { status: "cancelled", label: "已取消" },
] as const;
type Status = Task["status"];
/** Fixed task status columns; native HTML drag/drop and keyboard commands share one CAS operation. */
export function TaskBoard({ bundle, items, projects, disabled, onSelectTask, onBusyChange, invalidate }: {
  bundle: QueryClientBundle; items: Task[]; projects: Project[]; disabled: boolean;
  onSelectTask: (id: string) => void; onBusyChange: (busy: boolean) => void; invalidate: () => Promise<void>;
}) {
  const [pending, setPending] = useState<{ id: string; status: Status }>();
  const [error, setError] = useState("");
  const writing = useRef(false); const dragged = useRef<string | undefined>(undefined);
  const board = useRef<HTMLElement>(null);
  const focusRequest = useRef<{ id: string; origin: Element | null } | undefined>(undefined);
  useLayoutEffect(() => {
    const request = focusRequest.current;
    if (!request) return;
    if (!pending) focusRequest.current = undefined;
    const active = document.activeElement;
    if (active !== document.body && active !== request.origin) return;
    const cards = board.current?.querySelectorAll<HTMLElement>("[data-task-card]");
    if (!cards) return;
    Array.from(cards).find((card) => card.dataset.taskCard === request.id)?.focus();
  }, [pending]);

  const locked = disabled || !!pending;
  async function move(task: Task, status: Status, keyboard = false) {
    if (locked || writing.current || task.status === status || task.archived_at || task.deleted_at) return;
    writing.current = true; onBusyChange(true); setError("");
    focusRequest.current = keyboard ? { id: task.id, origin: document.activeElement } : undefined;
    setPending({ id: task.id, status });
    try {
      const { data } = await bundle.api.raw.PATCH("/tasks/{taskId}", {
        params: { path: { taskId: task.id } }, body: { status, expected_version: task.version },
      });
      if (!data) throw new Error("无法读取更新后的事项");
      bundle.queryClient.setQueryData(queryKeys.tasks.detail(task.id), data);
      // Keep a confirmed write visible even when the following list refetch fails.
      bundle.queryClient.setQueriesData<Task[] | { items: Task[]; next_cursor: string | null }>(
        { queryKey: [...queryKeys.tasks.all(), "list"] },
        (previous) => !previous ? previous : Array.isArray(previous)
          ? previous.map((row) => row.id === data.id ? data : row)
          : { ...previous, items: previous.items.map((row) => row.id === data.id ? data : row) },
      );
      await invalidate();
    } catch (cause) { setError(`状态修改失败，已回到原列：${cause instanceof Error ? cause.message : "请重试"}`); }
    finally { setPending(undefined); writing.current = false; onBusyChange(false); dragged.current = undefined; }
  }
  return <section ref={board} aria-label="事项看板" className="@container space-y-3">
    <p id="task-board-help" className="text-sm text-muted">拖动卡片、按 Alt + 左右方向键，或使用卡片菜单更改状态。</p>
    {error && <Alert tone="error">{error}</Alert>}
    <div className="grid gap-4 @min-[60rem]:grid-cols-4">{columns.map((column) => <section key={column.status} aria-label={column.label} className="min-w-0 space-y-2">
      <h2 className="section-title">{column.label}</h2>
      <List aria-label={`${column.label}事项`} className="min-h-24" onDragOver={(event) => { if (!locked && dragged.current) event.preventDefault(); }} onDrop={(event) => {
        event.preventDefault(); if (locked) return;
        const id = dragged.current;
        const row = items.find((item) => item.id === id);
        if (row) void move(row, column.status);
      }}>{items.filter((task) => (pending?.id === task.id ? pending.status : task.status) === column.status).map((task) => <TaskCard key={task.id} bundle={bundle} task={task} projectName={projects.find(({ id }) => id === task.project_id)?.title} disabled={locked}
        onSelectTask={onSelectTask}
        onDragStart={() => { dragged.current = task.id; }} onDragEnd={() => { dragged.current = undefined; }}
        onMove={(status) => void move(task, status, true)} />)}</List>
    </section>)}</div>
    {!items.length && <Empty title="暂无事项" />}
  </section>;
}
function TaskCard({ bundle, task, projectName, disabled, onSelectTask, onDragStart, onDragEnd, onMove }: {
  bundle: QueryClientBundle; task: Task; projectName?: string; disabled: boolean;
  onSelectTask: (id: string) => void; onDragStart: () => void; onDragEnd: () => void; onMove: (status: Status) => void;
}) {
  const parent = useQuery({ ...taskQuery(bundle, task.parent_id ?? ""), enabled: !!task.parent_id });
  const project = useQuery({ ...projectQuery(bundle, task.project_id ?? ""), enabled: !!task.project_id });
  const readOnly = !!task.archived_at || !!task.deleted_at
    || !!task.parent_id && (!parent.data || parent.isError || !!parent.data.archived_at || !!parent.data.deleted_at)
    || !!task.project_id && (!project.data || project.isError || !!project.data.archived_at || !!project.data.deleted_at);
  const locked = disabled || readOnly;
  return <ListItem data-task-card={task.id} tabIndex={0} aria-describedby="task-board-help" aria-disabled={locked} draggable={!locked}
    onDragStart={(event) => { if (locked) { event.preventDefault(); return; } event.dataTransfer.setData("text/plain", task.id); onDragStart(); }}
    onDragEnd={onDragEnd}
    onKeyDown={(event) => {
      if (event.target !== event.currentTarget || !event.altKey || locked) return;
      if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
      event.preventDefault();
      const index = columns.findIndex(({ status }) => status === task.status);
      const target = columns[index + (event.key === "ArrowRight" ? 1 : -1)];
      if (target) onMove(target.status);
    }}
    className="flex-wrap items-start gap-2"
    description={<span className="flex flex-wrap gap-x-2 gap-y-1">
      <span className="wrap-anywhere">{project.data?.title ?? projectName ?? task.project_id ?? "独立事项"}</span>
      {task.due && <span>截止 {formatTaskDate(task.due)}</span>}
      {readOnly && <span>只读事项</span>}
    </span>}
    actions={!readOnly && <Menu
      trigger={<Button size="sm" disabled={locked} aria-label={`更改状态 ${task.title}`}>更改状态</Button>}
      items={columns.filter(({ status }) => status !== task.status).map(({ status, label }) => ({
        key: status, label, onSelect: () => onMove(status),
      }))} />}>
    <TaskLink task={task} disabled={disabled} onSelectTask={onSelectTask} />
  </ListItem>;
}
