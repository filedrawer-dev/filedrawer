#!/usr/bin/env node
// The `filedrawer` command of the npm package @filedrawer/cli: it runs the binary
// of the platform package npm installed beside it, one of its optionalDependencies
// (pack.mjs), with the same arguments, and exits as the binary did.
"use strict";

const { spawnSync } = require("node:child_process");
const { existsSync } = require("node:fs");
const { constants } = require("node:os");
const { basename, join } = require("node:path");

// The platforms are the launcher's optionalDependencies, @filedrawer/cli-<os>-<cpu>.
const PLATFORMS = Object.keys(require("../package.json").optionalDependencies).map((name) =>
  name.replace("@filedrawer/cli-", ""),
);
const platform = `${process.platform}-${process.arch}`;
const exe = process.platform === "win32" ? "filedrawer.exe" : "filedrawer";

// Where the install that put this file here puts the platform package: in this
// package's own node_modules, as in a global install, or beside this package in the
// @filedrawer folder. Not require.resolve, which goes on to the node_modules of
// every folder above and to $NODE_PATH, where a package is someone else's.
const beside = join(__dirname, "..", "..");
const binary = [join(__dirname, "..", "node_modules", "@filedrawer"), beside]
  .filter((folder) => basename(folder) === "@filedrawer")
  .map((folder) => join(folder, `cli-${platform}`, "bin", exe))
  .find((path) => existsSync(path));
if (!binary) {
  console.error(
    [
      `filedrawer: npm installed no binary for ${platform}.`,
      `@filedrawer/cli has binaries for ${PLATFORMS.join(", ")}.`,
      "On one of those, reinstall without --omit=optional or --no-optional.",
      "Elsewhere, install it from PyPI or crates.io:",
      "  uvx filedrawer check ENTRY.md",
      "  cargo install filedrawer --locked",
    ].join("\n"),
  );
  process.exit(1);
}

const run = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (run.error) {
  console.error(`filedrawer: couldn't run ${binary}: ${run.error.message}`);
  process.exit(1);
}
// A binary a signal killed ends this process by the same signal; one node ignores,
// such as SIGPIPE, exits as a shell reports it, 128 plus its number.
if (run.signal) {
  process.kill(process.pid, run.signal);
  process.exit(128 + (constants.signals[run.signal] ?? 0));
}
process.exit(run.status);
