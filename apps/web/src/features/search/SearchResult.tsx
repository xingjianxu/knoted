import { Badge, ListItem } from "@bixlab/mtone";
import { Link } from "@tanstack/react-router";
import { Fragment, type ReactNode } from "react";
import { z } from "zod";

import { searchTypes, type SearchHit } from "./queries";

function highlighted(text: string, query: string): ReactNode {
  const words = [...new Set(query.split(/\s+/u).filter(Boolean))];
  if (!words.length) return text;
  words.sort((left, right) => right.length - left.length);
  const pattern = words.map((word) => word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|");
  return text.split(new RegExp(`(${pattern})`, "giu")).map((part, index) => index % 2 === 0
    ? <Fragment key={index}>{part}</Fragment>
    : <mark className="bg-accent-soft text-inherit" key={index}>{part}</mark>);
}

const uuid = z.uuid();

function chatTarget(hit: SearchHit) {
  if (hit.entity_type !== "chat") return undefined;
  const chatSessionId = uuid.safeParse(hit.session_id).data;
  const message = uuid.safeParse(hit.id).data;
  return chatSessionId && message ? { chatSessionId, message } : undefined;
}

function resultTitle({ entity_type, id, title }: SearchHit, chat: ReturnType<typeof chatTarget>) {
  const className = "wrap-anywhere font-semibold text-accent hover:underline";
  switch (entity_type) {
    case "note": return <Link className={className} to="/notes/$noteId" params={{ noteId: id }}>{title}</Link>;
    case "file": return <Link className={className} to="/files/$fileId" params={{ fileId: id }}>{title}</Link>;
    case "project": return <Link className={className} to="/projects/$projectId" params={{ projectId: id }}>{title}</Link>;
    case "task": return <Link className={className} to="/tasks/$taskId" params={{ taskId: id }}>{title}</Link>;
    case "chat": return chat ? (
      <Link className={className} to="/chat/$chatSessionId" params={{ chatSessionId: chat.chatSessionId }} search={{ message: chat.message }}>{title}</Link>
    ) : <span className="wrap-anywhere font-semibold">{title}</span>;
  }
}

const timeFormat = new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" });

export function SearchResult({ hit, query }: { hit: SearchHit; query: string }) {
  const updated = new Date(hit.updated_at);
  const chat = chatTarget(hit);
  const canOpen = hit.entity_type !== "chat" || !!chat;
  return (
    <ListItem description={
      <span className="block space-y-1 wrap-anywhere">
        {hit.snippet && <span className="block whitespace-pre-wrap text-ink">{highlighted(hit.snippet, query)}</span>}
        <span className="flex flex-wrap gap-x-3 text-sm text-muted">
          {hit.path && <span>{hit.path}</span>}
          {Number.isFinite(updated.getTime()) && <time dateTime={hit.updated_at}>{timeFormat.format(updated)}</time>}
          {(hit.entity_type === "note" || hit.entity_type === "file" || !canOpen) && <span>无法定位到原文位置。</span>}
        </span>
      </span>
    }>
      <span className="flex flex-wrap items-center gap-2">
        <Badge>{searchTypes[hit.entity_type]}</Badge>
        {resultTitle(hit, chat)}
      </span>
    </ListItem>
  );
}
