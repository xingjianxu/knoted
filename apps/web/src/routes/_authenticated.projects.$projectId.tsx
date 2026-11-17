import { createFileRoute, notFound } from "@tanstack/react-router";
export const Route = createFileRoute("/_authenticated/projects/$projectId")({
  beforeLoad: ({ params }) => {
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(params.projectId)) throw notFound();
  },
  component: () => null,
});
