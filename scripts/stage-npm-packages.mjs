import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const artifacts = path.resolve(process.argv[2] ?? "");
if (!process.argv[2]) {
  throw new Error("usage: node scripts/stage-npm-packages.mjs <artifacts-dir>");
}

const packages = {
  "darwin-arm64": "darwin-arm64",
  "darwin-x64": "darwin-x64",
  "linux-arm64": "linux-arm64",
  "linux-x64": "linux-x64",
};

for (const [artifactName, packageName] of Object.entries(packages)) {
  const source = path.join(
    artifacts,
    `release-${artifactName}`,
    "ycloud",
  );
  const destination = path.join(
    root,
    "npm",
    "platforms",
    packageName,
    "bin",
    "ycloud",
  );
  if (!fs.existsSync(source)) {
    throw new Error(`release binary is missing: ${source}`);
  }
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.copyFileSync(source, destination);
  fs.chmodSync(destination, 0o755);
}

console.log("Native npm packages staged.");
