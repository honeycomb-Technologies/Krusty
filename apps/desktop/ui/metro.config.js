const { getDefaultConfig } = require('expo/metro-config');
const fs = require('fs');
const path = require('path');

const projectRoot = __dirname;
const monorepoRoot = path.resolve(projectRoot, '../../..');
const mobileRoot = path.resolve(monorepoRoot, 'apps/mobile');

function firstExisting(...candidates) {
  for (const candidate of candidates) {
    if (candidate && fs.existsSync(candidate)) return candidate;
  }
  return null;
}

const mobileNodeModules = firstExisting(
  path.resolve(projectRoot, 'node_modules'),
  path.resolve(mobileRoot, 'node_modules'),
  '/Users/Jacob/Documents/Mitsuro/apps/mobile/node_modules',
);

const config = getDefaultConfig(projectRoot);

const watchFolders = [
  path.resolve(monorepoRoot, 'packages/api'),
  path.resolve(monorepoRoot, 'packages/state'),
  path.resolve(monorepoRoot, 'packages/ui'),
  mobileRoot,
];
if (mobileNodeModules) {
  // Prefer realpath so Metro watches the actual directory, not a missing relative path.
  watchFolders.push(fs.realpathSync(mobileNodeModules));
}
config.watchFolders = watchFolders;

config.resolver.nodeModulesPaths = [
  path.resolve(projectRoot, 'node_modules'),
  ...(mobileNodeModules ? [fs.realpathSync(mobileNodeModules)] : []),
  path.resolve(monorepoRoot, 'node_modules'),
].filter((entry, index, all) => entry && all.indexOf(entry) === index);

const realMobileNodeModules = mobileNodeModules
  ? fs.realpathSync(mobileNodeModules)
  : null;

config.resolver.extraNodeModules = {
  '@mitsuro/api': path.resolve(monorepoRoot, 'packages/api/src'),
  '@mitsuro/state': path.resolve(monorepoRoot, 'packages/state/src'),
  '@mitsuro/ui': path.resolve(monorepoRoot, 'packages/ui/src'),
};

function resolveExistingFile(basePath) {
  const candidates = [
    basePath,
    `${basePath}.tsx`,
    `${basePath}.ts`,
    `${basePath}.web.tsx`,
    `${basePath}.web.ts`,
    `${basePath}.js`,
    `${basePath}.jsx`,
    `${basePath}.mjs`,
    `${basePath}.cjs`,
    `${basePath}.css`,
    path.join(basePath, 'index.tsx'),
    path.join(basePath, 'index.ts'),
    path.join(basePath, 'index.js'),
    path.join(basePath, 'package.json'),
  ];
  for (const candidate of candidates) {
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
      if (candidate.endsWith('package.json')) {
        try {
          const pkg = JSON.parse(fs.readFileSync(candidate, 'utf8'));
          const entry = pkg.module || pkg.browser || pkg.main || 'index.js';
          const resolved = path.resolve(path.dirname(candidate), entry);
          if (fs.existsSync(resolved)) return resolved;
        } catch {
          // fall through
        }
      } else {
        return candidate;
      }
    }
  }
  return null;
}

function resolveMobileFile(subpath) {
  return resolveExistingFile(path.join(mobileRoot, subpath));
}

const defaultResolveRequest = config.resolver.resolveRequest;
config.resolver.resolveRequest = (context, moduleName, platform) => {
  if (moduleName === '@mobile' || moduleName.startsWith('@mobile/')) {
    const subpath = moduleName === '@mobile' ? 'index' : moduleName.slice('@mobile/'.length);
    const filePath = resolveMobileFile(subpath);
    if (filePath) {
      return { type: 'sourceFile', filePath };
    }
  }

  if (defaultResolveRequest) {
    return defaultResolveRequest(context, moduleName, platform);
  }
  return context.resolveRequest(context, moduleName, platform);
};

config.resolver.disableHierarchicalLookup = false;

module.exports = config;
