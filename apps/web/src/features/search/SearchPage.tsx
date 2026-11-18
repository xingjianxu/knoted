import { Alert, Button, Empty, Form, FormField, Input, List, MultiSelect, SearchInput, Select, Spinner } from "@bixlab/mtone";
import { useQuery } from "@tanstack/react-query";
import { useReducer, useState } from "react";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import type { QueryClientBundle } from "../../lib/query";
import { projectChoicesQuery } from "../tasks/queries";
import { SearchResult } from "./SearchResult";
import { searchQuery, searchSorts, searchTagChoicesQuery, searchTypes, validateSearch, type SearchFilters } from "./queries";

const typeOptions = [{ value: "", label: "全部类型" }, ...Object.entries(searchTypes).map(([value, label]) => ({ value, label }))];
const sortOptions = Object.entries(searchSorts).map(([value, label]) => ({ value, label }));
const archiveOptions = [
  { value: "", label: "全部归档状态" },
  { value: "false", label: "未归档" },
  { value: "true", label: "已归档" },
];

function toLocalTime(value: string | undefined) {
  if (!value) return "";
  const date = new Date(value);
  return new Date(date.getTime() - date.getTimezoneOffset() * 60_000).toISOString().slice(0, -1);
}

function fromLocalTime(value: string) {
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toISOString() : undefined;
}

function SearchForm({ bundle, filters, onSearch }: {
  bundle: QueryClientBundle;
  filters: SearchFilters;
  onSearch: (value: SearchFilters) => void;
}) {
  const [draft, setDraft] = useState(filters);
  const [error, setError] = useState("");
  const tags = useQuery(searchTagChoicesQuery(bundle));
  const projects = useQuery(projectChoicesQuery(bundle));
  const tagOptions = new Map((tags.data ?? []).map((tag) => [tag.id, tag.name]));
  for (const id of draft.tags ?? []) {
    if (!tagOptions.has(id)) tagOptions.set(id, id);
  }
  const projectOptions = new Map((projects.data ?? []).map((project) => [project.id, project.title]));
  if (draft.project && !projectOptions.has(draft.project)) projectOptions.set(draft.project, draft.project);

  return (
    <Form role="search" aria-label="搜索资料" className="space-y-4" onSubmit={(event) => {
      event.preventDefault();
      if (!draft.q.trim()) {
        setError("请输入搜索词。");
        return;
      }
      if (draft.updated_from && draft.updated_to && Date.parse(draft.updated_from) > Date.parse(draft.updated_to)) {
        setError("更新时间起点须早于或等于终点。");
        return;
      }
      setError("");
      onSearch(validateSearch(draft));
    }}>
      <div className="flex flex-wrap items-end gap-3">
        <div className="min-w-0 flex-1 basis-48">
          <FormField label="搜索词">
            <SearchInput
              type="search"
              maxLength={500}
              value={draft.q}
              onChange={(event) => setDraft({ ...draft, q: event.target.value })}
            />
          </FormField>
        </div>
        <Button type="submit" variant="solid">搜索</Button>
      </div>

      <div className="grid grid-cols-1 items-end gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <FormField label="类型">
          <Select
            options={typeOptions}
            value={draft.type ?? ""}
            onValueChange={(value) => setDraft({ ...draft, type: validateSearch({ type: value }).type })}
          />
        </FormField>
        <FormField label="标签">
          <MultiSelect
            options={Array.from(tagOptions, ([value, label]) => ({ value, label }))}
            value={draft.tags ?? []}
            onValueChange={(value) => setDraft({ ...draft, tags: value })}
          />
        </FormField>
        <FormField label="项目">
          <Select
            options={[{ value: "", label: "全部项目" }, ...Array.from(projectOptions, ([value, label]) => ({ value, label }))]}
            value={draft.project ?? ""}
            onValueChange={(value) => setDraft({ ...draft, project: value || undefined })}
          />
        </FormField>
        <FormField label="归档">
          <Select
            options={archiveOptions}
            value={String(draft.archived ?? "")}
            onValueChange={(value) => setDraft({ ...draft, archived: value ? value === "true" : undefined })}
          />
        </FormField>
        <FormField label="更新时间起">
          <Input
            type="datetime-local"
            step="any"
            value={toLocalTime(draft.updated_from)}
            onChange={(event) => setDraft({ ...draft, updated_from: fromLocalTime(event.target.value) })}
          />
        </FormField>
        <FormField label="更新时间止">
          <Input
            type="datetime-local"
            step="any"
            value={toLocalTime(draft.updated_to)}
            onChange={(event) => setDraft({ ...draft, updated_to: fromLocalTime(event.target.value) })}
          />
        </FormField>
        <FormField label="路径">
          <Input value={draft.path ?? ""} onChange={(event) => setDraft({ ...draft, path: event.target.value })} />
        </FormField>
        <FormField label="格式">
          <Input value={draft.format ?? ""} onChange={(event) => setDraft({ ...draft, format: event.target.value })} />
        </FormField>
        <FormField label="排序">
          <Select
            options={sortOptions}
            value={draft.sort ?? "relevance"}
            onValueChange={(value) => setDraft({ ...draft, sort: validateSearch({ sort: value }).sort })}
          />
        </FormField>
      </div>
      {(tags.isError || projects.isError) && (
        <Alert tone="error">
          <div className="flex flex-wrap items-center gap-2">
            <span>无法读取标签或项目选项。</span>
            <Button type="button" onClick={() => {
              if (tags.isError) void tags.refetch();
              if (projects.isError) void projects.refetch();
            }}>重试</Button>
          </div>
        </Alert>
      )}
      {error && <Alert tone="error">{error}</Alert>}
    </Form>
  );
}

function SearchResults({ bundle, filters }: { bundle: QueryClientBundle; filters: SearchFilters }) {
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const result = useQuery(searchQuery(bundle, filters, page.cursor));
  const next = result.data?.next_cursor ?? null;
  return (
    <section aria-label="搜索结果" aria-busy={result.isFetching} className="space-y-3">
      {result.isFetching && <Spinner label="正在搜索" />}
      {result.isError && (
        <Alert tone="error">
          <div className="flex flex-wrap items-center gap-2">
            <span>搜索暂时不可用。</span>
            <Button onClick={() => void result.refetch()}>重试搜索</Button>
          </div>
        </Alert>
      )}
      {!result.isError && !result.isFetching && result.data && (result.data.items.length
        ? <List aria-label="匹配资料">
            {result.data.items.map((hit) => (
              <SearchResult key={`${hit.entity_type}:${hit.id}`} hit={hit} query={filters.q} />
            ))}
          </List>
        : <Empty title="没有匹配的资料" />)}
      {(result.data || page.previous.length > 0) && (
        <nav aria-label="搜索翻页" className="flex gap-2">
          <Button disabled={result.isFetching || page.previous.length === 0} onClick={() => dispatch({ type: "previous" })}>上一页</Button>
          <Button disabled={result.isFetching || result.isError || !next || next === page.cursor} onClick={() => dispatch({ type: "next", nextCursor: next })}>下一页</Button>
        </nav>
      )}
    </section>
  );
}

export function SearchPage({ bundle, filters, onSearch }: {
  bundle: QueryClientBundle;
  filters: SearchFilters;
  onSearch: (value: SearchFilters) => void;
}) {
  const filterKey = JSON.stringify(filters);
  const [submission, setSubmission] = useState(0);

  return (
    <div className="mx-auto max-w-5xl space-y-6">
      <h2 className="text-2xl font-semibold">搜索</h2>
      <SearchForm
        key={`form:${filterKey}`}
        bundle={bundle}
        filters={filters}
        onSearch={(next) => {
          if (JSON.stringify(next) === filterKey) setSubmission((current) => current + 1);
          else onSearch(next);
        }}
      />
      {filters.q && (
        <SearchResults key={`results:${filterKey}:${submission}`} bundle={bundle} filters={filters} />
      )}
    </div>
  );
}
