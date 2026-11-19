import { createFileRoute, useNavigate, useRouterState } from "@tanstack/react-router";
import { ProjectsPage } from "../features/projects/ProjectsPage";

export const Route = createFileRoute("/_authenticated/projects")({ component: ProjectsRoute });

function ProjectsRoute() {
  const bundle = Route.useRouteContext();
  const navigate = useNavigate();
  // UUID identity is case-insensitive; server DTOs use lowercase.
  const projectId = useRouterState({
    select: (state) => state.location.pathname.match(/^\/projects\/([0-9a-f-]+)\/?$/i)?.[1]?.toLowerCase(),
  });
  return (
    <ProjectsPage
      bundle={bundle}
      projectId={projectId}
      onSelectTask={(taskId) => void navigate({ to: "/tasks/$taskId", params: { taskId } })}
      onSelectProject={(id) => {
        if (id === null) {
          void navigate({ to: "/projects" });
          return;
        }
        void navigate({ to: "/projects/$projectId", params: { projectId: id } });
      }}
    />
  );
}
