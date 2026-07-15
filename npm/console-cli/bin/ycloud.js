#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const { executableName, platformPackage } = require("../lib/platform");

let executable;
try {
  executable = require.resolve(
    `${platformPackage(process.platform, process.arch)}/bin/${executableName(
      process.platform,
    )}`,
  );
} catch (error) {
  console.error(
    `Unable to find the YCloud Console CLI binary for ${process.platform}/${process.arch}.`,
  );
  console.error("Reinstall without --omit=optional and try again.");
  process.exit(1);
}

const result = spawnSync(executable, process.argv.slice(2), {
  stdio: "inherit",
});

if (result.error) {
  console.error(`Failed to start YCloud Console CLI: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
