import { createFileRoute } from "@tanstack/react-router";
import { AuthPage } from "../features/auth/auth-page";
import { safeReturnTo } from "../lib/router";
export const Route = createFileRoute("/login")({
  validateSearch: (search: Record<string, unknown>) => ({
    redirect: safeReturnTo(search.redirect),
  }),
  component: Login,
});
function Login() {
  const { redirect } = Route.useSearch();
  return <AuthPage mode="login" returnTo={redirect} />;
}
