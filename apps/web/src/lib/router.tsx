import { createRouter, type RouterHistory } from "@tanstack/react-router";
import { createQueryClient, type QueryClientBundle } from "./query";
import { routeTree } from "../routeTree.gen";

/** Strict local destinations implemented by the authenticated routes. */
export function safeReturnTo(value: unknown): string {
  return typeof value === "string" &&
    (["/", "/settings/security", "/notes", "/tasks", "/trash", "/files"].includes(value) || /^\/(?:notes|tasks|files)\/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value))
    ? value
    : "/";
}
export function createAppRouter(
  options: { history?: RouterHistory; bundle?: QueryClientBundle } = {},
) {
  const bundle =
    options.bundle ??
    createQueryClient({
      onUnauthorized: () => {
        const path = router.state.location.pathname;
        // Notes owns an initialized local draft beyond server cache eviction.
        // Its first-load 401 still redirects; mounted sessions expose explicit recovery.
        if (path === "/" || path === "/settings/security" || path === "/trash" || /^\/files(?:\/|$)/.test(path)) {
          void bundle.queryClient.cancelQueries().then(() => {
            bundle.queryClient.clear();
            void router.navigate({
              to: "/login",
              search: { redirect: safeReturnTo(path) },
              replace: true,
            });
          });
        }
      },
    });
  const router = createRouter({
    routeTree,
    context: bundle,
    history: options.history,
    defaultPendingMs: 0,
  });
  return router;
}
export type AppRouter = ReturnType<typeof createAppRouter>;
declare module "@tanstack/react-router" {
  interface Register {
    router: AppRouter;
  }
}
