import { useState } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { createAppRouter, type AppRouter } from "./lib/router";
export default function App({ router: supplied }: { router?: AppRouter }) {
  const [router] = useState(() => supplied ?? createAppRouter());
  return (
    <QueryClientProvider client={router.options.context!.queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  );
}
