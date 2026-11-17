import { createFileRoute, notFound } from "@tanstack/react-router";
export const Route = createFileRoute("/_authenticated/tasks/$taskId")({
  beforeLoad: ({ params }) => {
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(params.taskId)) throw notFound();
  },
  component: () => null,
});
