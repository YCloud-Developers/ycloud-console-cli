import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const [artifactsArg, outputArg] = process.argv.slice(2);
if (!artifactsArg || !outputArg) {
  throw new Error(
    "usage: node scripts/generate-release-checksums.mjs <artifacts-dir> <output>",
  );
}

function collectArchives(directory) {
  const archives = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      archives.push(...collectArchives(entryPath));
    } else if (entry.isFile() && entry.name.endsWith(".tar.gz")) {
      archives.push(entryPath);
    }
  }
  return archives;
}

const artifacts = path.resolve(artifactsArg);
const output = path.resolve(outputArg);
const archives = collectArchives(artifacts).sort((left, right) =>
  path.basename(left).localeCompare(path.basename(right)),
);

if (archives.length === 0) {
  throw new Error(`no .tar.gz archives found in ${artifacts}`);
}

const names = new Set();
const checksums = archives.map((archive) => {
  const name = path.basename(archive);
  if (names.has(name)) {
    throw new Error(`duplicate release archive name: ${name}`);
  }
  names.add(name);

  const checksum = crypto
    .createHash("sha256")
    .update(fs.readFileSync(archive))
    .digest("hex");
  return `${checksum}  ${name}`;
});

fs.writeFileSync(output, `${checksums.join("\n")}\n`);
console.log(`Release checksums written to ${output}`);
