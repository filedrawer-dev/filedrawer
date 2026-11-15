#!/usr/bin/env node
// Packs the npm packages of the `filedrawer` command (design.md, Distribution; #84):
//
//   node crates/cli/npm/pack.mjs platform RUST_TARGET BINARY OUTDIR
//   node crates/cli/npm/pack.mjs launcher OUTDIR
//   node crates/cli/npm/pack.mjs placeholders OUTDIR
//
// `platform` packs @filedrawer/cli-<os>-<cpu> around one release binary, and
// `launcher` packs @filedrawer/cli, whose optionalDependencies are those packages,
// so npm installs only the one for its machine; filedrawer.js runs it. Each package
// takes its version from crates/cli/Cargo.toml and carries the crate's LICENSE. Each
// command prints the path of the .tgz it wrote. release-cli.yml runs both.
// `placeholders` writes a 0.0.0 of each package, a folder each, which
// ops/claim-names publishes before the first release.

import { execSync } from "node:child_process";
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

// Writes a package's files into FOLDER: its package.json, the crate's LICENSE, and
// each of FILES, copied `from` a path or written from its `text`.
function writePackage(folder, manifest, files) {
  mkdirSync(folder, { recursive: true });
  writeFileSync(join(folder, "package.json"), JSON.stringify(manifest, null, 2) + "\n");
  copyFileSync(join(CRATE, "LICENSE"), join(folder, "LICENSE"));
  for (const [to, { from, text, mode }] of Object.entries(files)) {
    mkdirSync(dirname(join(folder, to)), { recursive: true });
    if (text === undefined) copyFileSync(from, join(folder, to));
    else writeFileSync(join(folder, to), text);
    if (mode) chmodSync(join(folder, to), mode);
  }
}

// Writes the package in a temp folder, packs it into OUTDIR, and prints the .tgz.
function packFolder(manifest, files, outdir) {
  const folder = mkdtempSync(join(tmpdir(), "filedrawer-npm-"));
  try {
    writePackage(folder, manifest, files);
    // Through a shell, which finds npm.cmd on Windows, with each argument quoted
    // for sh and cmd.exe alike: a path holds no double quote.
    const args = ["npm", "pack", "--json", "--pack-destination", resolve(outdir), folder];
    const packed = execSync(args.map((arg) => `"${arg}"`).join(" "), { encoding: "utf8" });
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
  const name = platformName(platform);
  const manifest = {
    ...common(name, `The filedrawer command for ${platform.os} ${platform.cpu}; install @filedrawer/cli.`),
    os: [platform.os],
    cpu: [platform.cpu],
    files: ["bin"],
  };
  const readme =
    `# ${name}\n\n` +
    `The \`filedrawer\` binary for ${platform.os} ${platform.cpu}, which [@filedrawer/cli](https://www.npmjs.com/package/@filedrawer/cli) installs and runs.\n` +
    "Install that package rather than this one.\n";
  packFolder(manifest, { [`bin/${exe}`]: { from: binary, mode: 0o755 }, "README.md": { text: readme } }, outdir);
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

// What the launcher's placeholder runs: there is no binary until the release.
const PLACEHOLDER_BIN = `#!/usr/bin/env node
console.error([
  "filedrawer: this is a placeholder that holds the name on npm until the first release.",
  "Meanwhile, install it from PyPI or crates.io:",
  "  uvx filedrawer check ENTRY.md",
  "  cargo install filedrawer --locked",
].join("\\n"));
process.exit(1);
`;

// Writes a 0.0.0 of each package into OUTDIR, one folder each, and prints them in
// the order to publish them, the launcher last. npm's trusted publishing can be
// set up only for a package npm has, so ops/claim-names publishes these first.
function writePlaceholders(outdir) {
  const names = [...Object.values(PLATFORMS).map(platformName), "@filedrawer/cli"];
  for (const name of names) {
    const folder = join(resolve(outdir), name.replace("@filedrawer/", ""));
    const launcher = name === "@filedrawer/cli";
    const manifest = {
      ...common(name, "Holds the name until the first release; install @filedrawer/cli."),
      version: "0.0.0",
      ...(launcher && { bin: { filedrawer: "bin/filedrawer.js" } }),
    };
    const readme =
      `# ${name}\n\n` +
      "A placeholder that holds the name on npm until the first release of [@filedrawer/cli](https://www.npmjs.com/package/@filedrawer/cli), File Drawer's offline entry checker.\n";
    writePackage(folder, manifest, {
      "README.md": { text: readme },
      ...(launcher && { "bin/filedrawer.js": { text: PLACEHOLDER_BIN, mode: 0o755 } }),
    });
    console.log(folder);
  }
}

const [command, ...args] = process.argv.slice(2);
if (command === "platform" && args.length === 3) {
  packPlatform(...args);
} else if (command === "launcher" && args.length === 1) {
  packLauncher(...args);
} else if (command === "placeholders" && args.length === 1) {
  writePlaceholders(...args);
} else {
  console.error(
    "usage: pack.mjs platform RUST_TARGET BINARY OUTDIR | pack.mjs launcher OUTDIR | pack.mjs placeholders OUTDIR",
  );
  process.exit(2);
}
