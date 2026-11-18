import { createFileRoute } from "@tanstack/react-router";
import { SearchPage } from "../features/search/SearchPage";
import { validateSearch } from "../features/search/queries";

export const Route = createFileRoute("/_authenticated/search")({
  validateSearch,
  component: SearchRoute,
});

function SearchRoute() {
  const navigate = Route.useNavigate();
  return (
    <SearchPage
      bundle={Route.useRouteContext()}
      filters={Route.useSearch()}
      onSearch={(filters) => void navigate({ search: filters })}
    />
  );
}
