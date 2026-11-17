import { createFileRoute, notFound } from "@tanstack/react-router";
export const Route = createFileRoute("/_authenticated/notes/$noteId")({ beforeLoad: ({ params }) => {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(params.noteId)) throw notFound();
}, component: () => null });
