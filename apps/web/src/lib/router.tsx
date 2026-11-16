import { createRouter, type RouterHistory } from "@tanstack/react-router";
import { createQueryClient, type QueryClientBundle } from "./query";
import { routeTree } from "../routeTree.gen";

/** Only destinations implemented in T20 may be returned to after authentication. */
export function safeReturnTo(value: unknown): string {
  return typeof value === "string" &&
    ["/", "/settings/security"].includes(value)
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
        if (path === "/" || path === "/settings/security") {
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
