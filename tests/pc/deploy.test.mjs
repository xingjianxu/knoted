// dev 一键部署脚本的场景测试（pnpm test:pc）。
// 覆盖：迁移判定驱动 down+up 或 restart、已最新空转、构建失败不动指针、
// 退役前置检查、切换后验证失败复位指针、deploy.env 抓取与权限、.worktrees 路径守卫。
//
// 脚本不复制 build-artifact.sh / pcwrap / service.sh 的逻辑，测试因此用
// PATH 前置 stub（pnpm、curl）与显式 seam（产物父目录、/proc 根、构建命令）
// 模拟真实副作用，而不是等待一次几分钟的真实构建。
import { spawnSync } from "node:child_process";
import {
  appendFileSync,
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { resolve } from "node:path";
import { after, describe, it } from "node:test";
import assert from "node:assert/strict";

const worktree = resolve(import.meta.dirname, "../..");
const previousRevision = "0".repeat(39) + "1";

const temporary = [];
after(() => {
  for (const dir of temporary) rmSync(dir, { recursive: true, force: true });
});

function makeTempDir(label) {
  const dir = mkdtempSync(join(tmpdir(), `knoted-deploy-${label}-`));
  temporary.push(dir);
  return dir;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    timeout: 120_000,
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

function releaseRepository() {
  const root = makeTempDir("git");
  const git = (...args) => {
    const result = run("git", ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "-c", "commit.gpgsign=false", ...args], { cwd: root });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  git("init", "--quiet");
  mkdirSync(join(root, "services/intelligence"), { recursive: true });
  writeFileSync(join(root, "services/intelligence/pyproject.toml"), "[project]\nname = \"legacy-fixture\"\n");
  git("add", ".");
  git("commit", "--quiet", "-m", "legacy release");
  const legacyRevision = git("rev-parse", "HEAD");

  rmSync(join(root, "services"), { recursive: true });
  for (const relative of [
    "tools/dev/build-artifact.sh", "tools/dev/deploy-dev.sh",
    "tools/dev/pc/service.sh", "tools/dev/artifact-generation", "process-compose.yaml",
  ]) {
    mkdirSync(resolve(join(root, relative), ".."), { recursive: true });
    cpSync(join(worktree, relative), join(root, relative));
  }
  git("add", "--all");
  git("commit", "--quiet", "-m", "current release");
  return { repoRoot: root, legacyRevision, targetRevision: git("rev-parse", "HEAD") };
}

const { repoRoot, legacyRevision, targetRevision } = releaseRepository();
const deployScript = join(repoRoot, "tools/dev/deploy-dev.sh");

function writeExecutable(path, contents) {
  mkdirSync(resolve(path, ".."), { recursive: true });
  writeFileSync(path, contents);
  chmodSync(path, 0o755);
}

// 与 tools/dev/build-artifact.sh 相同的命令生成清单，避免测试自己实现排序规则。
function writeManifest(root) {
  const result = run("bash", ["-c", "find bin share -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS"], {
    cwd: root,
    env: { ...process.env, LC_ALL: "C" },
  });
  assert.equal(result.status, 0, `manifest fixture failed: ${result.stderr}`);
}

// build-artifact.sh 的产物形状：只提供部署脚本实际校验的条目。
function createArtifact(parent, revision, migrations = ["0001_registry.sql"], { legacy = false } = {}) {
  const root = join(parent, revision);
  mkdirSync(join(root, "bin"), { recursive: true });
  mkdirSync(join(root, "share/knoted/web"), { recursive: true });
  mkdirSync(join(root, "share/knoted/migrations"), { recursive: true });
  for (const name of ["knoted-api", "knoted-worker"]) {
    writeExecutable(join(root, `bin/${name}`), "#!/usr/bin/env bash\nexit 0\n");
  }
  if (legacy) {
    for (const image of ["bin/knoted-intelligence", processImages["knoted-intelligence"]]) {
      writeExecutable(join(root, image), "#!/usr/bin/env bash\nexit 0\n");
    }
  }
  writeFileSync(join(root, "share/knoted/web/index.html"), "<!doctype html><title>knoted</title>\n");
  for (const migration of migrations) {
    writeFileSync(join(root, `share/knoted/migrations/${migration}`), "select 1;\n");
  }
  writeFileSync(join(root, "share/knoted/REVISION"), `${revision}\n`);
  writeManifest(root);
  return root;
}

// 真实实例里 api/worker 直接跑 Rust 二进制，intelligence 的 bash 包装器 exec 固定解释器，
// 所以 /proc/exe 看到的是产物里的 python3.12。
const processImages = {
  "knoted-api": "bin/knoted-api",
  "knoted-worker": "bin/knoted-worker",
  "knoted-intelligence": "share/knoted/intelligence/python/bin/python3.12",
};

// 真实的 /proc/<pid> 同时有世界可读的 cmdline/status 与受 ptrace 校验的 exe。
// service.sh 在 exec 前把运行根解析成具体产物目录，所以 cmdline 的 argv[0] 里带 <sha>。
function createProcFixture(procRoot, artifactRoot) {
  Object.values(processImages).forEach((image, index) => {
    if (!existsSync(join(artifactRoot, image))) return;
    const pidDir = join(procRoot, String(1000 + index));
    mkdirSync(pidDir, { recursive: true });
    symlinkSync(join(artifactRoot, image), join(pidDir, "exe"));
    writeFileSync(join(pidDir, "cmdline"), `${join(artifactRoot, image)}\0`);
    writeFileSync(
      join(pidDir, "status"),
      `Name:\tknoted\nUid:\t${process.getuid()}\t${process.getuid()}\t${process.getuid()}\t${process.getuid()}\n` +
        `Gid:\t${process.getgid()}\t${process.getgid()}\t${process.getgid()}\t${process.getgid()}\n`,
    );
  });
  // 生产实例跑在 Nix store 里，必须不被 dev 部署的进程校验误伤。
  mkdirSync(join(procRoot, "2000"), { recursive: true });
  symlinkSync("/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-knoted/bin/knoted-api", join(procRoot, "2000", "exe"));
  writeFileSync(
    join(procRoot, "2000", "cmdline"),
    "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-knoted/bin/knoted-api\0--config\0/run/knoted-prod-config/app.toml\0",
  );
}

// 内核解析 /proc/<pid>/exe 要求调用者与目标 uid+gid 一致（PTRACE_MODE_READ_FSCREDS）。
// 删掉 exe 模拟跨组：只有 cmdline 可用。
function removeProcImages(procRoot) {
  for (const entry of readdirSync(procRoot)) {
    rmSync(join(procRoot, entry, "exe"), { force: true });
  }
}

// 让假进程的主组与测试运行者不同，触发 gid 诊断。
function setProcGid(procRoot, gid) {
  for (const entry of readdirSync(procRoot)) {
    const statusFile = join(procRoot, entry, "status");
    if (!existsSync(statusFile)) continue;
    writeFileSync(statusFile, `Name:\tknoted\nUid:\t${process.getuid()}\t${process.getuid()}\t${process.getuid()}\t${process.getuid()}\nGid:\t${gid}\t${gid}\t${gid}\t${gid}\n`);
  }
}

// 未解析运行根的布局：cmdline 只剩 <runtime_parent>/current/<image>，判定不到具体产物。
// 只改这实例运行根下的进程；/nix/store 里的生产实例同理要以 bin/knoted-api 结尾，不能误伤。
function unresolveProcCmdlines(procRoot, runtimeParentDir) {
  for (const entry of readdirSync(procRoot)) {
    const cmdlineFile = join(procRoot, entry, "cmdline");
    if (!existsSync(cmdlineFile)) continue;
    const argv0 = readFileSync(cmdlineFile, "utf8").replace(/\0.*$/s, "");
    if (!argv0.startsWith(`${runtimeParentDir}/`)) continue;
    const image = Object.values(processImages).find((candidate) => argv0.endsWith(candidate));
    if (!image) continue;
    writeFileSync(cmdlineFile, `${join(runtimeParentDir, "current", image)}\0`);
  }
}

// stub 的 pnpm 记录调用顺序，并在"重启"时把假 /proc 的可执行文件指向当前
// current 指针，模拟 process-compose 真的换了进程映像。`process list` 是只读探测，
// 不计入调用记录。
function writePnpmStub(path) {
  writeExecutable(
    path,
    `#!/usr/bin/env bash
set -euo pipefail
relink() {
  local target index pid image argv0
  [ "\${STUB_NO_RELINK:-0}" = 1 ] && return 0
  target=$(readlink -f "\${STUB_RUNTIME_PARENT:?}/current")
  index=0
  for image in bin/knoted-api bin/knoted-worker share/knoted/intelligence/python/bin/python3.12; do
    pid=$((1000 + index))
    [ -x "$target/$image" ] || continue
    mkdir -p "\${STUB_PROC:?}/$pid"
    # STUB_EXE=0 模拟跨组：/proc/<pid>/exe 读不到，只剩 cmdline。
    if [ "\${STUB_EXE:-1}" = 1 ]; then
      rm -f "\${STUB_PROC:?}/$pid/exe"
      ln -s "$target/$image" "\${STUB_PROC:?}/$pid/exe"
    fi
    # STUB_CMD_ALIAS 模拟未解析运行根的 service.sh：cmdline 使用别名。
    if [ -n "\${STUB_CMD_ALIAS:-}" ]; then
      argv0="\${STUB_RUNTIME_PARENT:?}/$STUB_CMD_ALIAS/$image"
    else
      argv0="$target/$image"
    fi
    printf '%s\\0' "$argv0" >"\${STUB_PROC:?}/$pid/cmdline"
    index=$((index + 1))
  done
}
case "$*" in
"pc process list") [ "\${STUB_INSTANCE_STOPPED:-0}" = 0 ]; exit ;;
"pc process get knoted-intelligence") [ "\${STUB_LEGACY_GRAPH:-0}" = 1 ]; exit ;;
esac
printf '%s|%s\\n' "$PWD" "$*" >>"\${STUB_LOG:?}"
case "$*" in
"pc down")
  [ "\${STUB_DOWN_FAIL:-0}" = 1 ] && exit 1
  exit 0
  ;;
"pc process restart "*)
  [ "\${STUB_RESTART_FAIL:-0}" = 1 ] && exit 1
  relink
  exit 0
  ;;
"pc up -D")
  relink
  exit 0
  ;;
esac
exit 0
`,
  );
}

// stub 的 curl 覆盖 readiness 探针与 HTTPS 验证，两个开关分开注入：
// STUB_READY_FAIL 模拟实例没起来，STUB_HTTPS_FAIL 模拟进程就绪但入口不通。
function writeCurlStub(path) {
  writeExecutable(
    path,
    `#!/usr/bin/env bash
set -euo pipefail
for argument in "$@"; do
  case "$argument" in
  *internal/v1/health*)
    [ "\${STUB_READY_FAIL:-0}" = 1 ] && exit 22
    printf '{"status":"ok"}'
    exit 0
    ;;
  *health/ready*)
    [ "\${STUB_READY_FAIL:-0}" = 1 ] && exit 22
    printf '{"status":"ready"}'
    exit 0
    ;;
  https://*)
    [ "\${STUB_HTTPS_FAIL:-0}" = 1 ] && exit 22
    printf '%s' "\${STUB_HTTPS_STATUS:-200}"
    exit 0
    ;;
  esac
done
exit 0
`,
  );
}

function writeBuildStub(path) {
  writeExecutable(
    path,
    `#!/usr/bin/env bash
set -euo pipefail
printf '%s|build %s %s %s\\n' "$PWD" "$1" "$2" "\${KNOTED_OFFICE_ORIGIN:-}" >>"\${STUB_LOG:?}"
[ "\${STUB_BUILD_FAIL:-0}" = 1 ] && exit 1
cp -r "\${STUB_ARTIFACT_TEMPLATE:?}" "$2"
`,
  );
}

const officeOrigin = "https://knoted.example.test:15402";

function fixture({
  targetMigrations,
  previousMigrations = ["0001_registry.sql"],
  withTargetArtifact = true,
  legacy = false,
  runtimeParent,
} = {}) {
  const base = makeTempDir("run");
  const runtimeParentDir = runtimeParent ?? join(base, "runtime");
  mkdirSync(runtimeParentDir, { recursive: true });
  const instanceRoot = join(base, "service");
  mkdirSync(instanceRoot, { recursive: true });
  const procRoot = join(base, "proc");
  mkdirSync(procRoot, { recursive: true });
  const stubBin = join(base, "stubs");
  mkdirSync(stubBin, { recursive: true });
  const logFile = join(base, "pnpm.log");
  writeFileSync(logFile, "");

  const previousRoot = createArtifact(runtimeParentDir, previousRevision, previousMigrations, { legacy });
  symlinkSync(previousRoot, join(runtimeParentDir, "current"));
  createProcFixture(procRoot, previousRoot);

  const templateRoot = join(base, "artifact-template");
  cpSync(createArtifact(join(base, "template-build"), targetRevision, targetMigrations), templateRoot, {
    recursive: true,
  });
  if (withTargetArtifact) createArtifact(runtimeParentDir, targetRevision, targetMigrations);

  writePnpmStub(join(stubBin, "pnpm"));
  writeCurlStub(join(stubBin, "curl"));
  writeExecutable(join(stubBin, "ss"), `#!/usr/bin/env bash
if [ "\${STUB_CONTROL_BUSY:-0}" = 1 ]; then printf '%s\n' 'LISTEN 0 4096 127.0.0.1:12345 0.0.0.0:*'; fi
`);
  const buildStub = join(base, "build-artifact-stub.sh");
  writeBuildStub(buildStub);

  const env = {
    ...process.env,
    PATH: `${stubBin}:${process.env.PATH}`,
    KNOTED_INSTANCE_ROOT: instanceRoot,
    KNOTED_DEPLOY_RUNTIME_PARENT: runtimeParentDir,
    KNOTED_DEPLOY_PROC_ROOT: procRoot,
    KNOTED_DEPLOY_BUILD_CMD: buildStub,
    KNOTED_DEPLOY_READY_TIMEOUT: "3",
    KNOTED_API_PORT: "15602",
    KNOTED_HTTPS_PORT: "15601",
    KNOTED_HTTPS_DOMAIN: "knoted.example.test",
    STUB_LOG: logFile,
    STUB_RUNTIME_PARENT: runtimeParentDir,
    STUB_PROC: procRoot,
    STUB_ARTIFACT_TEMPLATE: templateRoot,
    KNOTED_BUILD_ROOT: join(base, "build"),
    // 运行变量必须由 deploy.env 或活动进程提供，不能从测试宿主环境串味。
    KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR: legacy ? "/retired-docling-root" : "",
    KNOTED_INTELLIGENCE_BGE_M3_DIR: legacy ? "/retired-bge-root" : "",
    KNOTED_SEARCH_ENABLED: "",
    KNOTED_SEARCH_CREDENTIAL_FILE: "",
    KNOTED_OFFICE_ORIGIN: "",
    KNOTED_OFFICE_JWT_SECRET_FILE: "",
  };
  return { base, instanceRoot, runtimeParentDir, procRoot, logFile, env, templateRoot };
}

function deploy(context, args = [targetRevision]) {
  return run("bash", [context.script ?? deployScript, ...args], { cwd: repoRoot, env: context.env });
}

// 让部署脚本通过实际 Git common-dir 找到隔离的主检出，既可执行真实 launcher，
// 也能模拟任务 worktree 已更新、主检出的 launcher 仍旧的情况。
function attachLauncher(context, { legacy = false } = {}) {
  const checkout = join(context.base, "main-checkout");
  const linked = join(context.base, "linked-checkout");
  const clone = run("git", ["clone", "--quiet", "--shared", "--no-checkout", repoRoot, checkout]);
  assert.equal(clone.status, 0, clone.stderr);
  const worktree = run("git", ["-C", checkout, "worktree", "add", "--quiet", "--detach", linked, targetRevision]);
  assert.equal(worktree.status, 0, worktree.stderr);
  for (const directory of [checkout, linked]) {
    cpSync(join(repoRoot, "process-compose.yaml"), join(directory, "process-compose.yaml"));
  }
  const launcher = join(checkout, "tools/dev/pc/service.sh");
  mkdirSync(resolve(launcher, ".."), { recursive: true });
  if (legacy) {
    writeExecutable(launcher, '#!/usr/bin/env bash\nprintf "runtime_root=%s\\n" "$KNOTED_RUNTIME_ROOT"\n');
  } else {
    cpSync(join(repoRoot, "tools/dev/pc/service.sh"), launcher);
  }
  context.script = join(linked, "tools/dev/deploy-dev.sh");
  cpSync(deployScript, context.script);
  cpSync(join(repoRoot, "tools/dev/pc/service.sh"), join(linked, "tools/dev/pc/service.sh"));
  return checkout;
}

function pcCalls(logFile) {
  return readFileSync(logFile, "utf8").trim().split("\n").filter(Boolean);
}

// stub 逐行记录 “<cwd>|<argv>”，调用序列断言只看 argv。
function pcArgs(logFile) {
  return pcCalls(logFile).map((line) => line.split("|").slice(1).join("|"));
}

function pcCwd(logFile) {
  return [...new Set(pcCalls(logFile).map((line) => line.split("|")[0]))];
}

function mainCheckout() {
  const commonDir = run("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], {
    cwd: repoRoot,
    encoding: "utf8",
  }).stdout.trim();
  return commonDir.replace(/\/\.git$/, "");
}

function currentTarget(context) {
  return run("readlink", ["-f", join(context.runtimeParentDir, "current")]).stdout.trim();
}

function writeDeployEnv(instanceRoot, values) {
  const body = Object.entries(values)
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
  writeFileSync(join(instanceRoot, "deploy.env"), `${body}\n`, { mode: 0o600 });
}

const runtimeVariables = {
  KNOTED_SEARCH_ENABLED: "1",
  KNOTED_SEARCH_CREDENTIAL_FILE: "/var/lib/search/credentials",
  KNOTED_OFFICE_ORIGIN: officeOrigin,
  KNOTED_OFFICE_JWT_SECRET_FILE: "/var/lib/onlyoffice-secrets/jwt",
};

describe("deploy-dev.sh", () => {
  it("新迁移触发 down + up -D 并切换指针", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql", "0002_notes.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
    assert.deepEqual(readdirSync(context.runtimeParentDir).sort(), [targetRevision, "current"].sort());
  });

  it("迁移集合不变只重启 API 和 Worker", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), [
      "pc process restart knoted-api",
      "pc process restart knoted-worker",
    ]);
    // pcwrap daemon 的 cwd 决定后续 prepare/restart 用哪份 service.sh，必须锚在主检出。
    assert.deepEqual(pcCwd(context.logFile), [mainCheckout()]);
  });

  it("停止失败时不切指针也不启动", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql", "0002_notes.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    context.env.STUB_DOWN_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /停止实例失败/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc down"]);
  });

  it("已是最新提交时清理历史产物，无需构建或重启", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    const old = join(context.runtimeParentDir, previousRevision);
    const legacyRevision = "a".repeat(40);
    const legacy = createArtifact(context.runtimeParentDir, legacyRevision);
    renameSync(legacy, join(context.runtimeParentDir, legacyRevision.slice(0, 12)));
    const stagedRevision = "b".repeat(40);
    const staged = createArtifact(context.runtimeParentDir, stagedRevision);
    renameSync(staged, join(context.runtimeParentDir, `.staging-${stagedRevision.slice(0, 12)}`));
    const unrecognized = join(context.runtimeParentDir, "c".repeat(40));
    mkdirSync(join(unrecognized, "share/knoted"), { recursive: true });
    writeFileSync(join(unrecognized, "share/knoted/REVISION"), `${"c".repeat(40)}\n`);
    const unrecognizedManifest = run("sha256sum", ["share/knoted/REVISION"], { cwd: unrecognized });
    assert.equal(unrecognizedManifest.status, 0, unrecognizedManifest.stderr);
    writeFileSync(join(unrecognized, "SHA256SUMS"), unrecognizedManifest.stdout);
    writeFileSync(join(unrecognized, "keep-me"), "unrelated");
    const alias = join(context.runtimeParentDir, "d".repeat(40));
    symlinkSync(context.templateRoot, alias);
    chmodSync(join(old, "share/knoted"), 0o555);
    rmSync(join(context.runtimeParentDir, "current"));
    symlinkSync(join(context.runtimeParentDir, targetRevision), join(context.runtimeParentDir, "current"));
    rmSync(context.procRoot, { recursive: true, force: true });
    mkdirSync(context.procRoot, { recursive: true });
    createProcFixture(context.procRoot, join(context.runtimeParentDir, targetRevision));

    const result = deploy(context);
    if (existsSync(old)) chmodSync(join(old, "share/knoted"), 0o755);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(pcArgs(context.logFile), []);
    assert.match(result.stderr, /已是最新/);
    assert.deepEqual(readdirSync(context.runtimeParentDir).sort(), [
      targetRevision, "current", "c".repeat(40), "d".repeat(40),
    ].sort());
    assert.ok(existsSync(join(context.templateRoot, "SHA256SUMS")));
  });

  it("--prune-current 只核验并清理正在运行的版本", () => {
    const context = fixture();
    const result = deploy(context, ["--prune-current"]);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(readdirSync(context.runtimeParentDir).sort(), [previousRevision, "current"].sort());
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("当前产物的清单不完整时保留旧产物", () => {
    const context = fixture();
    const current = join(context.runtimeParentDir, previousRevision);
    const manifest = join(current, "SHA256SUMS");
    writeFileSync(manifest, readFileSync(manifest, "utf8").replace(/^.*  bin\/knoted-worker\n/m, ""));
    appendFileSync(join(current, "bin/knoted-worker"), "# changed unlisted file\n");

    const result = deploy(context, ["--prune-current"]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /SHA256SUMS/);
    assert.ok(existsSync(join(context.runtimeParentDir, targetRevision)));
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("共享同一运行根的部署与清理互斥", () => {
    const context = fixture();
    const result = run("flock", ["--close", context.runtimeParentDir, "bash", deployScript, "--prune-current"], {
      cwd: repoRoot,
      env: context.env,
    });
    assert.equal(result.status, 2, result.stderr);
    assert.match(result.stderr, /另一个部署正在使用运行根/);
    assert.ok(existsSync(join(context.runtimeParentDir, targetRevision)));
  });

  it("运行根下有挂载点时保留产物", () => {
    const context = fixture();
    const bin = join(context.base, "stubs");
    const mount = join(context.runtimeParentDir, targetRevision, "mounted");
    writeExecutable(join(bin, "findmnt"), `#!/usr/bin/env bash
printf '%s\\n' "$STUB_MOUNT"
`);
    context.env.STUB_MOUNT = mount;
    const result = deploy(context, ["--prune-current"]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /运行根下存在挂载点/);
    assert.ok(existsSync(join(context.runtimeParentDir, targetRevision)));
  });

  it("已是当前版本但 HTTPS 不可用时失败并保留旧产物", () => {
    const context = fixture();
    const target = join(context.runtimeParentDir, targetRevision);
    rmSync(join(context.runtimeParentDir, "current"));
    symlinkSync(target, join(context.runtimeParentDir, "current"));
    removeProcImages(context.procRoot);
    createProcFixture(context.procRoot, target);
    context.env.STUB_HTTPS_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /HTTPS 入口不可用/);
    assert.ok(existsSync(join(context.runtimeParentDir, previousRevision)));
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("HTTPS 重定向不会触发产物清理", () => {
    const context = fixture();
    context.env.STUB_HTTPS_STATUS = "302";
    const result = deploy(context, ["--prune-current"]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /HTTPS 入口不可用.*302/);
    assert.ok(existsSync(join(context.runtimeParentDir, targetRevision)));
  });

  it("构建失败时指针不动", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"], withTargetArtifact: false });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    context.env.STUB_BUILD_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(
      pcArgs(context.logFile).filter((line) => line.startsWith("pc ")),
      [],
    );
  });

  it("构建时把固定提交、产物目录与 Office origin 传下去", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"], withTargetArtifact: false });
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    const buildCall = pcArgs(context.logFile).find((line) => line.startsWith("build "));
    assert.equal(buildCall, `build ${targetRevision} ${join(context.runtimeParentDir, targetRevision)} ${officeOrigin}`);
  });

  it("HTTPS 验证失败时把指针复位并重启回旧版本", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    // 只让 HTTPS 失败：readiness 与进程映像都正常，真正走到 check_https。
    context.env.STUB_HTTPS_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /HTTPS 入口不可用/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.ok(existsSync(join(context.runtimeParentDir, previousRevision)));
    assert.ok(existsSync(join(context.runtimeParentDir, targetRevision)));
    // 切到新产物及复位后各重启一次 API 和 Worker。
    assert.deepEqual(pcArgs(context.logFile), [
      "pc process restart knoted-api",
      "pc process restart knoted-worker",
      "pc process restart knoted-api",
      "pc process restart knoted-worker",
    ]);
  });

  it("进程仍在旧目录时收敛等待会等满超时并复位", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    // 重启“成功”，argv 已指向目标，但可读的 exe 仍表明映像没换。
    context.env.STUB_EXE = "0";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /仍运行在/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });

  it("exe 不可读时用 cmdline 判定进程映像并部署成功", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    // 跨组：/proc/<pid>/exe 一律 EPERM，只剩世界可读的 cmdline。
    removeProcImages(context.procRoot);
    context.env.STUB_EXE = "0";

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.match(result.stderr, /cmdline/);
    assert.doesNotMatch(result.stderr, /没有找到运行中的 knoted-api/);
  });

  it("从历史短 SHA 目录升级时由主检出的 launcher 固定新产物路径", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    const shortRoot = join(context.runtimeParentDir, "4a546b2f0efd");
    renameSync(currentTarget(context), shortRoot);
    const current = join(context.runtimeParentDir, "current");
    rmSync(current);
    symlinkSync(shortRoot, current);
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    // 旧进程由未解析运行根的 service.sh 启动：exe 不可读，cmdline 只有 current。
    // 重启后会由本次的 service.sh 重新拉起，所以这不应该阻止部署。
    removeProcImages(context.procRoot);
    unresolveProcCmdlines(context.procRoot, context.runtimeParentDir);
    setProcGid(context.procRoot, 980);
    const checkout = attachLauncher(context);

    const result = deploy(context);

    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcCwd(context.logFile), [checkout]);
    assert.match(result.stderr, /主检出的 launcher 能为新进程固定产物路径/);
  });

  it("主检出仍用旧 launcher 时不切换产物也不重启", () => {
    const context = fixture();
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    removeProcImages(context.procRoot);
    unresolveProcCmdlines(context.procRoot, context.runtimeParentDir);
    setProcGid(context.procRoot, 980);
    attachLauncher(context, { legacy: true });

    const result = deploy(context);
    assert.equal(result.status, 2, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(pcArgs(context.logFile), []);
    assert.match(result.stderr, /先更新主检出.*sg .*未切换指针/);
  });

  it("有可读但不匹配的 exe 时不使用指向目标的 argv", () => {
    const context = fixture();
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    removeProcImages(context.procRoot);
    symlinkSync("/usr/bin/bash", join(context.procRoot, "1000", "exe"));
    context.env.STUB_EXE = "0";

    const result = deploy(context);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /没有找到运行中的 knoted-api/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });

  it("仅有 cmdline 但指向旧 SHA 时仍复位", () => {
    const context = fixture();
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    removeProcImages(context.procRoot);
    context.env.STUB_NO_RELINK = "1";

    const result = deploy(context);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /仍运行在/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });

  it("切换后无法判定进程映像时立即失败且不回滚", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    removeProcImages(context.procRoot);
    // 切换前 cmdline 有具体产物目录；重启后仅引用自定义别名，不能当成旧 SHA。
    context.env.STUB_EXE = "0";
    context.env.STUB_CMD_ALIAS = "configured-alias";
    symlinkSync(join(context.runtimeParentDir, "current"), join(context.runtimeParentDir, "configured-alias"));
    context.env.KNOTED_DEPLOY_READY_TIMEOUT = "60";

    const started = Date.now();
    const result = deploy(context);
    const elapsed = Date.now() - started;

    assert.notEqual(result.status, 0);
    // 判不出来时回滚只会扩大空窗，所以指针留在新产物，也不重复重启。
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), [
      "pc process restart knoted-api",
      "pc process restart knoted-worker",
    ]);
    assert.ok(elapsed < 20_000, `必须快速失败，实际 ${elapsed}ms`);
    assert.match(result.stderr, /不回滚/);
  });

  it("带迁移的部署 readiness 超时时保留新产物而不是回退指针", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql", "0002_notes.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    context.env.STUB_READY_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
  });

  it("摘要不符的产物在重建后清理", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const target = join(context.runtimeParentDir, targetRevision);
    appendFileSync(join(target, "bin/knoted-api"), "# corrupt\n");

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    const quarantined = readdirSync(context.runtimeParentDir).filter((name) => name.includes(".incomplete."));
    assert.deepEqual(quarantined, []);
    assert.equal(currentTarget(context), target);
  });

  it("调用者环境变量覆盖 deploy.env 并回写文件", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"], withTargetArtifact: false });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const shellOrigin = "https://from-shell.example.test:15402";
    context.env.KNOTED_OFFICE_ORIGIN = shellOrigin;

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    const buildCall = pcArgs(context.logFile).find((line) => line.startsWith("build "));
    assert.equal(buildCall, `build ${targetRevision} ${join(context.runtimeParentDir, targetRevision)} ${shellOrigin}`);
    assert.match(readFileSync(join(context.instanceRoot, "deploy.env"), "utf8"), new RegExp(`^KNOTED_OFFICE_ORIGIN=${shellOrigin}$`, "m"));
  });

  it("显式给出全部现用配置仍可读取旧 deploy.env 并完成部署", () => {
    const context = fixture();
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    Object.assign(context.env, runtimeVariables);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
  });

  it("实例停着时 restart 失败回退到完整启动", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    context.env.STUB_RESTART_FAIL = "1";

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), [
      "pc process restart knoted-api",
      "pc down",
      "pc up -D",
    ]);
  });

  it("从已核实的 API 和 Worker 抓取运行变量并写成 0600", () => {
    const context = fixture();
    const { KNOTED_SEARCH_ENABLED, KNOTED_SEARCH_CREDENTIAL_FILE, ...office } = runtimeVariables;
    for (const [pid, values] of [
      ["1000", office],
      ["1001", { KNOTED_SEARCH_ENABLED, KNOTED_SEARCH_CREDENTIAL_FILE }],
    ]) {
      const input = Object.entries(values).map(([key, value]) => `${key}=${value}\0`).join("");
      writeFileSync(join(context.procRoot, pid, "environ"), input);
    }

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    const deployEnv = join(context.instanceRoot, "deploy.env");
    assert.equal(statSync(deployEnv).mode & 0o777, 0o600);
    for (const [key, value] of Object.entries(runtimeVariables)) {
      assert.match(readFileSync(deployEnv, "utf8"), new RegExp(`^${key}=${value}$`, "m"), key);
    }
  });

  it("未经 exe 核实的进程不提供可选运行变量", () => {
    const context = fixture();
    removeProcImages(context.procRoot);
    const input = Object.entries(runtimeVariables).map(([key, value]) => `${key}=${value}\0`).join("");
    writeFileSync(join(context.procRoot, "1000", "environ"), input);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /environ 不可读或无法确认来源/);
    assert.equal(existsSync(join(context.instanceRoot, "deploy.env")), false);
  });

  it("实际打不开 environ 时报告原因并采用显式配置", async () => {
    const context = fixture();
    context.env.KNOTED_SEARCH_ENABLED = "1";
    // 用 Unix socket 模拟 stat/-r 通过、实际打开失败。
    const environ = join(context.procRoot, "1000", "environ");
    const server = createServer();
    await new Promise((resolve, reject) => server.once("error", reject).listen(environ, resolve));
    let result;
    try {
      chmodSync(environ, 0o600);
      assert.equal(run("test", ["-r", environ]).status, 0);
      result = deploy(context);
    } finally {
      await new Promise((resolve) => server.close(resolve));
    }
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /environ 不可读/);
    assert.match(result.stderr, /sg /);
    assert.equal(readFileSync(join(context.instanceRoot, "deploy.env"), "utf8"), "KNOTED_SEARCH_ENABLED=1\n");
  });

  it("旧进程图仍可连接时拒绝切换", () => {
    const context = fixture({ legacy: true });
    for (const pid of ["1000", "1001", "1002"]) rmSync(join(context.procRoot, pid), { recursive: true });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const result = deploy(context);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /旧.*编排.*停止/);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("旧 daemon 不可达时拒绝遗留计算进程", () => {
    const context = fixture({ legacy: true });
    for (const pid of ["1000", "1001"]) rmSync(join(context.procRoot, pid), { recursive: true });
    context.env.STUB_INSTANCE_STOPPED = "1";
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const result = deploy(context);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /knoted-intelligence.*1002/);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("旧控制端口仍占用时拒绝把连接失败当停机", () => {
    const context = fixture({ legacy: true });
    for (const pid of ["1000", "1001", "1002"]) rmSync(join(context.procRoot, pid), { recursive: true });
    context.env.STUB_INSTANCE_STOPPED = "1";
    context.env.STUB_CONTROL_BUSY = "1";
    context.env.PC_PORT_NUM = "12345";
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /控制端口 12345.*停止/);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("已停止旧编排时完整加载新进程图并只保留现用变量", () => {
    const context = fixture({ legacy: true });
    for (const pid of ["1000", "1001", "1002"]) rmSync(join(context.procRoot, pid), { recursive: true });
    context.env.STUB_INSTANCE_STOPPED = "1";
    context.env.KNOTED_OFFICE_ORIGIN = officeOrigin;
    writeDeployEnv(context.instanceRoot, {
      ...runtimeVariables,
      KNOTED_INTELLIGENCE_BGE_M3_DIR: "/retired-model-root",
      KNOTED_CHAT_LOCAL_MODEL: "retired-model",
      KNOTED_VECTORS_ENABLED: "1",
    });
    attachLauncher(context);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc up -D"]);
    const renderedEnv = readFileSync(join(context.instanceRoot, "deploy.env"), "utf8");
    assert.equal(renderedEnv, Object.entries(runtimeVariables).map(([key, value]) => `${key}=${value}\n`).join(""));
  });

  it("跨进程图失败时保留两份固定产物供排查", () => {
    const context = fixture({ legacy: true });
    for (const pid of ["1000", "1001", "1002"]) rmSync(join(context.procRoot, pid), { recursive: true });
    context.env.STUB_INSTANCE_STOPPED = "1";
    context.env.STUB_HTTPS_FAIL = "1";
    attachLauncher(context);
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc up -D"]);
    assert.ok(existsSync(join(context.runtimeParentDir, previousRevision)));
  });

  it("旧手工启动没有 deploy.env 时从已渲染配置保留搜索和 Office", () => {
    const context = fixture({ legacy: true, withTargetArtifact: false });
    for (const pid of ["1000", "1001", "1002"]) rmSync(join(context.procRoot, pid), { recursive: true });
    context.env.STUB_INSTANCE_STOPPED = "1";
    attachLauncher(context);
    const configDir = join(context.instanceRoot, "config");
    mkdirSync(configDir);
    const credentialPath = join(context.base, "not created search file");
    const jwtPath = join(context.base, "not created Office file");
    writeFileSync(join(configDir, "knoted.toml"), `[server]
bind = "127.0.0.1:15602"
[database]
url = "postgres://fixture@localhost/do-not-copy"
[files]
enabled = true
[intelligence]
base_url = "http://127.0.0.1:15603"
[office]
document_server_origin = "${officeOrigin}"
jwt_secret_file = "${jwtPath}"
[credentials]
credential_file = "${credentialPath}"
[search]
enabled = true
api_key_credential = "meilisearch"
`);

    const result = deploy(context);

    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.ok(pcArgs(context.logFile).some(line => line.startsWith("build ") && line.endsWith(` ${officeOrigin}`)));
    assert.equal(readFileSync(join(context.instanceRoot, "deploy.env"), "utf8"), [
      "KNOTED_SEARCH_ENABLED=1",
      `KNOTED_SEARCH_CREDENTIAL_FILE=${credentialPath}`,
      `KNOTED_OFFICE_ORIGIN=${officeOrigin}`,
      `KNOTED_OFFICE_JWT_SECRET_FILE=${jwtPath}`,
      "",
    ].join("\n"));
    assert.equal(existsSync(credentialPath), false);
    assert.equal(existsSync(jwtPath), false);
    assert.doesNotMatch(result.stdout + result.stderr, /do-not-copy/);
  });

  it("没有旧部署环境且渲染配置不可用时拒绝清空保留功能配置", () => {
    const context = fixture({ legacy: true });
    for (const pid of ["1000", "1001", "1002"]) rmSync(join(context.procRoot, pid), { recursive: true });
    context.env.STUB_INSTANCE_STOPPED = "1";

    const result = deploy(context);

    assert.notEqual(result.status, 0);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.match(result.stderr, /旧.*配置.*显式/);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("旧渲染配置使用不受支持的搜索字段时拒绝改写", () => {
    const context = fixture({ legacy: true });
    for (const pid of ["1000", "1001", "1002"]) rmSync(join(context.procRoot, pid), { recursive: true });
    context.env.STUB_INSTANCE_STOPPED = "1";
    const configDir = join(context.instanceRoot, "config");
    mkdirSync(configDir);
    writeFileSync(join(configDir, "knoted.toml"), '[server]\n[database]\n[files]\n[search]\nenabled = true\nindex_uid = "custom"\n');

    const result = deploy(context);

    assert.notEqual(result.status, 0);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.match(result.stderr, /旧渲染配置.*显式/);
    assert.equal(existsSync(join(context.instanceRoot, "deploy.env")), false);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("当前指针已经更新但旧项目仍在时拒绝重启和清理", () => {
    const context = fixture();
    context.env.STUB_LEGACY_GRAPH = "1";
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const result = deploy(context);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /旧.*编排.*停止/);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("清理前再次识别旧解释器后代", () => {
    const context = fixture();
    const legacy = createArtifact(context.runtimeParentDir, "a".repeat(40), undefined, { legacy: true });
    const pid = join(context.procRoot, "1002");
    mkdirSync(pid);
    // 跨组的 guardian 只有固定解释器 argv[0] 可读。
    writeFileSync(join(pid, "cmdline"), `${join(legacy, processImages["knoted-intelligence"])}\0-m\0knoted_intelligence.guardian\0`);
    const result = deploy(context, ["--prune-current"]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /knoted-intelligence.*1002/);
    assert.ok(existsSync(legacy));
  });

  it("候选产物缺少 Web 时，即使清单完整也不重启或切换", () => {
    const context = fixture();
    const target = join(context.runtimeParentDir, targetRevision);
    for (const root of [target, context.templateRoot]) {
      rmSync(join(root, "share/knoted/web/index.html"));
      writeManifest(root);
    }
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const result = deploy(context);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /share\/knoted\/web\/index\.html/);
    assert.ok(pcArgs(context.logFile).every((line) => !line.startsWith("pc ")));
  });

  it("拒绝非法 REVISION 并且错误信息不回显标记内容", () => {
    const context = fixture();
    const target = join(context.runtimeParentDir, targetRevision);
    const invalidMarker = "untrusted-release-marker";
    for (const root of [target, context.templateRoot]) {
      writeFileSync(join(root, "share/knoted/REVISION"), `${invalidMarker}\n`);
      writeManifest(root);
    }
    const result = deploy(context);

    assert.notEqual(result.status, 0);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.match(result.stderr, /REVISION/);
    assert.doesNotMatch(result.stderr, /untrusted-release-marker/);
  });

  it("拒绝在 .worktrees 里部署稳定实例", () => {
    const sandbox = join(makeTempDir("guard"), ".worktrees", "task");
    mkdirSync(sandbox, { recursive: true });
    const context = fixture({ targetMigrations: ["0001_registry.sql"], runtimeParent: join(sandbox, "runtime") });

    const result = deploy(context);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /worktrees/);
  });

  it("拒绝非完整 SHA 参数", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    const result = deploy(context, ["8b0457e"]);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /40/);
  });

  it("拒绝旧源码提交并保留已有的同 SHA 归档产物", () => {
    const context = fixture();
    const archived = createArtifact(context.runtimeParentDir, legacyRevision, undefined, { legacy: true });
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context, [legacyRevision]);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /目标提交.*产物代/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.ok(existsSync(join(archived, "bin/knoted-intelligence")));
    assert.equal(readdirSync(context.runtimeParentDir).some(path => path.includes(".incomplete.")), false);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("独立构建入口在触碰构建根前拒绝旧源码提交", () => {
    const context = fixture();
    const output = join(context.base, "archived-build");
    // The old builder reaches cargo. A stub makes the regression safe.
    writeExecutable(join(context.base, "stubs/cargo"), "#!/usr/bin/env bash\nexit 91\n");
    const result = run("bash", [join(repoRoot, "tools/dev/build-artifact.sh"), legacyRevision, output], {
      cwd: repoRoot,
      env: context.env,
    });

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /目标提交.*产物代/);
    assert.equal(existsSync(context.env.KNOTED_BUILD_ROOT), false);
    assert.equal(existsSync(output), false);
  });

  it("拒绝不存在的提交", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    const result = deploy(context, ["f".repeat(40)]);
    assert.equal(result.status, 2);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });
});
