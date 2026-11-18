// Knoted 测试实例编排的场景测试（pnpm test:pc）。
// 覆盖：pcwrap/编排配置形状、固定产物校验、模板渲染、独立证书工具、API 端口占用、迁移失败传播。
import { spawnSync } from "node:child_process";
import {
  appendFileSync,
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { after, describe, it } from "node:test";
import assert from "node:assert/strict";

const repoRoot = resolve(import.meta.dirname, "../..");
const serviceScript = join(repoRoot, "tools/dev/pc/service.sh");
const checkCertificate = join(repoRoot, "tools/dev/pc/check-certificate.sh");
const revision = "0123456789abcdef0123456789abcdef01234567";

const temporary = [];

// 运行期工具由 NixOS 提供，测试里用"临时 profile 目录"代替，避免依赖调用者 PATH：
// openssl 只供独立证书工具测试使用；API prepare 不依赖 Caddy/证书。
function resolveTool(name) {
  const dirs = [
    ...(process.env.PATH ?? "").split(":"),
    "/nix/var/nix/profiles/system/sw/bin",
    "/run/current-system/sw/bin",
  ].filter(Boolean);
  for (const dir of dirs) {
    const candidate = join(dir, name);
    if (existsSync(candidate)) return candidate;
  }
  throw new Error(`测试需要 ${name}：PATH 与 boot profile 里都找不到`);
}

const emptyStaticPath = mkdtempSync(join(tmpdir(), "knoted-pc-static-"));
temporary.push(emptyStaticPath);
const toolProfile = mkdtempSync(join(tmpdir(), "knoted-pc-tools-"));
temporary.push(toolProfile);
symlinkSync(resolveTool("openssl"), join(toolProfile, "openssl"));

function makeTempDir(label) {
  const dir = mkdtempSync(join(tmpdir(), `knoted-pc-${label}-`));
  temporary.push(dir);
  return dir;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    timeout: 60_000,
    ...options,
  });
  if (result.error) {
    return {
      ...result,
      status: -1,
      stdout: result.stdout ?? "",
      stderr: `${command}: ${result.error.message}`,
    };
  }
  return result;
}

async function freePort() {
  const server = createServer();
  await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
  const { port } = server.address();
  await new Promise((resolveClose) => server.close(resolveClose));
  return port;
}

function writeExecutable(path, contents) {
  writeFileSync(path, contents);
  chmodSync(path, 0o755);
}

// 用与 tools/dev/build-artifact.sh 相同的命令生成清单，避免重新实现排序规则。
function writeManifest(root) {
  const result = run(
    "bash",
    [
      "-c",
      "find bin share -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS",
    ],
    { cwd: root, env: { ...process.env, LC_ALL: "C" } },
  );
  assert.equal(result.status, 0, `manifest fixture failed: ${result.stderr}`);
}

function createRuntimeFixture() {
  const root = join(makeTempDir("runtime"), "current");
  mkdirSync(join(root, "bin"), { recursive: true });
  mkdirSync(join(root, "share/knoted/web"), { recursive: true });
  mkdirSync(join(root, "share/knoted/migrations"), { recursive: true });
  writeExecutable(join(root, "bin/knoted-api"), "#!/usr/bin/env bash\nexit 0\n");
  writeExecutable(join(root, "bin/knoted-worker"), "#!/usr/bin/env bash\nexit 0\n");
  writeFileSync(join(root, "share/knoted/web/index.html"), "<!doctype html><title>knoted</title>\n");
  writeFileSync(join(root, "share/knoted/migrations/0001_registry.sql"), "select 1;\n");
  writeFileSync(join(root, "share/knoted/REVISION"), `${revision}\n`);
  // Only integrity fixtures: no Python computation is started by prepare.
  writeExecutable(join(root, "bin/knoted-intelligence"), "#!/usr/bin/env bash\nexit 0\n");
  const pythonRoot = join(root, "share/knoted/intelligence");
  mkdirSync(join(pythonRoot, "python/bin"), { recursive: true });
  mkdirSync(join(pythonRoot, "python/lib/python3.12/site-packages/knoted_intelligence"), { recursive: true });
  writeExecutable(join(pythonRoot, "python/bin/python3.12"), '#!/usr/bin/env bash\nexec python3 "$@"\n');
  for (const file of [
    "BUILD.json",
    "requirements.txt",
    "uv.lock",
    "python/lib/python3.12/os.py",
    "python/lib/python3.12/site-packages/knoted_intelligence/app.py",
    "python/lib/python3.12/site-packages/knoted_intelligence/guardian.py",
    "python/lib/python3.12/site-packages/knoted_intelligence/supervision.py",
    "python/lib/python3.12/site-packages/knoted_intelligence/parsing/pdf.py",
    "python/lib/python3.12/site-packages/docling/document_converter.py",
    "python/lib/python3.12/site-packages/torch/__init__.py",
    "python/lib/python3.12/site-packages/onnxruntime/__init__.py",
  ]) {
    mkdirSync(resolve(join(pythonRoot, file), ".."), { recursive: true });
    writeFileSync(join(pythonRoot, file), "integrity fixture\n");
  }
  writeManifest(root);
  return root;
}

function createCertificateFixture(hostname) {
  const dir = makeTempDir("cert");
  const result = run(resolveTool("openssl"), [
    "req",
    "-x509",
    "-newkey",
    "rsa:2048",
    "-nodes",
    "-days",
    "2",
    "-subj",
    `/CN=${hostname}`,
    "-addext",
    `subjectAltName=DNS:${hostname}`,
    "-keyout",
    join(dir, "key.pem"),
    "-out",
    join(dir, "fullchain.pem"),
  ]);
  assert.equal(result.status, 0, `openssl fixture failed: ${result.stderr}`);
  return dir;
}

// prepareEnv 关闭 boot profile 兜底，让每个用例自己决定工具来源。
function prepareEnv({ runtimeRoot, instanceRoot, certDir, apiPort, httpsPort, intelligencePort, domain }, extra = {}) {
  return {
    ...process.env,
    // 默认只暴露临时 profile（真实 openssl）；/run/current-system 兜底也关掉，
    // 否则切换后（工具装进系统 PATH）"缺依赖"的负向断言会失效。
    KNOTED_SYSTEM_PROFILE_BIN: extra.KNOTED_SYSTEM_PROFILE_BIN ?? toolProfile,
    KNOTED_STATIC_PATH: extra.KNOTED_STATIC_PATH ?? emptyStaticPath,
    KNOTED_INSTANCE_ROOT: instanceRoot,
    KNOTED_RUNTIME_ROOT: runtimeRoot,
    KNOTED_CERT_DIR: certDir,
    KNOTED_API_PORT: String(apiPort),
    KNOTED_INTELLIGENCE_PORT: String(intelligencePort),
    KNOTED_HTTPS_PORT: String(httpsPort),
    KNOTED_HTTPS_DOMAIN: domain,
    KNOTED_LOG_LEVEL: "debug",
    KNOTED_SEARCH_ENABLED: "",
    KNOTED_SEARCH_CREDENTIAL_FILE: "",
    KNOTED_OFFICE_ORIGIN: "",
    KNOTED_OFFICE_JWT_SECRET_FILE: "",
    DATABASE_URL: "postgres://dev@127.0.0.1:5432/dev",
    ...extra,
  };
}

async function fullFixture() {
  const base = makeTempDir("instance");
  const runtimeRoot = createRuntimeFixture();
  const certDir = createCertificateFixture("knoted.example.test");
  const apiPort = await freePort();
  const httpsPort = await freePort();
  const intelligencePort = await freePort();
  return {
    base,
    runtimeRoot,
    certDir,
    apiPort,
    httpsPort,
    intelligencePort,
    domain: "knoted.example.test",
    instanceRoot: join(base, "service"),
  };
}

after(() => {
  for (const dir of temporary) rmSync(dir, { recursive: true, force: true });
});

describe("编排配置", () => {
  it("pcwrap 固定控制端口、日志与项目名", () => {
    const config = JSON.parse(readFileSync(join(repoRoot, "pcwrap.config.json"), "utf8"));
    assert.equal(config.name, "knoted");
    assert.equal(config.port, 15600);
    assert.equal(config.configFile, "process-compose.yaml");
    assert.equal(config.logFile, "/data/knoted-dev/service/logs/process-compose.log");
    assert.equal(config.keepProject, true);
    assert.equal(config.profiles, undefined, "不定义 prod profile");
  });

  it("编排只含测试应用进程，控制端口不写进 yaml", () => {
    const yaml = readFileSync(join(repoRoot, "process-compose.yaml"), "utf8");
    for (const name of ["prepare", "migrate", "knoted-api", "knoted-worker"]) {
      assert.match(yaml, new RegExp(`\\n  ${name}:`), `缺少进程 ${name}`);
    }
    assert.doesNotMatch(yaml, /^\s*port:/m, "process-compose.yaml 不接受 port 字段，控制端口由 pcwrap 注入");
    const processNames = yaml
      .split("\n")
      .filter((line) => /^  [a-z][a-z0-9-]*:$/.test(line))
      .map((line) => line.trim().replace(":", ""));
    assert.deepEqual(
      processNames.sort(),
      ["knoted-api", "knoted-intelligence", "knoted-worker", "migrate", "prepare"],
      "外部依赖（PostgreSQL/Strata）不得进编排",
    );
    assert.match(yaml, /condition: process_completed_successfully/);
  });
});

describe("prepare", () => {
  it("仅按显式开关启用本机全文检索，配置只保存凭据路径", async () => {
    const fixture = await fullFixture();
    const result = run("bash", [serviceScript, "prepare"], {
      env: prepareEnv(fixture, {
        KNOTED_SEARCH_ENABLED: "1",
        KNOTED_SEARCH_CREDENTIAL_FILE: "/run/knoted/search.key",
      }),
    });
    assert.equal(result.status, 0, result.stderr);
    const config = readFileSync(join(fixture.instanceRoot, "config/knoted.toml"), "utf8");
    assert.match(config, /\[credentials\]\ncredential_file = "\/run\/knoted\/search.key"/);
    assert.match(config, /\[search\]\nenabled = true\napi_key_credential = "meilisearch"/);
  });

  it("只渲染应用配置，不生成 HTTPS 配置或 reload 入口", async () => {
    const fixture = await fullFixture();
    const result = run(
      "bash",
      [serviceScript, "prepare"],
      { env: prepareEnv(fixture) },
    );
    assert.equal(result.status, 0, result.stderr);

    const toml = readFileSync(join(fixture.instanceRoot, "config/knoted.toml"), "utf8");
    assert.match(toml, new RegExp(`bind = "127\\.0\\.0\\.1:${fixture.apiPort}"`));
    assert.match(toml, /rp_id = "knoted\.example\.test"/);
    assert.ok(toml.includes(`root = "${fixture.instanceRoot}/assets"`), "asset storage follows the isolated instance root");
    assert.match(toml, new RegExp(`origin = "https://knoted\\.example\\.test:${fixture.httpsPort}"`));
    assert.match(toml, /url = "postgres:\/\/dev@127\.0\.0\.1:5432\/dev"/);
    assert.doesNotMatch(toml, /@[A-Z_]+@/);
    assert.ok(toml.includes('root = "/data/vaultsyncd-dev/dev-server/vaults/knoted-test"'));
    assert.ok(toml.includes(`staging_root = "${fixture.instanceRoot}/staging"`));
    assert.match(toml, /\[files\]\nenabled = true/);
    assert.doesNotMatch(toml, /^\[office\]$/m);

    assert.equal(statSync(join(fixture.instanceRoot, "config/knoted.toml")).mode & 0o777, 0o600);
    assert.equal(statSync(join(fixture.instanceRoot, "state")).mode & 0o777, 0o700);
    assert.equal(existsSync(join(fixture.instanceRoot, "config/Caddyfile")), false);
    assert.equal(existsSync(join(fixture.instanceRoot, "bin/knoted-https-reload")), false);

  });

  it("仅用可信来源和绝对凭据路径启用 Office；不打开凭据文件", async () => {
    const fixture = await fullFixture();
    const secretFile = join(fixture.base, "not-created test input");
    const result = run("bash", [serviceScript, "prepare"], {
      env: prepareEnv(fixture, {
        KNOTED_OFFICE_ORIGIN: "https://docs.example.test:19443",
        KNOTED_OFFICE_JWT_SECRET_FILE: secretFile,
      }),
    });
    assert.equal(result.status, 0, result.stderr);
    const toml = readFileSync(join(fixture.instanceRoot, "config/knoted.toml"), "utf8");
    assert.ok(toml.includes('[office]\ndocument_server_origin = "https://docs.example.test:19443"'));
    assert.ok(toml.includes(`jwt_secret_file = "${secretFile}"`));
    assert.equal(existsSync(secretFile), false);
  });

  it("拒绝不完整或会破坏 TOML 的 Office 配置", async () => {
    const fixture = await fullFixture();
    for (const path of ["", "relative", '/path"\n[extra]']) {
      const result = run("bash", [serviceScript, "prepare"], {
        env: prepareEnv(fixture, {
          KNOTED_OFFICE_ORIGIN: "https://docs.example.test:19443",
          KNOTED_OFFICE_JWT_SECRET_FILE: path,
        }),
      });
      assert.equal(result.status, 1);
      assert.equal(existsSync(join(fixture.instanceRoot, "config/knoted.toml")), false);
    }
  });

  it("拒绝被篡改或清单不完整的固定产物", async () => {
    const fixture = await fullFixture();
    appendFileSync(join(fixture.runtimeRoot, "share/knoted/web/index.html"), "<!-- tampered -->\n");
    const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /SHA256SUMS/);
    assert.equal(
      readdirSync(fixture.base).includes("service"),
      false,
      "校验失败时不应创建实例目录",
    );
  });

  it("宿主证书缺失时仍可准备 API", async () => {
    const fixture = await fullFixture();
    rmSync(join(fixture.certDir, "fullchain.pem"));
    const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    assert.equal(result.status, 0, result.stderr);
  });

  it("宿主 HTTPS 已监听时仍能准备 API", async () => {
    const fixture = await fullFixture();
    const blocker = createServer();
    await new Promise((resolveListen) => blocker.listen(fixture.httpsPort, "127.0.0.1", resolveListen));
    try {
      const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
      assert.equal(result.status, 0, result.stderr);
    } finally {
      await new Promise((resolveClose) => blocker.close(resolveClose));
    }
  });

  it("端口被占用时拒绝启动", async () => {
    const fixture = await fullFixture();
    const blocker = createServer();
    await new Promise((resolveListen) => blocker.listen(fixture.apiPort, "127.0.0.1", resolveListen));
    try {
      const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
      assert.equal(result.status, 1);
      assert.match(result.stderr, new RegExp(`端口 ${fixture.apiPort} 已被占用`));
    } finally {
      await new Promise((resolveClose) => blocker.close(resolveClose));
    }
  });
});

describe("证书校验", () => {
  it("接受匹配 hostname 的证书", () => {
    const certDir = createCertificateFixture("knoted.example.test");
    const result = run(
      "bash",
      [
        checkCertificate,
        join(certDir, "fullchain.pem"),
        join(certDir, "key.pem"),
        "knoted.example.test",
      ],
      { env: { ...process.env, PATH: `${toolProfile}:${process.env.PATH}` } },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "", "成功时不打印证书内容");
  });

  it("拒绝 hostname 不匹配的证书", () => {
    const certDir = createCertificateFixture("knoted.example.test");
    const result = run(
      "bash",
      [
        checkCertificate,
        join(certDir, "fullchain.pem"),
        join(certDir, "key.pem"),
        "wlab.example.test",
      ],
      { env: { ...process.env, PATH: `${toolProfile}:${process.env.PATH}` } },
    );
    assert.equal(result.status, 1);
    assert.match(result.stderr, /hostname 不匹配/);
  });
});

describe("运行期依赖", () => {
  it("PATH 没有 TLS 工具也能准备 API", async () => {
    const fixture = await fullFixture();
    const restricted = makeTempDir("api-only-path");
    for (const tool of ["bash", "dirname", "readlink", "cat", "find", "sort", "xargs", "sha256sum", "cmp", "curl", "ss", "install", "mktemp", "chmod", "mv", "grep"]) {
      symlinkSync(resolveTool(tool), join(restricted, tool));
    }
    const result = run(resolveTool("bash"), [serviceScript, "prepare"], {
      env: prepareEnv(fixture, { PATH: restricted, KNOTED_SYSTEM_PROFILE_BIN: emptyStaticPath }),
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(existsSync(join(fixture.instanceRoot, "config/knoted.toml")), true);
  });

  it("缺少工具时在写配置文件之前失败", async () => {
    const fixture = await fullFixture();
    const restricted = makeTempDir("restricted-path");
    for (const tool of ["bash", "dirname", "readlink", "cat", "find", "sort", "xargs", "sha256sum", "cmp"]) {
      symlinkSync(resolveTool(tool), join(restricted, tool));
    }
    const result = run(resolveTool("bash"), [serviceScript, "prepare"], {
      env: prepareEnv(fixture, { PATH: restricted, KNOTED_SYSTEM_PROFILE_BIN: emptyStaticPath }),
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /缺少运行期依赖/);
    assert.equal(existsSync(join(fixture.instanceRoot, "config")), false, "不应留下半个实例");
  });
});

describe("migrate", () => {
  it("把固定产物的 migrations 交给 sqlx，并传播失败退出码", async () => {
    const fixture = await fullFixture();
    run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });

    const stubDir = makeTempDir("stub");
    const record = join(stubDir, "args.txt");
    writeExecutable(
      join(stubDir, "sqlx"),
      `#!/usr/bin/env bash\nprintf '%s\\n' "$*" > ${JSON.stringify(record)}\nexit 0\n`,
    );
    const okEnv = { ...prepareEnv(fixture), PATH: `${stubDir}:${process.env.PATH}` };
    const ok = run("bash", [serviceScript, "migrate"], { env: okEnv });
    assert.equal(ok.status, 0, ok.stderr);
    assert.match(
      readFileSync(record, "utf8"),
      new RegExp(`migrate run --source ${fixture.runtimeRoot}/share/knoted/migrations`),
    );

    writeExecutable(join(stubDir, "sqlx"), "#!/usr/bin/env bash\nexit 7\n");
    const failed = run("bash", [serviceScript, "migrate"], { env: okEnv });
    assert.equal(failed.status, 7, "迁移失败必须原样传播退出码");
    assert.match(failed.stderr, /createdb -h 127\.0\.0\.1 knoted_dev/, "失败时给出建库命令");
  });

  it("缺少 sqlx 时给出明确错误而不是静默跳过", async () => {
    const fixture = await fullFixture();
    run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    // A real host sqlx must not turn this missing-tool case into a database call.
    const restricted = makeTempDir("no-sqlx-path");
    for (const tool of ["bash", "dirname", "readlink", "cat", "find", "sort", "xargs", "sha256sum", "cmp"]) {
      symlinkSync(resolveTool(tool), join(restricted, tool));
    }
    const result = run(resolveTool("bash"), [serviceScript, "migrate"], {
      env: prepareEnv(fixture, { PATH: restricted, KNOTED_SYSTEM_PROFILE_BIN: emptyStaticPath }),
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /sqlx-cli/);
  });
});

describe("paths", () => {
  it("默认指向本机测试实例", () => {
    const env = { ...process.env };
    for (const key of Object.keys(env)) {
      if (key.startsWith("KNOTED_") || key === "DATABASE_URL") delete env[key];
    }
    const result = run("bash", [serviceScript, "paths"], { env });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /^instance_root=\/data\/knoted-dev\/service$/m);
    assert.match(result.stdout, /^api_port=15602$/m);
    assert.match(result.stdout, /^https_port=15601$/m);
    assert.match(result.stdout, /^domain=wlab\.132140\.xyz$/m);
  });
});

describe("数据库归属", () => {
  it("默认连接串用宿主用户，不再有专用角色", async () => {
    const fixture = await fullFixture();
    const env = prepareEnv(fixture);
    delete env.DATABASE_URL;
    const result = run("bash", [serviceScript, "prepare"], { env });
    assert.equal(result.status, 0, result.stderr);
    const toml = readFileSync(join(fixture.instanceRoot, "config/knoted.toml"), "utf8");
    assert.match(toml, /url = "postgres:\/\/xingjian@127\.0\.0\.1:5432\/knoted_dev"/);
    assert.doesNotMatch(toml, /knoted_dev@/);

    // 迁移实际拿到的连接串也必须是同一个默认值（不是只渲染对了 TOML）。
    const stubDir = makeTempDir("default-url-stub");
    const record = join(stubDir, "database-url.txt");
    writeExecutable(
      join(stubDir, "sqlx"),
      `#!/usr/bin/env bash\nprintf '%s' "$DATABASE_URL" > ${JSON.stringify(record)}\nexit 0\n`,
    );
    const migrated = run("bash", [serviceScript, "migrate"], {
      env: { ...env, PATH: `${stubDir}:${env.PATH}` },
    });
    assert.equal(migrated.status, 0, migrated.stderr);
    assert.equal(
      readFileSync(record, "utf8"),
      "postgres://xingjian@127.0.0.1:5432/knoted_dev",
      "migrate 的默认连接串必须与模板一致",
    );
  });
});

describe("渲染与校验边界", () => {
  it("DATABASE_URL 里的 & 原样保留（bash patsub_replacement）", async () => {
    const fixture = await fullFixture();
    const env = prepareEnv(fixture, {
      DATABASE_URL: "postgres://dev:p&ss@127.0.0.1:5432/dev",
    });
    const result = run("bash", [serviceScript, "prepare"], { env });
    assert.equal(result.status, 0, result.stderr);
    const toml = readFileSync(join(fixture.instanceRoot, "config/knoted.toml"), "utf8");
    assert.match(toml, /url = "postgres:\/\/dev:p&ss@127\.0\.0\.1:5432\/dev"/);
  });

  it("拒绝运行根里的符号链接", async () => {
    const fixture = await fullFixture();
    symlinkSync("index.html", join(fixture.runtimeRoot, "share/knoted/web/alias.html"));
    const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /非普通文件/);
  });

  it("拒绝缺少 REVISION 的运行根", async () => {
    const fixture = await fullFixture();
    rmSync(join(fixture.runtimeRoot, "share/knoted/REVISION"));
    const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /REVISION/);
  });

  it("拒绝从任务工作树运行稳定实例", async () => {
    const fixture = await fullFixture();
    const env = prepareEnv(fixture, {
      KNOTED_INSTANCE_ROOT: join(fixture.base, ".worktrees/task/service"),
    });
    const result = run("bash", [serviceScript, "prepare"], { env });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /\.worktrees/);
  });

  it("paths 不回显数据库密码", () => {
    const result = run("bash", [serviceScript, "paths"], {
      env: {
        ...process.env,
        DATABASE_URL: "postgres://dev:topsecret@127.0.0.1:5432/dev",
      },
    });
    assert.equal(result.status, 0, result.stderr);
    // 掩码覆盖 userinfo 整段（user 也可能是敏感信息）
    assert.match(result.stdout, /database_url=postgres:\/\/\*\*\*@127\.0\.0\.1:5432\/dev/);
    assert.doesNotMatch(result.stdout, /topsecret/);
  });

  it("拒绝不是 PEM 的证书文件", () => {
    const certDir = makeTempDir("badcert");
    writeFileSync(join(certDir, "fullchain.pem"), "not a certificate\n");
    writeFileSync(join(certDir, "key.pem"), "not a key\n");
    const result = run(
      "bash",
      [
        checkCertificate,
        join(certDir, "fullchain.pem"),
        join(certDir, "key.pem"),
        "knoted.example.test",
      ],
      { env: { ...process.env, PATH: `${toolProfile}:${process.env.PATH}` } },
    );
    assert.equal(result.status, 1);
    assert.match(result.stderr, /不是有效 PEM 证书/);
  });
});

describe("仓库入口", () => {
  it("package.json 暴露 pc 与 test:pc 脚本", () => {
    const pkg = JSON.parse(readFileSync(join(repoRoot, "package.json"), "utf8"));
    assert.equal(pkg.scripts.pc, "pcwrap");
    assert.match(pkg.scripts["test:pc"], /node --test/);
    assert.equal(pkg.devDependencies["@bixlab/pcwrap"], "0.1.0");
  });
});

// The service entry must execute the fixed Worker with the rendered config.
describe("T25 Worker", () => {
  it("worker 入口传入配置并传播退出码", async () => {
    const fixture = await fullFixture();
    const env = prepareEnv(fixture);
    const prepared = run("bash", [serviceScript, "prepare"], { env });
    assert.equal(prepared.status, 0, prepared.stderr);
    const record = join(fixture.base, "worker-args.txt");
    writeExecutable(join(fixture.runtimeRoot, "bin/knoted-worker"), '#!/usr/bin/env bash\nprintf "%s\\n" "$@" > "$KNOTED_WORKER_ARGS"\nexit 7\n');
    const result = run("bash", [serviceScript, "worker"], { env: { ...env, KNOTED_WORKER_ARGS: record } });
    assert.equal(result.status, 7, result.stderr);
    assert.deepEqual(readFileSync(record, "utf8").trim().split("\n"), ["--config", join(fixture.instanceRoot, "config/knoted.toml")]);
  });
  it("缺失 Worker 的产物在 prepare 之前拒绝", async () => {
    const fixture = await fullFixture();
    rmSync(join(fixture.runtimeRoot, "bin/knoted-worker"));
    writeManifest(fixture.runtimeRoot);
    const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /bin\/knoted-worker/);
    assert.equal(existsSync(join(fixture.instanceRoot, "config")), false);
  });
});

describe("Python 固定产物", () => {
  for (const file of [
    "bin/knoted-intelligence",
    "share/knoted/intelligence/python/bin/python3.12",
    "share/knoted/intelligence/python/lib/python3.12/os.py",
    "share/knoted/intelligence/python/lib/python3.12/site-packages/knoted_intelligence/app.py",
    "share/knoted/intelligence/python/lib/python3.12/site-packages/docling/document_converter.py",
    "share/knoted/intelligence/python/lib/python3.12/site-packages/torch/__init__.py",
    "share/knoted/intelligence/python/lib/python3.12/site-packages/onnxruntime/__init__.py",
    "share/knoted/intelligence/BUILD.json",
    "share/knoted/intelligence/requirements.txt",
    "share/knoted/intelligence/uv.lock",
  ]) {
    it(`重新生成清单也不能掩盖缺少 ${file}`, async () => {
      const fixture = await fullFixture();
      rmSync(join(fixture.runtimeRoot, file), { force: true });
      writeManifest(fixture.runtimeRoot);
      const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /Python/);
    });
  }
});

describe("T47 intelligence", () => {
  it("固定解释器进程、ready依赖及有界停止配置", () => {
    const yaml = readFileSync(join(repoRoot, "process-compose.yaml"), "utf8");
    assert.match(yaml, /\n  knoted-intelligence:/);
    assert.match(yaml, /command: "exec bash tools\/dev\/pc\/service.sh intelligence"/);
    assert.match(yaml, /condition: process_healthy/);
    assert.match(yaml, /timeout_seconds: 30/);
  });
  it("degraded JSON 不满足 intelligence readiness", async () => {
    const fixture = await fullFixture();
    const stub = makeTempDir("health-stub");
    writeExecutable(join(stub, "curl"), "#!/usr/bin/env bash\necho '{\"status\":\"degraded\",\"active_executions\":0}'\n");
    const result = run("bash", [serviceScript, "probe-intelligence"], {
      env: prepareEnv(fixture, { PATH: `${stub}:${process.env.PATH}` }),
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /degraded|未就绪/);
  });
});
