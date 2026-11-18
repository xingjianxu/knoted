import { createFileRoute } from "@tanstack/react-router";
import { JobsPage } from "../features/files/jobs/JobsPage";

export const Route = createFileRoute("/_authenticated/files_/jobs")({
  component: JobsRoute,
});

function JobsRoute() {
  return <JobsPage bundle={Route.useRouteContext()} />;
}
