import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { DashboardPage } from "../features/dashboard/DashboardPage";
export const Route = createFileRoute("/_authenticated/")({ component: DashboardRoute });
function DashboardRoute() {
  const navigate = useNavigate();
  return <DashboardPage bundle={Route.useRouteContext()}
    onSelectNote={noteId => { void navigate({ to: "/notes/$noteId", params: { noteId } }); }}
    onSelectTask={taskId => { void navigate({ to: "/tasks/$taskId", params: { taskId } }); }} />;
}
