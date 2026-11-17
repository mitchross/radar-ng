const { getDefaultConfig } = require("expo/metro-config");
const path = require("node:path");

const config = getDefaultConfig(__dirname);
// Forecast presentation and alert planning shared with web.
config.watchFolders = [path.resolve(__dirname, "../web/src/shared")];
// Shared files import astronomy-engine; resolve it from the app's install, not web/node_modules.
config.resolver.nodeModulesPaths = [path.resolve(__dirname, "node_modules")];

module.exports = config;
