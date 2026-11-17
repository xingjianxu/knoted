import { createFileRoute } from "@tanstack/react-router";
import { NotesPage } from "../features/notes/NotesPage";
export const Route = createFileRoute("/_authenticated/notes")({ component: NotesRoute });
function NotesRoute() { return <NotesPage bundle={Route.useRouteContext()} />; }
