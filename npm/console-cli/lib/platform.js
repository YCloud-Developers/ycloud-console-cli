"use strict";

const PLATFORM_PACKAGES = {
  "darwin-arm64": "@ycloud-ai/console-cli-darwin-arm64",
  "darwin-x64": "@ycloud-ai/console-cli-darwin-x64",
  "linux-arm64": "@ycloud-ai/console-cli-linux-arm64",
  "linux-x64": "@ycloud-ai/console-cli-linux-x64",
};

function platformPackage(platform, arch) {
  const packageName = PLATFORM_PACKAGES[`${platform}-${arch}`];
  if (!packageName) {
    throw new Error(`unsupported platform: ${platform}/${arch}`);
  }
  return packageName;
}

function executableName(platform) {
  return platform === "win32" ? "ycloud.exe" : "ycloud";
}

module.exports = { executableName, platformPackage };
