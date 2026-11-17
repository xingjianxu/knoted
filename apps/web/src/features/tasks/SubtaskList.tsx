import { useReducer } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, Spinner } from "@bixlab/mtone";
import type { components } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import type { QueryClientBundle } from "../../lib/query";
import { tasksQuery, type Project, type Task } from "./queries";
import { TaskForm } from "./TaskForm";
import { TaskList } from "./TaskList";
export function SubtaskList({ bundle, parent, projects, disabled, onSelectTask, onCreate }: {
  bundle: QueryClientBundle; parent: Task; projects: Project[]; disabled: boolean;
  onSelectTask: (id: string) => void; onCreate: (input: components["schemas"]["TaskCreate"]) => Promise<void>;
}) {
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState(parent.id));
  // Include archived children so completion/archive blockers are not hidden.
  const children = useQuery(tasksQuery(bundle, { parent_id: parent.id, cursor: page.cursor ?? undefined }));
  return <section aria-label="子事项" className="space-y-3">
    <h3>子事项</h3>
    {children.isFetching && <Spinner label="正在读取子事项" />}
    {children.error && <Alert tone="error">{children.error.message}<Button onClick={() => void children.refetch()}>重试子事项</Button></Alert>}
    <TaskList items={children.data?.items ?? []} projects={projects} onSelectTask={onSelectTask} label="子事项列表" />
    <div className="flex gap-2"><Button disabled={children.isFetching || !page.previous.length} onClick={() => dispatch({ type: "previous" })}>上一页子事项</Button><Button disabled={children.isFetching || !children.data?.next_cursor} onClick={() => dispatch({ type: "next", nextCursor: children.data?.next_cursor ?? null })}>下一页子事项</Button></div>
    <TaskForm parent={parent} projects={projects} parents={[parent]} disabled={disabled} onSubmit={onCreate} submitLabel="创建子事项" />
  </section>;
}
