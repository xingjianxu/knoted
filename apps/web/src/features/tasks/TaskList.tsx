import { Badge, Button, Empty, List, ListItem } from "@bixlab/mtone";
import type { Project, Task } from "./queries";
import { statusOptions } from "./TaskForm";
export function TaskList({ items, projects, onSelectTask, label = "事项列表项" }: {
  items: Task[]; projects: Project[]; onSelectTask: (id: string) => void; label?: string;
}) {
  return <><List aria-label={label}>{items.map((task) => <ListItem key={task.id}
    description={statusOptions.find((row) => row.value === task.status)?.label}
    actions={<Button aria-label={`打开事项 ${task.title}`} onClick={() => onSelectTask(task.id)}>打开</Button>}>
    {task.title} <span>{projects.find((row) => row.id === task.project_id)?.title ?? (task.project_id ? task.project_id : "独立事项")}</span>
    {task.parent_id && <Badge>子事项</Badge>}{task.archived_at && <Badge>已归档</Badge>}
  </ListItem>)}</List>{items.length === 0 && <Empty title="暂无事项" />}</>;
}
