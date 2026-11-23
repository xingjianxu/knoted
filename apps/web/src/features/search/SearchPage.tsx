import { Alert, Button, Card, Empty, Fieldset, Form, Input, List, PageLayout, Select, Spinner, Text, Toolbar } from "@bixlab/mtone";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useId, useReducer, useRef, useState } from "react";
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

const timeFormat = new Intl.DateTimeFormat("zh-CN", { dateStyle: "short", timeStyle: "short" });

function toLocalTime(value: string | undefined) {
  if (!value) return "";
  const date = new Date(value);
  return new Date(date.getTime() - date.getTimezoneOffset() * 60_000).toISOString().slice(0, -1);
}

function fromLocalTime(value: string) {
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toISOString() : undefined;
}

function SearchForm({ bundle, filters, filterKey, onSearch }: {
  bundle: QueryClientBundle;
  filters: SearchFilters;
  filterKey: string;
  onSearch: (value: SearchFilters) => void;
}) {
  const [inputs, setInputs] = useState({ filterKey, draft: filters });
  const [expanded, setExpanded] = useState(false);
  const [error, setError] = useState<"query" | "range">();
  const filtersId = useId();
  const dirtyId = useId();
  const keyword = useRef<HTMLInputElement>(null);
  const rangeEnd = useRef<HTMLInputElement>(null);
  const tags = useQuery(searchTagChoicesQuery(bundle));
  const projects = useQuery(projectChoicesQuery(bundle));

  if (inputs.filterKey !== filterKey) {
    setInputs({ filterKey, draft: filters });
    setError(undefined);
  }
  const { draft } = inputs;
  const setDraft = (value: SearchFilters) => setInputs({ filterKey, draft: value });

  const tagOptions = new Map((tags.data ?? []).map((tag) => [tag.id, tag.name]));
  for (const id of draft.tags ?? []) {
    if (!tagOptions.has(id)) tagOptions.set(id, id);
  }
  const projectOptions = new Map((projects.data ?? []).map((project) => [project.id, project.title]));
  if (draft.project && !projectOptions.has(draft.project)) projectOptions.set(draft.project, draft.project);

  const normalized = validateSearch(draft);
  const submitted = [
    filters.q && normalized.q !== filters.q && `搜索词 ${filters.q}`,
    filters.type && searchTypes[filters.type],
    filters.project && `项目 ${projectOptions.get(filters.project) ?? filters.project}`,
    filters.tags?.length && `标签 ${filters.tags.map((id) => tagOptions.get(id) ?? id).join("、")}`,
    filters.archived !== undefined && (filters.archived ? "已归档" : "未归档"),
    filters.updated_from && `更新时间起 ${timeFormat.format(new Date(filters.updated_from))}`,
    filters.updated_to && `更新时间止 ${timeFormat.format(new Date(filters.updated_to))}`,
    filters.path && `路径 ${filters.path}`,
    filters.format && `格式 ${filters.format}`,
    filters.sort && filters.sort !== "relevance" && searchSorts[filters.sort],
  ].filter(Boolean);
  const changed = JSON.stringify(normalized) !== filterKey;
  useEffect(() => {
    if (expanded && error === "range") rangeEnd.current?.focus();
  }, [expanded, error]);

  return (
    <Form role="search" aria-label="搜索资料" className="space-y-3" onSubmit={(event) => {
      event.preventDefault();
      if (!draft.q.trim()) {
        setError("query");
        keyword.current?.focus();
        return;
      }
      if (draft.updated_from && draft.updated_to && Date.parse(draft.updated_from) > Date.parse(draft.updated_to)) {
        setError("range");
        if (expanded) rangeEnd.current?.focus();
        setExpanded(true);
        return;
      }
      setError(undefined);
      if (expanded) {
        setExpanded(false);
        keyword.current?.focus();
      }
      onSearch(normalized);
    }}>
      <div className="flex flex-wrap items-start gap-2">
        <div className="min-w-0 flex-1 basis-44">
          <Form.Item label={<span className="sr-only">搜索词</span>} error={error === "query" ? "请输入搜索词。" : undefined}>
            <Input.Search
              ref={keyword}
              type="search"
              placeholder="输入搜索词"
              maxLength={500}
              allowClear
              value={draft.q}
              onChange={(event) => {
                setDraft({ ...draft, q: event.target.value });
                if (error === "query") setError(undefined);
              }}
            />
          </Form.Item>
        </div>
        <Button type="submit" variant="solid" aria-describedby={changed ? dirtyId : undefined}>搜索</Button>
      </div>
      <Toolbar
        background="white"
        aria-label="搜索筛选与排序"
        start={
          <>
            <Button type="button" aria-expanded={expanded} aria-controls={filtersId} onClick={() => setExpanded(!expanded)}>
              筛选条件
            </Button>
            <div className="min-w-0">
              <Form.Item label={<span className="sr-only">排序</span>}>
                <Select
                  options={sortOptions}
                  value={draft.sort ?? "relevance"}
                  onValueChange={(value) => setDraft({ ...draft, sort: validateSearch({ sort: value }).sort })}
                />
              </Form.Item>
            </div>
            {changed && <Text id={dirtyId} muted className="text-sm">有待提交的修改</Text>}
          </>
        }
      />
      {submitted.length > 0 && <p className="text-sm wrap-anywhere text-muted">已应用：{submitted.join(" · ")}</p>}
      <div id={filtersId} hidden={!expanded}>
        {expanded && <Card size="small"><div className="grid gap-4 @min-[42rem]/workspace:grid-cols-2">
          <Fieldset legend="对象" className="min-w-0">
            <div className="space-y-2">
              <Form.Item label="类型">
                <Select
                  options={typeOptions}
                  placeholder="全部类型"
                  value={draft.type ?? null}
                  onValueChange={(value) => setDraft({ ...draft, type: validateSearch({ type: value }).type })}
                />
              </Form.Item>
              <Form.Item label="项目">
                <Select
                  options={[{ value: "", label: "全部项目" }, ...Array.from(projectOptions, ([value, label]) => ({ value, label }))]}
                  placeholder="全部项目"
                  value={draft.project ?? null}
                  onValueChange={(value) => setDraft({ ...draft, project: value || undefined })}
                />
              </Form.Item>
              {projects.isError && <Alert tone="error">
                无法读取项目选项。{" "}
                <Button type="button" onClick={() => void projects.refetch()}>重试读取项目</Button>
              </Alert>}
              <Form.Item label="标签">
                <Select
                  mode="multiple"
                  options={Array.from(tagOptions, ([value, label]) => ({ value, label }))}
                  value={draft.tags ?? []}
                  onValueChange={(value) => setDraft({ ...draft, tags: value })}
                />
              </Form.Item>
              {tags.isError && <Alert tone="error">
                无法读取标签选项。{" "}
                <Button type="button" onClick={() => void tags.refetch()}>重试读取标签</Button>
              </Alert>}
              <Form.Item label="归档">
                <Select
                  options={archiveOptions}
                  placeholder="全部归档状态"
                  value={draft.archived === undefined ? null : String(draft.archived)}
                  onValueChange={(value) => setDraft({ ...draft, archived: value ? value === "true" : undefined })}
                />
              </Form.Item>
            </div>
          </Fieldset>
          <div className="min-w-0 space-y-4">
            <Fieldset legend="更新时间" className="min-w-0">
              <div className="space-y-2">
                <Form.Item label="更新时间起">
                  <Input
                    type="datetime-local"
                    step="any"
                    value={toLocalTime(draft.updated_from)}
                    onChange={(event) => {
                      setDraft({ ...draft, updated_from: fromLocalTime(event.target.value) });
                      if (error === "range") setError(undefined);
                    }}
                  />
                </Form.Item>
                <Form.Item label="更新时间止" error={error === "range" ? "更新时间起点须早于或等于终点。" : undefined}>
                  <Input
                    ref={rangeEnd}
                    type="datetime-local"
                    step="any"
                    value={toLocalTime(draft.updated_to)}
                    onChange={(event) => {
                      setDraft({ ...draft, updated_to: fromLocalTime(event.target.value) });
                      if (error === "range") setError(undefined);
                    }}
                  />
                </Form.Item>
              </div>
            </Fieldset>
            <Fieldset legend="文件位置" className="min-w-0">
              <div className="space-y-2">
                <Form.Item label="路径">
                  <Input value={draft.path ?? ""} onChange={(event) => setDraft({ ...draft, path: event.target.value })} />
                </Form.Item>
                <Form.Item label="格式">
                  <Input value={draft.format ?? ""} onChange={(event) => setDraft({ ...draft, format: event.target.value })} />
                </Form.Item>
              </div>
            </Fieldset>
          </div>
        </div></Card>}
      </div>
    </Form>
  );
}

function SearchResults({ bundle, filters }: { bundle: QueryClientBundle; filters: SearchFilters }) {
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const result = useQuery(searchQuery(bundle, filters, page.cursor));
  const next = result.data?.next_cursor ?? null;
  const canGoNext = !!next && next !== page.cursor && !page.previous.includes(next);
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
          <Button disabled={result.isFetching || result.isError || !canGoNext} onClick={() => dispatch({ type: "next", nextCursor: next })}>下一页</Button>
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
    <PageLayout title="搜索" className="min-w-0">
      <div className="min-w-0 space-y-4">
        <SearchForm
          bundle={bundle}
          filters={filters}
          filterKey={filterKey}
          onSearch={(next) => {
            if (JSON.stringify(next) === filterKey) setSubmission((current) => current + 1);
            else onSearch(next);
          }}
        />
        {filters.q && (
          <SearchResults key={`results:${filterKey}:${submission}`} bundle={bundle} filters={filters} />
        )}
      </div>
    </PageLayout>
  );
}
