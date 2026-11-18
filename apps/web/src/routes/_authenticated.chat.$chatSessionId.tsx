import { createFileRoute, notFound } from "@tanstack/react-router";
import { z } from "zod";
import { ChatThread } from "../features/chat/ChatThread";

const uuid = z.uuid();

export const Route = createFileRoute("/_authenticated/chat/$chatSessionId")({
  beforeLoad: ({ params }) => {
    if (!uuid.safeParse(params.chatSessionId).success) throw notFound();
  },
  validateSearch: (search: Record<string, unknown>) => ({
    message: uuid.safeParse(search.message).data?.toLowerCase(),
  }),
  component: ChatSessionRoute,
});

function ChatSessionRoute() {
  const { chatSessionId } = Route.useParams();
  const { message } = Route.useSearch();
  const bundle = Route.useRouteContext();
  const sessionId = chatSessionId.toLowerCase();

  return <ChatThread key={sessionId} bundle={bundle} sessionId={sessionId} targetMessage={message} />;
}
