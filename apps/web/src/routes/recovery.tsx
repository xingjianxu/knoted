import { createFileRoute } from "@tanstack/react-router";
import { AuthPage } from "../features/auth/auth-page";
export const Route = createFileRoute("/recovery")({
  component: () => <AuthPage mode="recovery" />,
});
