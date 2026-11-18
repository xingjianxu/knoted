import { createFileRoute, Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { FilesPage } from "../features/files/FilesPage";

export const Route = createFileRoute("/_authenticated/files")({ component: FilesRoute });

function FilesRoute() {
  const bundle = Route.useRouteContext();
  const navigate = useNavigate();
  // UUID identity is case-insensitive; server DTOs use lowercase.
  const fileId = useRouterState({
    select: (state) => state.location.pathname.match(/^\/files\/([0-9a-f-]+)\/?$/i)?.[1]?.toLowerCase(),
  });
  return (
    <FilesPage
      bundle={bundle}
      fileId={fileId}
      jobsLink={<Link to="/files/jobs" className="text-accent hover:underline">索引任务</Link>}
      onSelectFile={(id) => {
        if (id === null) {
          void navigate({ to: "/files" });
          return;
        }
        void navigate({ to: "/files/$fileId", params: { fileId: id } });
      }}
    />
  );
}
