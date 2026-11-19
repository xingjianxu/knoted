import { createFileRoute, Link } from "@tanstack/react-router";
import { JobsPage } from "../features/files/jobs/JobsPage";

export const Route = createFileRoute("/_authenticated/files_/jobs")({
  component: JobsRoute,
});

function JobsRoute() {
  return <JobsPage bundle={Route.useRouteContext()} filesLink={<Link to="/files" className="page-link">返回文件</Link>} />;
}
