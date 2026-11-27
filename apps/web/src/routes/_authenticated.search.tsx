import { createFileRoute } from "@tanstack/react-router";
import { SearchPage } from "../features/search/SearchPage";
import { validateSearch } from "../features/search/queries";

export const Route = createFileRoute("/_authenticated/search")({
  validateSearch,
  component: SearchRoute,
});

function SearchRoute() {
  const navigate = Route.useNavigate();
  // Route search merges validated fields into the original URL parameters.
  const filters = Route.useSearch({ select: validateSearch });
  return (
    <SearchPage
      bundle={Route.useRouteContext()}
      filters={filters}
      onSearch={(filters) => void navigate({ search: filters })}
    />
  );
}
