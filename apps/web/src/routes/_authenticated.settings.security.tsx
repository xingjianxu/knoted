import { createFileRoute } from "@tanstack/react-router";
import { SecurityPage } from "../features/auth/security-page";
export const Route = createFileRoute("/_authenticated/settings/security")({
  component: SecurityPage,
});
