#!/usr/bin/env node
/**
 * Offline contract check for the Knoted browser OpenAPI document.
 *
 * Run from anywhere with `pnpm api:check` or `node api/examples/check.mjs`.
 * The OpenAPI document, example manifest and fixtures are local files.
 * Ajv validates the examples against the schemas in the OpenAPI document.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";
import Ajv2020 from "ajv/dist/2020.js";
import addFormats from "ajv-formats";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const API_DIR = join(ROOT, "api");
const EXAMPLES_DIR = join(API_DIR, "examples");
const HTTP_METHODS = new Set([
  "get",
  "put",
  "post",
  "delete",
  "options",
  "head",
  "patch",
  "trace",
]);

const BROWSER_DOCUMENT = "openapi.yaml";

/** This list is the acceptance contract; keep it hard-coded. */
const REQUIRED_BROWSER_OPERATIONS = [
  "POST /auth/bootstrap/options",
  "POST /auth/bootstrap/finish",
  "POST /auth/recovery/options",
  "POST /auth/recovery/finish",
  "POST /auth/login/options",
  "POST /auth/login/finish",
  "GET /auth/session",
  "POST /auth/logout",
  "GET /auth/sessions",
  "DELETE /auth/sessions/{sessionId}",
  "GET /auth/passkeys",
  "POST /auth/passkeys/options",
  "POST /auth/passkeys/finish",
  "DELETE /auth/passkeys/{passkeyId}",
  "GET /notes",
  "POST /notes",
  "GET /notes/{noteId}",
  "PATCH /notes/{noteId}",
  "DELETE /notes/{noteId}",
  "POST /notes/{noteId}/move",
  "POST /notes/{noteId}/restore",
  "GET /notes/{noteId}/snapshots",
  "POST /notes/{noteId}/snapshots",
  "POST /notes/{noteId}/snapshots/{snapshotId}/restore",
  "POST /assets",
  "GET /assets/{assetId}/content",
  "GET /tags",
  "POST /tags",
  "PATCH /tags/{tagId}",
  "DELETE /tags/{tagId}",
  "GET /links",
  "POST /links",
  "GET /entities/{entityType}/{entityId}/tags",
  "PUT /entities/{entityType}/{entityId}/tags/{tagId}",
  "DELETE /entities/{entityType}/{entityId}/tags/{tagId}",
  "DELETE /links/{linkId}",
  "GET /projects",
  "POST /projects",
  "GET /projects/{projectId}",
  "PATCH /projects/{projectId}",
  "DELETE /projects/{projectId}",
  "POST /projects/{projectId}/archive",
  "POST /projects/{projectId}/restore",
  "POST /projects/{projectId}/unarchive",
  "GET /tasks",
  "POST /tasks",
  "GET /tasks/{taskId}",
  "PATCH /tasks/{taskId}",
  "DELETE /tasks/{taskId}",
  "GET /tasks/{taskId}/activities",
  "POST /tasks/{taskId}/activities",
  "PATCH /tasks/{taskId}/activities/{activityId}",
  "DELETE /tasks/{taskId}/activities/{activityId}",
  "GET /tasks/{taskId}/activities/{activityId}/revisions",
  "POST /tasks/{taskId}/archive",
  "POST /tasks/{taskId}/unarchive",
  "POST /tasks/{taskId}/restore",
  "GET /dashboard",
  "GET /trash",
  "GET /files",
  "GET /files/{fileId}",
  "PATCH /files/{fileId}",
  "GET /files/{fileId}/content",
  "POST /files/{fileId}/relink",
  "POST /files/{fileId}/office-preview",
  "DELETE /files/{fileId}/office-preview",
  "GET /office-preview-sources/{previewId}",
  "GET /search",
];

const CLOSED_ENUMS = {
  TaskStatus: ["todo", "in_progress", "done", "cancelled"],
  TaskPriority: ["none", "low", "medium", "high"],
  EntityEndType: ["note", "file", "project", "task"],
  ActivityAuthorKind: ["manual", "system"],
  SearchType: ["note", "task", "project"],
  BlockType: [
    "heading",
    "paragraph",
    "bulletListItem",
    "numberedListItem",
    "checkListItem",
    "image",
    "codeBlock",
    "table",
  ],
};

const IDEMPOTENT_OPERATION_IDS = ["createAsset"];

const VERSIONED_DELETE_OPERATION_IDS = [
  "deleteNote",
  "deleteTask",
  "deleteProject",
  "deleteTag",
  "deleteLink",
  "deleteTaskActivity",
];

const EXPECTED_VERSION_SCHEMAS = [
  "NoteUpdate",
  "NoteMoveRequest",
  "NoteRestoreRequest",
  "SnapshotRestoreRequest",
  "TaskUpdate",
  "TaskVersionRequest",
  "ProjectUpdate",
  "ProjectVersionRequest",
  "TagUpdate",
  "FileSourceUpdate",
  "FileRelinkRequest",
  "ActivityUpdate",
];

const REQUIRED_EXAMPLES = [
  {
    document: "openapi.yaml",
    schema: "FileRelinkRequest",
    file: "valid/file-relink.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "FileRelinkRequest",
    file: "invalid/file-relink-extra-confirm.json",
    valid: false,
  },

  {
    document: "openapi.yaml",
    schema: "FileSource",
    file: "valid/file-source.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "FileSource",
    file: "invalid/file-source-missing-version.json",
    valid: false,
  },

  {
    document: "openapi.yaml",
    schema: "TrashPage",
    file: "valid/trash-page.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "TrashItem",
    file: "invalid/trash-item-missing-version.json",
    valid: false,
  },
  {
    document: "openapi.yaml",
    schema: "DashboardSummary",
    file: "valid/dashboard-summary.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "DashboardSummary",
    file: "invalid/dashboard-summary-with-indexing.json",
    valid: false,
  },
  {
    document: "openapi.yaml",
    schema: "TaskStatus",
    file: "valid/task-status-in-progress.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "TaskStatus",
    file: "invalid/task-status-blocked.json",
    valid: false,
  },
  {
    document: "openapi.yaml",
    schema: "DateValue",
    file: "valid/date-value-date-only.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "DateValue",
    file: "valid/date-value-with-time.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "DateValue",
    file: "invalid/date-value-bad-time.json",
    valid: false,
  },
  {
    document: "openapi.yaml",
    schema: "DateValue",
    file: "invalid/date-value-missing-date.json",
    valid: false,
  },
  {
    document: "openapi.yaml",
    schema: "DateValue",
    file: "invalid/date-value-missing-time.json",
    valid: false,
  },
  {
    document: "openapi.yaml",
    schema: "NoteUpdate",
    file: "valid/note-update.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "NoteUpdate",
    file: "invalid/note-update-missing-expected-version.json",
    valid: false,
  },
  {
    document: "openapi.yaml",
    schema: "TaskCreate",
    file: "valid/task-create.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "TaskCreate",
    file: "invalid/task-create-bad-priority.json",
    valid: false,
  },
  {
    document: "openapi.yaml",
    schema: "ApiError",
    file: "valid/api-error.json",
    valid: true,
  },
  {
    document: "openapi.yaml",
    schema: "ApiError",
    file: "invalid/api-error-missing-request-id.json",
    valid: false,
  },
];

const CSRF_EXEMPT_PATHS = new Set([
  "/auth/bootstrap/options",
  "/auth/bootstrap/finish",
  "/auth/recovery/options",
  "/auth/recovery/finish",
  "/auth/login/options",
  "/auth/login/finish",
]);

const LIST_RESPONSE_SCHEMAS = new Set([
  "AuthSessionPage",
  "PasskeyPage",
  "NotePage",
  "SnapshotPage",
  "TagPage",
  "ProjectPage",
  "TaskPage",
  "ActivityPage",
  "ActivityRevisionPage",
  "TrashPage",
  "FileSourcePage",
  "SearchResponse",
]);

const ETAG_ENTITY_SCHEMAS = new Set([
  "Note",
  "Project",
  "Task",
  "Tag",
  "FileSource",
]);

class ContractError extends Error {}

function fail(message) {
  throw new ContractError(message);
}

function check(condition, message) {
  if (!condition) fail(message);
}

function readJson(filePath) {
  const source = readFileSync(filePath, "utf8");
  try {
    return JSON.parse(source);
  } catch (error) {
    fail(`${relative(ROOT, filePath)}: JSON 解析失败：${error.message}`);
  }
}

function readYaml(filePath) {
  const source = readFileSync(filePath, "utf8");
  try {
    return parseYaml(source);
  } catch (error) {
    fail(`${relative(ROOT, filePath)}: YAML 解析失败：${error.message}`);
  }
}

function decodePointerSegment(segment) {
  return decodeURIComponent(segment).replaceAll("~1", "/").replaceAll("~0", "~");
}

function resolvePointer(document, ref, where) {
  check(
    typeof ref === "string" && ref.startsWith("#/"),
    `${where}: 只允许文档内部 $ref，发现 ${JSON.stringify(ref)}`,
  );
  let current = document;
  for (const raw of ref.slice(2).split("/")) {
    const segment = decodePointerSegment(raw);
    check(
      current !== null &&
        typeof current === "object" &&
        Object.hasOwn(current, segment),
      `${where}: 悬空 $ref ${ref}（缺少 ${segment}）`,
    );
    current = current[segment];
  }
  return current;
}

function resolveLocal(document, value, where) {
  let current = value;
  const seen = new Set();
  while (
    current !== null &&
    typeof current === "object" &&
    typeof current.$ref === "string"
  ) {
    check(!seen.has(current.$ref), `${where}: 循环 $ref ${current.$ref}`);
    seen.add(current.$ref);
    current = resolvePointer(document, current.$ref, where);
  }
  return current;
}

function walkRefs(document, node, where) {
  if (Array.isArray(node)) {
    node.forEach((item, index) => walkRefs(document, item, `${where}[${index}]`));
    return;
  }
  if (node === null || typeof node !== "object") return;
  for (const [key, value] of Object.entries(node)) {
    if (key === "$ref") {
      resolvePointer(document, value, where);
    } else {
      walkRefs(document, value, where === "" ? key : `${where}.${key}`);
    }
  }
}

function collectOperations(document, documentName) {
  check(
    document !== null && typeof document === "object",
    `${documentName}: 文档根必须是对象`,
  );
  check(
    document.paths !== null && typeof document.paths === "object",
    `${documentName}: 缺少 paths`,
  );
  const operations = [];
  for (const [path, pathItem] of Object.entries(document.paths)) {
    check(path.startsWith("/"), `${documentName}: 路径必须以 / 开头：${path}`);
    check(
      pathItem !== null && typeof pathItem === "object",
      `${documentName}: ${path} 不是对象`,
    );
    for (const [method, operation] of Object.entries(pathItem)) {
      if (!HTTP_METHODS.has(method)) continue;
      check(
        operation !== null && typeof operation === "object",
        `${documentName}: ${method.toUpperCase()} ${path} 不是对象`,
      );
      operations.push({ path, method: method.toUpperCase(), operation, pathItem });
    }
  }
  return operations;
}

function checkInfo(document, documentName, expectedServer) {
  check(
    typeof document.openapi === "string" && document.openapi.startsWith("3.1"),
    `${documentName}: openapi 必须为 3.1.x，实际 ${JSON.stringify(document.openapi)}`,
  );
  check(
    typeof document.info?.title === "string" && document.info.title.length > 0,
    `${documentName}: 缺少 info.title`,
  );
  check(
    typeof document.info?.version === "string" && document.info.version.length > 0,
    `${documentName}: 缺少 info.version`,
  );
  check(
    Array.isArray(document.servers) && document.servers.length > 0,
    `${documentName}: 缺少 servers`,
  );
  check(
    document.servers[0]?.url === expectedServer,
    `${documentName}: servers[0].url 必须为 ${expectedServer}，实际 ${JSON.stringify(document.servers[0]?.url)}`,
  );
}

function responseSchemaRef(response) {
  const schema = response?.content?.["application/json"]?.schema;
  return schema?.$ref;
}

function checkOperations(document, documentName, operations, requiredInventory) {
  const required = new Set(requiredInventory);
  const seenInventory = new Set();
  const operationIds = new Map();

  for (const { path, method, operation, pathItem } of operations) {
    const label = `${method} ${path}`;
    seenInventory.add(label);

    check(
      typeof operation.operationId === "string" &&
        /^[a-z][A-Za-z0-9]*$/.test(operation.operationId),
      `${label}: operationId 必须是唯一 camelCase，实际 ${JSON.stringify(operation.operationId)}`,
    );
    check(
      !operationIds.has(operation.operationId),
      `${label}: operationId 与 ${operationIds.get(operation.operationId)} 重复：${operation.operationId}`,
    );
    operationIds.set(operation.operationId, label);

    check(
      Array.isArray(operation.tags) && operation.tags.length > 0,
      `${label}: 缺少 tags`,
    );
    check(
      typeof operation.summary === "string" && operation.summary.trim().length > 0,
      `${label}: 缺少 summary`,
    );
    check(
      operation.responses !== null && typeof operation.responses === "object",
      `${label}: 缺少 responses`,
    );

    const successStatuses = ["200", "201", "204"].filter(
      (status) => operation.responses[status] !== undefined,
    );
    check(
      successStatuses.length > 0,
      `${label}: 缺少 200/201/204 成功响应`,
    );

    for (const status of ["200", "201"]) {
      const successResponse = operation.responses[status];
      if (successResponse === undefined) continue;
      const resolved = resolveLocal(
        document,
        successResponse,
        `${label} ${status} 成功响应`,
      );
      const content = resolved.content;
      check(
        content !== null &&
          typeof content === "object" &&
          Object.keys(content).length > 0,
        `${label}: ${status} 成功响应必须有 content`,
      );
      for (const [mediaType, media] of Object.entries(content)) {
        const schema = media?.schema;
        check(
          schema !== null &&
            typeof schema === "object" &&
            Object.keys(schema).length > 0,
          `${label}: ${status} ${mediaType} 响应 schema 不能为空`,
        );
        if (mediaType === "application/json") {
          check(
            typeof schema.$ref === "string" ||
              typeof schema.type === "string" ||
              Array.isArray(schema.anyOf) ||
              Array.isArray(schema.oneOf) ||
              Array.isArray(schema.allOf) ||
              schema.properties !== undefined ||
              schema.enum !== undefined ||
              schema.const !== undefined,
            `${label}: ${status} JSON 响应 schema 必须是具体 schema`,
          );
        }
      }
    }

    for (const [status, response] of Object.entries(operation.responses)) {
      const resolved = resolveLocal(document, response, `${label} ${status}`);
      check(
        typeof resolved.description === "string" && resolved.description.length > 0,
        `${label}: 响应 ${status} 缺少 description`,
      );
      if (!/^[45]\d\d$/.test(status)) continue;
      check(
        typeof response.$ref === "string" &&
          response.$ref.startsWith("#/components/responses/"),
        `${label}: 错误响应 ${status} 必须 $ref 共享响应组件`,
      );
      const schemaRef = responseSchemaRef(resolved);
      check(
        schemaRef === "#/components/schemas/ApiError",
        `${label}: 错误响应 ${status} 必须包装 ApiError，实际 ${JSON.stringify(schemaRef)}`,
      );
    }

    if (operation.operationId === "getOfficePreviewSource") {
      // No query or browser auth fallback; hide every authorization failure as 404.
      const scheme = document.components?.securitySchemes?.OfficeOutboxJWT;
      check(
        operation.security?.length === 1 &&
          Object.keys(operation.security[0]).length === 1 &&
          operation.security[0].OfficeOutboxJWT?.length === 0 &&
          scheme?.type === "http" && scheme.scheme === "bearer" &&
          scheme.bearerFormat === "JWT" &&
          ![...(pathItem.parameters ?? []), ...(operation.parameters ?? [])].some((parameter) =>
            resolveLocal(document, parameter, `${label} parameter`).in !== "path"
          ) &&
          operation.responses["401"] === undefined &&
          operation.responses["403"] === undefined &&
          operation.responses["404"] !== undefined,
        `${label}: Document Server 取源仅接受出站 JWT 请求头，授权失败统一返回 404`,
      );
    } else {
      check(
        operation.responses["401"] !== undefined,
        `${label}: 缺少 401 Unauthorized`,
      );
    }

    const response200Or201 = operation.responses["200"] ?? operation.responses["201"];
    if (response200Or201) {
      const resolved = resolveLocal(document, response200Or201, `${label} 成功响应`);
      const schemaRef = responseSchemaRef(resolved);
      if (schemaRef) {
        const schemaName = schemaRef.split("/").at(-1);
        if (ETAG_ENTITY_SCHEMAS.has(schemaName)) {
          check(
            resolved.headers?.ETag !== undefined,
            `${label}: ${schemaName} 响应必须声明 ETag 头`,
          );
        }
      }
    }

    const parameters = [];
    for (const [index, parameter] of [
      ...(pathItem.parameters ?? []),
      ...(operation.parameters ?? []),
    ].entries()) {
      parameters.push(
        resolveLocal(document, parameter, `${label} parameter[${index}]`),
      );
    }

    const csrfRequired =
      method !== "GET" &&
      !CSRF_EXEMPT_PATHS.has(path);
    if (csrfRequired) {
      const csrfParameter = parameters.find(
        (parameter) =>
          parameter.name === "X-CSRF-Token" && parameter.in === "header",
      );
      check(
        csrfParameter !== undefined,
        `${label}: 非 GET 操作必须声明 X-CSRF-Token`,
      );
      check(
        csrfParameter.required === false,
        `${label}: X-CSRF-Token 必须 required: false（同源中间件自动附加）`,
      );
      check(
        (operation.parameters ?? []).some(
          (parameter) => parameter.$ref === "#/components/parameters/CsrfToken",
        ),
        `${label}: CSRF 参数必须 $ref CsrfToken 组件`,
      );
    }

    const declaredPathParameters = new Map();
    for (const parameter of parameters) {
      check(
        typeof parameter.name === "string" && parameter.name.length > 0,
        `${label}: 参数缺少 name`,
      );
      check(
        ["path", "query", "header", "cookie"].includes(parameter.in),
        `${label}: 参数 ${parameter.name} 的 in 无效：${parameter.in}`,
      );
      if (parameter.in !== "path") continue;
      check(
        parameter.required === true,
        `${label}: 路径参数 ${parameter.name} 必须 required: true`,
      );
      check(
        parameter.schema?.type === "string" ||
          parameter.schema?.$ref !== undefined,
        `${label}: 路径参数 ${parameter.name} 缺少 schema`,
      );
      check(
        !declaredPathParameters.has(parameter.name),
        `${label}: 路径参数 ${parameter.name} 重复声明`,
      );
      declaredPathParameters.set(parameter.name, parameter);
    }

    const templateParameters = new Set(
      [...path.matchAll(/\{([^}]+)\}/g)].map((match) => match[1]),
    );
    for (const name of templateParameters) {
      check(
        declaredPathParameters.has(name),
        `${label}: 路径模板参数 {${name}} 未声明`,
      );
    }
    for (const name of declaredPathParameters.keys()) {
      check(
        templateParameters.has(name),
        `${label}: 声明了路径中不存在的参数 ${name}`,
      );
    }
  }

  for (const label of required) {
    check(seenInventory.has(label), `${documentName}: 缺少必需操作 ${label}`);
  }
  for (const label of seenInventory) {
    check(
      required.has(label),
      `${documentName}: 存在清单外操作 ${label}（不在 data-and-api 契约内）`,
    );
  }
}

function findOperation(operations, operationId) {
  return operations.find(({ operation }) => operation.operationId === operationId);
}

function checkParameter(document, { operation, path }, name, inValue, required) {
  const parameters = (operation.parameters ?? []).map((parameter) =>
    resolveLocal(document, parameter, `${operation.operationId} parameter`),
  );
  const parameter = parameters.find(
    (candidate) => candidate.name === name && candidate.in === inValue,
  );
  check(
    parameter !== undefined,
    `${operation.operationId} (${path}): 缺少 ${inValue} 参数 ${name}`,
  );
  check(
    parameter.required === required,
    `${operation.operationId} (${path}): ${name} required 必须为 ${required}`,
  );
}

function runChecks() {
  const browser = readYaml(join(API_DIR, BROWSER_DOCUMENT));
  checkInfo(browser, BROWSER_DOCUMENT, "/api/v1");

  const infoDescription = browser.info?.description;
  check(
    typeof infoDescription === "string" && infoDescription.length > 0,
    "openapi.yaml: info.description 必须存在",
  );
  for (const requiredText of [
    "/api/v1",
    "knoted_session",
    "HttpOnly",
    "Secure",
    "SameSite=Lax",
    "knoted_csrf",
    "X-CSRF-Token",
    "Origin",
    "expected_version",
    "Idempotency-Key",
  ]) {
    check(
      infoDescription.includes(requiredText),
      `openapi.yaml: info.description 缺少说明：${requiredText}`,
    );
  }
  check(
    /闲置\s*7\s*天/.test(infoDescription),
    "openapi.yaml: info.description 必须说明会话闲置 7 天过期",
  );
  check(
    /绝对\s*30\s*天/.test(infoDescription),
    "openapi.yaml: info.description 必须说明会话绝对 30 天过期",
  );
  check(
    /7\s*天过期/.test(infoDescription),
    "openapi.yaml: info.description 必须说明 Idempotency-Key 7 天过期",
  );

  const browserOperations = collectOperations(browser, BROWSER_DOCUMENT);
  checkOperations(
    browser,
    BROWSER_DOCUMENT,
    browserOperations,
    REQUIRED_BROWSER_OPERATIONS,
  );
  walkRefs(browser, browser, BROWSER_DOCUMENT);
  const browserSchemas = browser.components?.schemas ?? {};
  for (const [schemaName, expected] of Object.entries(CLOSED_ENUMS)) {
    const schema = browserSchemas[schemaName];
    check(
      schema !== undefined,
      `openapi.yaml: 缺少闭集枚举 schema ${schemaName}`,
    );
    check(
      schema.type === "string",
      `openapi.yaml#/components/schemas/${schemaName}: 枚举必须 type: string`,
    );
    check(
      Array.isArray(schema.enum),
      `openapi.yaml#/components/schemas/${schemaName}: 枚举必须显式列出 enum`,
    );
    check(
      JSON.stringify(schema.enum) === JSON.stringify(expected),
      `openapi.yaml#/components/schemas/${schemaName}: 枚举值不符，期望 ${expected.join("|")}，实际 ${schema.enum.join("|")}`,
    );
  }

  const dateValue = browserSchemas.DateValue;
  check(dateValue !== undefined, "openapi.yaml: 缺少 DateValue");
  check(
    dateValue.additionalProperties === false,
    "openapi.yaml#/components/schemas/DateValue: 必须 additionalProperties: false",
  );
  check(
    JSON.stringify(dateValue.required) ===
      JSON.stringify(["date", "time", "timezone"]),
    "openapi.yaml#/components/schemas/DateValue: required 必须精确为 date,time,timezone",
  );
  check(
    JSON.stringify(dateValue.properties?.time?.type) ===
      JSON.stringify(["string", "null"]),
    "openapi.yaml#/components/schemas/DateValue: time 必须为 [string,null]",
  );
  check(
    dateValue.properties?.time?.pattern === "^([01][0-9]|2[0-3]):[0-5][0-9]$",
    "openapi.yaml#/components/schemas/DateValue: time 必须使用 HH:mm 严格 pattern",
  );
  check(
    dateValue.properties?.timezone?.const === "Asia/Shanghai",
    "openapi.yaml#/components/schemas/DateValue: timezone 必须 const Asia/Shanghai",
  );

  for (const [schemaName, schema] of Object.entries(browserSchemas)) {
    if (
      !schemaName.endsWith("Page") &&
      !LIST_RESPONSE_SCHEMAS.has(schemaName)
    ) {
      continue;
    }
    check(
      schema.required?.includes("items"),
      `openapi.yaml#/components/schemas/${schemaName}: required 缺少 items`,
    );
    check(
      schema.required?.includes("next_cursor"),
      `openapi.yaml#/components/schemas/${schemaName}: required 缺少 next_cursor`,
    );
  }

  const noteSchema = browserSchemas.Note;
  check(
    noteSchema?.required?.includes("position"),
    "openapi.yaml#/components/schemas/Note: required 缺少 position",
  );
  check(
    noteSchema?.properties?.position?.type === "integer",
    "openapi.yaml#/components/schemas/Note: position 必须为 integer",
  );
  check(
    browserSchemas.NoteCreate?.properties?.position === undefined,
    "openapi.yaml#/components/schemas/NoteCreate: position 必须保持只读",
  );
  check(
    browserSchemas.NoteUpdate?.properties?.position === undefined,
    "openapi.yaml#/components/schemas/NoteUpdate: position 必须保持只读",
  );

  for (const schemaName of ["NoteUpdate", "TaskUpdate", "ProjectUpdate"]) {
    const schema = browserSchemas[schemaName];
    check(
      schema?.required?.includes("expected_version"),
      `openapi.yaml#/components/schemas/${schemaName}: required 缺少 expected_version`,
    );
    check(
      schema?.minProperties === 2,
      `openapi.yaml#/components/schemas/${schemaName}: 必须 minProperties: 2（至少一个变更字段）`,
    );
  }
  for (const schemaName of ["NoteMoveRequest", "NoteRestoreRequest"]) {
    const schema = browserSchemas[schemaName];
    check(
      schema?.required?.includes("expected_version"),
      `openapi.yaml#/components/schemas/${schemaName}: required 缺少 expected_version`,
    );
  }
  check(
    browserSchemas.NoteMoveRequest?.required?.includes("parent_id") &&
      browserSchemas.NoteMoveRequest?.required?.includes("position"),
    "openapi.yaml#/components/schemas/NoteMoveRequest: required 必须包含 parent_id/position",
  );

  const csrfParameterComponent = browser.components?.parameters?.CsrfToken;
  check(
    csrfParameterComponent?.name === "X-CSRF-Token" &&
      csrfParameterComponent?.in === "header" &&
      csrfParameterComponent?.required === false &&
      typeof csrfParameterComponent?.description === "string" &&
      csrfParameterComponent.description.includes("knoted_csrf") &&
      csrfParameterComponent.description.includes("Origin"),
    "openapi.yaml#/components/parameters/CsrfToken: 必须定义 X-CSRF-Token 头并与 knoted_csrf/Origin 关联",
  );

  for (const schemaName of EXPECTED_VERSION_SCHEMAS) {
    const schema = browser.components?.schemas?.[schemaName];
    check(
      schema !== undefined,
      `openapi.yaml: 缺少 schema ${schemaName}`,
    );
    check(
      schema.required?.includes("expected_version"),
      `openapi.yaml#/components/schemas/${schemaName}: required 缺少 expected_version`,
    );
  }

  for (const operationId of VERSIONED_DELETE_OPERATION_IDS) {
    const found = findOperation(browserOperations, operationId);
    check(found !== undefined, `openapi.yaml: 缺少删除操作 ${operationId}`);
    checkParameter(browser, found, "expected_version", "query", true);
  }

  for (const operationId of IDEMPOTENT_OPERATION_IDS) {
    const found = findOperation(browserOperations, operationId);
    check(found !== undefined, `openapi.yaml: 缺少幂等创建操作 ${operationId}`);
    const parameter = (found.operation.parameters ?? [])
      .map((entry) => resolveLocal(browser, entry, `${operationId} parameter`))
      .find((entry) => entry.name === "Idempotency-Key" && entry.in === "header");
    check(
      parameter?.required === true,
      `${operationId}: 必须要求 Idempotency-Key`,
    );
  }
  const createNote = findOperation(browserOperations, "createNote");
  check(
    (createNote.operation.parameters ?? []).every(
      (entry) => entry.$ref !== "#/components/parameters/IdempotencyKey",
    ),
    "createNote: 按契约不得要求 Idempotency-Key",
  );

  function listExampleFiles(directory) {
    const files = [];
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const fullPath = join(directory, entry.name);
      if (entry.isDirectory()) {
        files.push(...listExampleFiles(fullPath));
      } else if (entry.isFile()) {
        files.push(relative(EXAMPLES_DIR, fullPath).split("\\").join("/"));
      }
    }
    return files;
  }

  const manifest = readJson(join(EXAMPLES_DIR, "manifest.json"));
  check(
    manifest !== null && typeof manifest === "object",
    "api/examples/manifest.json: 根必须是对象",
  );
  check(
    Array.isArray(manifest.documents) &&
      manifest.documents.length === 1 && manifest.documents[0] === BROWSER_DOCUMENT,
    "api/examples/manifest.json: documents 只能登记 openapi.yaml",
  );
  check(
    Array.isArray(manifest.examples),
    "api/examples/manifest.json: examples 必须是数组",
  );

  const requiredByFile = new Map(
    REQUIRED_EXAMPLES.map((example) => [example.file, example]),
  );
  const manifestByFile = new Map();
  for (const example of manifest.examples) {
    check(
      example !== null && typeof example === "object",
      "api/examples/manifest.json: examples 条目必须是对象",
    );
    check(
      typeof example.file === "string" && example.file.length > 0,
      "api/examples/manifest.json: examples 条目缺少 file",
    );
    check(
      !manifestByFile.has(example.file),
      `api/examples/manifest.json: 样例文件重复登记：${example.file}`,
    );
    manifestByFile.set(example.file, example);
    check(
      requiredByFile.has(example.file),
      `api/examples/manifest.json: 清单外样例 ${example.file}（必须登记到 REQUIRED_EXAMPLES）`,
    );
    if (example.valid) {
      check(
        example.file.startsWith("valid/"),
        `api/examples/manifest.json: valid=true 的样例必须位于 valid/：${example.file}`,
      );
    } else {
      check(
        example.file.startsWith("invalid/"),
        `api/examples/manifest.json: valid=false 的样例必须位于 invalid/：${example.file}`,
      );
    }
  }
  for (const requiredExample of REQUIRED_EXAMPLES) {
    const listed = manifestByFile.get(requiredExample.file);
    check(
      listed !== undefined,
      `api/examples/manifest.json: 缺少必需样例 ${requiredExample.file}`,
    );
    check(
      listed.document === requiredExample.document &&
        listed.schema === requiredExample.schema &&
        listed.valid === requiredExample.valid,
      `api/examples/manifest.json: 必需样例登记不符 ${requiredExample.file}（期望 ${requiredExample.document}#${requiredExample.schema} valid=${requiredExample.valid}）`,
    );
    const requiredPath = resolve(EXAMPLES_DIR, requiredExample.file);
    let requiredStat;
    try {
      requiredStat = statSync(requiredPath);
    } catch {
      fail(`api/examples/${requiredExample.file}: 必需样例文件不存在`);
    }
    check(
      requiredStat.isFile(),
      `api/examples/${requiredExample.file}: 必需样例不是文件`,
    );
  }

  const listedFiles = new Set();
  const ajv = new Ajv2020({
    strict: false,
    allErrors: true,
    validateFormats: true,
  });
  addFormats(ajv);
  if (!ajv.formats.binary) {
    ajv.addFormat("binary", { validate: () => true });
  }
  if (!ajv.formats.int32) {
    ajv.addFormat("int32", {
      type: "number",
      validate: (value) => Number.isInteger(value),
    });
  }
  if (!ajv.formats.int64) {
    ajv.addFormat("int64", {
      type: "number",
      validate: (value) => Number.isInteger(value),
    });
  }

  let invalidRejected = 0;
  for (const [index, example] of manifest.examples.entries()) {
    const where = `api/examples/manifest.json examples[${index}]`;
    check(
      example !== null && typeof example === "object",
      `${where}: 必须是对象`,
    );
    check(
      example.document === BROWSER_DOCUMENT,
      `${where}: document 无效：${JSON.stringify(example.document)}`,
    );
    check(
      typeof example.schema === "string" && example.schema.length > 0,
      `${where}: schema 缺失`,
    );
    check(
      typeof example.file === "string" && example.file.length > 0,
      `${where}: file 缺失`,
    );
    check(typeof example.valid === "boolean", `${where}: valid 必须是布尔值`);

    const schema = browserSchemas[example.schema];
    check(
      schema !== undefined,
      `${where}: ${example.document} 缺少 #/components/schemas/${example.schema}`,
    );

    const examplePath = resolve(EXAMPLES_DIR, example.file);
    check(
      examplePath.startsWith(`${EXAMPLES_DIR}/`),
      `${where}: file 越出 api/examples：${example.file}`,
    );
    let stat;
    try {
      stat = statSync(examplePath);
    } catch {
      fail(`${where}: 样例文件不存在：api/examples/${example.file}`);
    }
    check(stat.isFile(), `${where}: 样例不是文件：api/examples/${example.file}`);
    check(
      !listedFiles.has(example.file),
      `${where}: 样例文件重复登记：${example.file}`,
    );
    listedFiles.add(example.file);

    const data = readJson(examplePath);
    let validate;
    try {
      validate = ajv.compile({
        components: browser.components,
        $ref: `#/components/schemas/${example.schema}`,
      });
    } catch (error) {
      fail(
        `${where}: Ajv 编译 ${example.document}#/components/schemas/${example.schema} 失败：${error.message}`,
      );
    }

    const accepted = validate(data);
    if (example.valid) {
      check(
        accepted,
        `${where}: 合法样例被拒绝：${example.file} → ${example.document}#/components/schemas/${example.schema}\n${ajv.errorsText(validate.errors, { separator: "\n" })}`,
      );
    } else {
      check(
        !accepted,
        `${where}: 非法样例被接受：${example.file} → ${example.document}#/components/schemas/${example.schema}`,
      );
      invalidRejected += 1;
    }
  }

  for (const directory of ["valid", "invalid"]) {
    const directoryPath = join(EXAMPLES_DIR, directory);
    let entries;
    try {
      entries = listExampleFiles(directoryPath);
    } catch {
      continue;
    }
    for (const file of entries) {
      check(
        listedFiles.has(file),
        `api/examples/${file}: 未登记到 manifest.json（孤立样例）`,
      );
    }
  }

  console.log(
    `api:check ok — ${browserOperations.length} operations, ${manifest.examples.length} examples (${invalidRejected} invalid rejected)`,
  );
}

try {
  runChecks();
} catch (error) {
  if (error instanceof ContractError) {
    console.error(`api:check failed — ${error.message}`);
    process.exit(1);
  }
  throw error;
}
