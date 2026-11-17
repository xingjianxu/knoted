import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, Empty, List, ListItem } from "@bixlab/mtone";
import { queryKeys, type QueryClientBundle } from "../../../lib/query";
import { projectQuery } from "../../projects/queries";
import { taskQuery, type Task } from "../queries";

const columns = [
  { status: "todo", label: "待办" }, { status: "in_progress", label: "进行中" },
  { status: "done", label: "已完成" }, { status: "cancelled", label: "已取消" },
] as const;
type Status = Task["status"];
/** Fixed task status columns; native HTML drag/drop and keyboard commands share one CAS operation. */
export function TaskBoard({ bundle, items, disabled, onSelectTask, onBusyChange, invalidate }: {
  bundle: QueryClientBundle; items: Task[]; disabled: boolean;
  onSelectTask: (id: string) => void; onBusyChange: (busy: boolean) => void; invalidate: () => Promise<void>;
}) {
  const [pending, setPending] = useState<{ id: string; status: Status }>();
  const [error, setError] = useState("");
  const writing = useRef(false); const dragged = useRef<string | undefined>(undefined);
  const [keyboardFocus, setKeyboardFocus] = useState<string>();
  const locked = disabled || !!pending;
  async function move(task: Task, status: Status, keyboard = false) {
    if (locked || writing.current || task.status === status || task.archived_at || task.deleted_at) return;
    writing.current = true; onBusyChange(true); setError("");
    setKeyboardFocus(keyboard ? task.id : undefined);
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
  return <section aria-label="事项看板" className="space-y-3">
    <p id="task-board-help">拖动事项到目标列，或聚焦卡片后按 Alt + 左右方向键修改状态。状态操作受父事项完成门槛约束。</p>
    {error && <Alert tone="error">{error}</Alert>}
    <div className="grid gap-4 md:grid-cols-4">{columns.map((column) => <section key={column.status} aria-label={column.label} className="space-y-2">
      <h3>{column.label}</h3>
      <List aria-label={`${column.label}事项`} className="min-h-24" onDragOver={(event) => { if (!locked && dragged.current) event.preventDefault(); }} onDrop={(event) => {
        event.preventDefault(); if (locked) return;
        const id = dragged.current;
        const row = items.find((item) => item.id === id);
        if (row) void move(row, column.status);
      }}>{items.filter((task) => (pending?.id === task.id ? pending.status : task.status) === column.status).map((task) => <TaskCard key={task.id} bundle={bundle} task={task} disabled={locked}
        focusOnMount={keyboardFocus === task.id} onSelectTask={onSelectTask}
        onDragStart={() => { dragged.current = task.id; }} onDragEnd={() => { dragged.current = undefined; }}
        onMove={(direction) => { const index = columns.findIndex((col) => col.status === task.status); const target = columns[index + direction]; if (target) void move(task, target.status, true); }} />)}</List>
    </section>)}</div>
    {!items.length && <Empty title="暂无事项" />}
  </section>;
}
function TaskCard({ bundle, task, disabled, focusOnMount, onSelectTask, onDragStart, onDragEnd, onMove }: {
  bundle: QueryClientBundle; task: Task; disabled: boolean; focusOnMount: boolean;
  onSelectTask: (id: string) => void; onDragStart: () => void; onDragEnd: () => void; onMove: (direction: -1 | 1) => void;
}) {
  const parent = useQuery({ ...taskQuery(bundle, task.parent_id ?? ""), enabled: !!task.parent_id });
  const project = useQuery({ ...projectQuery(bundle, task.project_id ?? ""), enabled: !!task.project_id });
  const readOnly = !!task.archived_at || !!task.deleted_at
    || !!task.parent_id && (!parent.data || parent.isError || !!parent.data.archived_at || !!parent.data.deleted_at)
    || !!task.project_id && (!project.data || project.isError || !!project.data.archived_at || !!project.data.deleted_at);
  const locked = disabled || readOnly;
  const titleRef = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (focusOnMount) titleRef.current?.closest("li")?.focus();
  }, [focusOnMount]);
  return <ListItem tabIndex={0} aria-describedby="task-board-help" aria-disabled={locked} draggable={!locked}
    onDragStart={(event) => { if (locked) { event.preventDefault(); return; } event.dataTransfer.setData("text/plain", task.id); onDragStart(); }}
    onDragEnd={onDragEnd}
    onKeyDown={(event) => {
      if (event.target !== event.currentTarget || !event.altKey || locked) return;
      if (event.key === "ArrowRight" || event.key === "ArrowLeft") { event.preventDefault(); onMove(event.key === "ArrowRight" ? 1 : -1); }
    }}
    description={readOnly ? "只读事项" : undefined}
    actions={<Button disabled={disabled} aria-label={`打开事项 ${task.title}`} onClick={() => onSelectTask(task.id)}>打开</Button>}>
    <span ref={titleRef}>{task.title}</span>
  </ListItem>;
}
