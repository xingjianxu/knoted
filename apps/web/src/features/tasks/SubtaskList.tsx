import { useReducer, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, Dialog, SectionGroup, Spinner } from "@bixlab/mtone";
import type { components } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import type { QueryClientBundle } from "../../lib/query";
import { tasksQuery, type Project, type Task } from "./queries";
import { TaskForm } from "./TaskForm";
import { TaskList } from "./TaskList";

export function SubtaskList({ bundle, parent, projects, disabled, onSelectTask, onCreate }: {
  bundle: QueryClientBundle;
  parent: Task;
  projects: Project[];
  disabled: boolean;
  onSelectTask: (id: string) => void;
  onCreate: (input: components["schemas"]["TaskCreate"]) => Promise<void>;
}) {
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState(parent.id));
  const [open, setOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  // Include archived children so completion/archive blockers are not hidden.
  const children = useQuery(tasksQuery(bundle, { parent_id: parent.id, cursor: page.cursor ?? undefined }));
  const nextCursor = children.data?.next_cursor ?? null;
  return <SectionGroup dividers={false}>
    <SectionGroup.Section
      title="子事项"
      headingLevel={2}
      actions={
        <Dialog title="新建子事项" open={open} onOpenChange={(next) => {
          if (!creating) setOpen(next);
        }} trigger={<Button disabled={disabled}>新建子事项</Button>}>
          {open && <TaskForm parent={parent} projects={projects} parents={[parent]} disabled={disabled}
            onCancel={() => setOpen(false)} onSubmit={async (input) => {
              setCreating(true);
              try {
                await onCreate(input);
                setOpen(false);
              } finally {
                setCreating(false);
              }
            }} submitLabel="创建子事项" />}
        </Dialog>
      }
    >
      <div className="space-y-3">
        {children.isFetching && <Spinner label="正在读取子事项" />}
        {children.error && <Alert tone="error">{children.error.message}
          <Button disabled={children.isFetching} onClick={() => void children.refetch()}>重试子事项</Button>
        </Alert>}
        {children.data && (children.data.items.length > 0 || children.isSuccess) &&
          <TaskList items={children.data.items} projects={projects} onSelectTask={onSelectTask} label="子事项列表" />}
        {(children.data || page.previous.length > 0) && <nav aria-label="子事项分页" className="flex flex-wrap gap-2">
          <Button disabled={children.isFetching || !page.previous.length}
            onClick={() => dispatch({ type: "previous" })}>上一页子事项</Button>
          <Button disabled={children.isFetching || nextCursor === null || nextCursor === page.cursor}
            onClick={() => dispatch({ type: "next", nextCursor })}>下一页子事项</Button>
        </nav>}
      </div>
    </SectionGroup.Section>
  </SectionGroup>;
}
