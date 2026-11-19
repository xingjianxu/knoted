import { createFileRoute, useNavigate, useRouterState } from "@tanstack/react-router";
import { TasksPage } from "../features/tasks/TasksPage";
export const Route = createFileRoute("/_authenticated/tasks")({ component: TasksRoute });
function TasksRoute() {
  const bundle = Route.useRouteContext(); const navigate = useNavigate();
  const taskId = useRouterState({ select: (state) => state.location.pathname.match(/^\/tasks\/([0-9a-f-]+)\/?$/i)?.[1]?.toLowerCase() });
  return <TasksPage bundle={bundle} taskId={taskId}
    onSelectProject={(projectId) => void navigate({ to: "/projects/$projectId", params: { projectId } })}
    onSelectTask={(id) => { if (id) void navigate({ to: "/tasks/$taskId", params: { taskId: id } }); else void navigate({ to: "/tasks" }); }} />;
}
