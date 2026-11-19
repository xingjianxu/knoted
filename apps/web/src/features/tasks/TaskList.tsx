import { Badge, Empty, Link, List, ListItem } from "@bixlab/mtone";
import { priorityOptions, statusOptions } from "./TaskForm";
import type { Project, Task } from "./queries";

export function TaskLink({ task, onSelectTask, disabled }: {
  task: Pick<Task, "id" | "title">;
  disabled?: boolean;
  onSelectTask?: (id: string) => void;
}) {
  return <Link href={`/tasks/${task.id}`} className="page-link wrap-anywhere" disabled={disabled} onClick={(event) => {
    if (disabled) {
      event.preventDefault();
      return;
    }
    if (!onSelectTask || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    onSelectTask(task.id);
  }}>{task.title}</Link>;
}

export function formatTaskDate(value: Task["due"]) {
  return value ? `${value.date}${value.time ? ` ${value.time}` : ""}` : "无";
}

export function TaskList({ items, projects, onSelectTask, label = "事项列表项" }: {
  items: Task[];
  projects: Project[];
  onSelectTask?: (id: string) => void;
  label?: string;
}) {
  if (!items.length) return <Empty title="暂无事项" />;
  return <List aria-label={label}>
    {items.map((task) => <ListItem key={task.id}
      description={<span className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span>{statusOptions.find(({ value }) => value === task.status)?.label}</span>
        <span className="wrap-anywhere">{projects.find(({ id }) => id === task.project_id)?.title ?? task.project_id ?? "独立事项"}</span>
        <span>优先级 {priorityOptions.find(({ value }) => value === task.priority)?.label}</span>
        {task.parent_id && <Badge>子事项</Badge>}
        {task.archived_at && <Badge>已归档</Badge>}
      </span>}>
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-1">
        <TaskLink task={task} onSelectTask={onSelectTask} />
        {task.due && <span className="text-sm text-muted">截止 {formatTaskDate(task.due)}</span>}
      </div>
    </ListItem>)}
  </List>;
}
