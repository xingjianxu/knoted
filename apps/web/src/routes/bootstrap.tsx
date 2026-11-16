import { createFileRoute } from "@tanstack/react-router";
import { AuthPage } from "../features/auth/auth-page";
export const Route = createFileRoute("/bootstrap")({
  component: () => <AuthPage mode="bootstrap" />,
});
