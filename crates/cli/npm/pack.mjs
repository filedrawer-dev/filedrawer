#!/usr/bin/env node
// Packs the npm packages of the `filedrawer` command (design.md, Distribution; #84):
//
//   node crates/cli/npm/pack.mjs platform RUST_TARGET BINARY OUTDIR
//   node crates/cli/npm/pack.mjs launcher OUTDIR
//
// `platform` packs @filedrawer/cli-<os>-<cpu> around one release binary, and
// `launcher` packs @filedrawer/cli, whose optionalDependencies are those packages,
// so npm installs only the one for its machine; filedrawer.js runs it. Each package
// takes its version from crates/cli/Cargo.toml and carries the crate's LICENSE. Each
// command prints the path of the .tgz it wrote. release-cli.yml runs both.

import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const CRATE = resolve(HERE, "..");

// The targets release-cli.yml builds, by npm's names for their os and cpu.
// Linux binaries are static (musl), so one package serves glibc and musl systems.
const PLATFORMS = {
  "x86_64-unknown-linux-musl": { os: "linux", cpu: "x64" },
  "aarch64-unknown-linux-musl": { os: "linux", cpu: "arm64" },
  "aarch64-apple-darwin": { os: "darwin", cpu: "arm64" },
  "x86_64-apple-darwin": { os: "darwin", cpu: "x64" },
  "x86_64-pc-windows-msvc": { os: "win32", cpu: "x64" },
};

const platformName = ({ os, cpu }) => `@filedrawer/cli-${os}-${cpu}`;

function version() {
  const cargo = readFileSync(join(CRATE, "Cargo.toml"), "utf8");
  return cargo.match(/^version = "(.*)"$/m)[1];
}

// What every package says of itself. npm's trusted publishing refuses a
// repository.url that isn't exactly the repo's, and a scoped package is private
// unless its first publish says otherwise.
function common(name, description) {
  return {
    name,
    version: version(),
    description,
    license: "AGPL-3.0-only",
    homepage: "https://filedrawer.dev",
    repository: { type: "git", url: "git+https://github.com/filedrawer-dev/filedrawer.git" },
    bugs: "https://github.com/filedrawer-dev/filedrawer/issues",
    publishConfig: { access: "public" },
  };
}

// Writes the package in a temp folder, packs it into OUTDIR, and prints the .tgz.
function packFolder(manifest, files, outdir) {
  const folder = mkdtempSync(join(tmpdir(), "filedrawer-npm-"));
  try {
    writeFileSync(join(folder, "package.json"), JSON.stringify(manifest, null, 2) + "\n");
    copyFileSync(join(CRATE, "LICENSE"), join(folder, "LICENSE"));
    for (const [to, { from, mode }] of Object.entries(files)) {
      mkdirSync(dirname(join(folder, to)), { recursive: true });
      copyFileSync(from, join(folder, to));
      if (mode) chmodSync(join(folder, to), mode);
    }
    const packed = execFileSync("npm", ["pack", "--json", "--pack-destination", resolve(outdir), folder], {
      encoding: "utf8",
      shell: process.platform === "win32",
    });
    console.log(join(resolve(outdir), JSON.parse(packed)[0].filename));
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
}

function packPlatform(target, binary, outdir) {
  const platform = PLATFORMS[target];
  if (!platform) {
    console.error(`pack.mjs: no npm package for ${target}; the targets are ${Object.keys(PLATFORMS).join(", ")}`);
    process.exit(2);
  }
  const exe = basename(binary);
  const manifest = {
    ...common(platformName(platform), `The filedrawer command for ${platform.os} ${platform.cpu}; install @filedrawer/cli.`),
    os: [platform.os],
    cpu: [platform.cpu],
    files: ["bin"],
  };
  packFolder(manifest, { [`bin/${exe}`]: { from: binary, mode: 0o755 } }, outdir);
}

function packLauncher(outdir) {
  const manifest = {
    ...common("@filedrawer/cli", "Checks File Drawer entries offline, with the analyzer filedrawer.dev runs."),
    keywords: ["ai-safety", "null-results", "research", "markdown", "linter"],
    bin: { filedrawer: "bin/filedrawer.js" },
    files: ["bin"],
    engines: { node: ">=18" },
    optionalDependencies: Object.fromEntries(Object.values(PLATFORMS).map((p) => [platformName(p), version()])),
  };
  packFolder(
    manifest,
    {
      "bin/filedrawer.js": { from: join(HERE, "filedrawer.js"), mode: 0o755 },
      "README.md": { from: join(HERE, "README.md") },
    },
    outdir,
  );
}

const [command, ...args] = process.argv.slice(2);
if (command === "platform" && args.length === 3) {
  packPlatform(...args);
} else if (command === "launcher" && args.length === 1) {
  packLauncher(...args);
} else {
  console.error("usage: pack.mjs platform RUST_TARGET BINARY OUTDIR | pack.mjs launcher OUTDIR");
  process.exit(2);
}
