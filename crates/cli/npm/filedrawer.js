#!/usr/bin/env node
// The `filedrawer` command of the npm package @filedrawer/cli: it runs the binary
// of the platform package npm installed beside it, one of its optionalDependencies
// (pack.mjs), with the same arguments, and exits as the binary did.
"use strict";

const { spawnSync } = require("node:child_process");

const PLATFORMS = ["linux-x64", "linux-arm64", "darwin-arm64", "darwin-x64", "win32-x64"];
const platform = `${process.platform}-${process.arch}`;
const exe = process.platform === "win32" ? "filedrawer.exe" : "filedrawer";

let binary;
try {
  binary = require.resolve(`@filedrawer/cli-${platform}/bin/${exe}`);
} catch {
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
if (run.signal) {
  process.kill(process.pid, run.signal);
}
process.exit(run.status);
