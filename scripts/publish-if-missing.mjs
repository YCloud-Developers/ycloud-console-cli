import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

function defaultRunNpm(args) {
  return spawnSync(process.env.NPM_CLI ?? "npm", args, {
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
  });
}

export function publishIfMissing(
  packageDirectory,
  { runNpm = defaultRunNpm, log = console.log } = {},
) {
  const directory = path.resolve(packageDirectory);
  const manifestPath = path.join(directory, "package.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));

  if (!manifest.name || !manifest.version) {
    throw new Error(`${manifestPath} must contain name and version`);
  }

  const spec = `${manifest.name}@${manifest.version}`;
  const lookup = runNpm([
    "view",
    spec,
    "version",
    "--json",
    "--prefer-online",
  ]);

  if (lookup.error) {
    throw lookup.error;
  }

  if (lookup.status === 0) {
    const publishedVersion = JSON.parse(lookup.stdout.trim());
    if (publishedVersion !== manifest.version) {
      throw new Error(
        `npm returned unexpected version for ${spec}: ${lookup.stdout.trim()}`,
      );
    }
    log(`${spec} already exists; skipping publish.`);
    return "skipped";
  }

  const lookupOutput = `${lookup.stdout}\n${lookup.stderr}`;
  if (!/\bE404\b|404 Not Found/i.test(lookupOutput)) {
    throw new Error(`failed to check ${spec} on npm:\n${lookupOutput.trim()}`);
  }

  const published = runNpm(["publish", directory, "--access", "public"]);
  if (published.error) {
    throw published.error;
  }
  if (published.status !== 0) {
    throw new Error(
      `failed to publish ${spec}:\n${published.stdout}\n${published.stderr}`,
    );
  }

  if (published.stdout) {
    process.stdout.write(published.stdout);
  }
  if (published.stderr) {
    process.stderr.write(published.stderr);
  }
  return "published";
}

const isCli =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isCli) {
  if (process.argv.length !== 3) {
    throw new Error("usage: node scripts/publish-if-missing.mjs <package-dir>");
  }
  publishIfMissing(process.argv[2]);
}
