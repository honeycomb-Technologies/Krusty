const { getDefaultConfig } = require('expo/metro-config');
const path = require('path');

const projectRoot = __dirname;
const monorepoRoot = path.resolve(projectRoot, '../..');

const config = getDefaultConfig(projectRoot);

// Watch workspace packages for changes
config.watchFolders = [
  path.resolve(monorepoRoot, 'packages/api'),
  path.resolve(monorepoRoot, 'packages/state'),
  path.resolve(monorepoRoot, 'packages/ui'),
];

// Resolve modules from both project and monorepo root
config.resolver.nodeModulesPaths = [
  path.resolve(projectRoot, 'node_modules'),
  path.resolve(monorepoRoot, 'node_modules'),
];

// Ensure packages resolve to their source
config.resolver.disableHierarchicalLookup = false;

module.exports = config;
