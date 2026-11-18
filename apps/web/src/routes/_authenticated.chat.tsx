import { createFileRoute, Outlet, useRouterState } from "@tanstack/react-router";
import { ChatPage } from "../features/chat/ChatPage";

export const Route = createFileRoute("/_authenticated/chat")({ component: ChatRoute });

function ChatRoute() {
  const sessionId = useRouterState({
    select: (state) => state.location.pathname.match(/^\/chat\/([0-9a-f-]+)\/?$/i)?.[1]?.toLowerCase(),
  });
  return (
    <ChatPage bundle={Route.useRouteContext()} currentSession={sessionId}>
      <Outlet />
    </ChatPage>
  );
}
