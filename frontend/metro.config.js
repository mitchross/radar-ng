const { getDefaultConfig } = require("expo/metro-config");
const path = require("node:path");

const config = getDefaultConfig(__dirname);
// Dependency-free forecast presentation and alert planning shared with web.
config.watchFolders = [path.resolve(__dirname, "../web/src/shared")];

module.exports = config;
