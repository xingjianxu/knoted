import { createFileRoute, notFound } from "@tanstack/react-router";
export const Route = createFileRoute("/_authenticated/files/$fileId")({
  beforeLoad: ({ params }) => {
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(params.fileId)) throw notFound();
  },
  component: () => null,
});
