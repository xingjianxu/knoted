import { createFileRoute } from "@tanstack/react-router";
import { TrashPage } from "../features/trash/TrashPage";
export const Route = createFileRoute("/_authenticated/trash")({ component: TrashRoute });
function TrashRoute() { return <TrashPage bundle={Route.useRouteContext()} />; }
