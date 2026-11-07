// Makes the site work offline:
// - Serves/emits the files Tesseract.js (puzzle scanning) would otherwise load from a CDN (its worker, its WebAssembly
//   core, and the English training data) under `tesseract/`, see src/scan/scanFaceValues.ts.
// - Writes a service worker (offline/sw.js, registered by src/offline/registerServiceWorker.ts) that precaches
//   everything the user-facing pages can load.
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';

const require = createRequire(import.meta.url);
const packageDir = (name) => path.dirname(require.resolve(`${name}/package.json`));

// Published name (under tesseract/) => source file
const tesseractFiles = () => {
  const tesseractDir = packageDir('tesseract.js');
  const coreDir = packageDir('tesseract.js-core');
  const engDir = packageDir('@tesseract.js-data/eng');
  return {
    'worker.min.js': path.join(tesseractDir, 'dist/worker.min.js'),
    // We only use the LSTM engine (the default), which picks one of these depending on the browser's SIMD support
    'tesseract-core-lstm.wasm.js': path.join(coreDir, 'tesseract-core-lstm.wasm.js'),
    'tesseract-core-simd-lstm.wasm.js': path.join(coreDir, 'tesseract-core-simd-lstm.wasm.js'),
    'tesseract-core-relaxedsimd-lstm.wasm.js': path.join(coreDir, 'tesseract-core-relaxedsimd-lstm.wasm.js'),
    // The LSTM engine's (default) model
    'eng.traineddata.gz': path.join(engDir, '4.0.0_best_int/eng.traineddata.gz'),
  };
};

// Pages that should work offline (the others are developer tools)
const OFFLINE_PAGES = ['index.html', 'play.html'];

const listFiles = (dir, prefix = '') =>
  fs.existsSync(dir) ?
    fs
      .readdirSync(dir, { withFileTypes: true })
      .flatMap((entry) =>
        entry.isDirectory() ?
          listFiles(path.join(dir, entry.name), `${prefix}${entry.name}/`)
        : [`${prefix}${entry.name}`],
      )
  : [];

// Everything in the bundle that the offline pages can (transitively) load
const getReachableFiles = (bundle) => {
  const fileNames = Object.keys(bundle);
  const reachable = new Set();
  const queue = [];
  const add = (fileName) => {
    if (bundle[fileName] && !reachable.has(fileName)) {
      reachable.add(fileName);
      queue.push(fileName);
    }
  };

  OFFLINE_PAGES.forEach(add);
  fileNames.filter((fileName) => fileName.startsWith('tesseract/')).forEach(add);

  while (queue.length) {
    const output = bundle[queue.shift()];
    if (output.type === 'chunk') {
      output.imports.forEach(add);
      output.dynamicImports.forEach(add);
      output.viteMetadata?.importedCss.forEach(add);
      output.viteMetadata?.importedAssets.forEach(add);
    }

    // Catches what's only referenced by URL, e.g. the scripts in HTML pages, and workers
    const source = output.type === 'chunk' ? output.code : output.source;
    if (typeof source === 'string') {
      fileNames.filter((fileName) => source.includes(path.posix.basename(fileName))).forEach(add);
    }
  }
  return [...reachable];
};

export default function offline() {
  let config;

  return {
    name: 'slither-offline',

    configResolved(resolvedConfig) {
      config = resolvedConfig;
    },

    configureServer(server) {
      const files = tesseractFiles();
      server.middlewares.use((req, res, next) => {
        const match = /^\/tesseract\/([^/?#]+)/.exec(req.url ?? '');
        const file = match && files[match[1]];
        if (!file) {
          next();
          return;
        }
        res.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : 'application/octet-stream');
        fs.createReadStream(file).pipe(res);
      });
    },

    generateBundle() {
      if (config.command !== 'build') {
        return;
      }
      for (const [name, file] of Object.entries(tesseractFiles())) {
        this.emitFile({ type: 'asset', fileName: `tesseract/${name}`, source: fs.readFileSync(file) });
      }
    },

    writeBundle(outputOptions, bundle) {
      const outDir = outputOptions.dir ?? config.build.outDir;
      const publicFiles = config.publicDir ? listFiles(config.publicDir) : [];
      const precache = [...new Set([...getReachableFiles(bundle), ...publicFiles])].sort();

      const hash = createHash('sha256');
      for (const file of precache) {
        hash.update(file);
        hash.update(fs.readFileSync(path.join(outDir, file)));
      }
      const version = hash.digest('hex').slice(0, 16);

      const template = fs.readFileSync(path.join(import.meta.dirname, 'sw.js'), 'utf8');
      const serviceWorker = template
        .replace('__VERSION__', JSON.stringify(version))
        .replace('__PRECACHE__', JSON.stringify(precache, null, 2));
      fs.writeFileSync(path.join(outDir, 'sw.js'), serviceWorker);

      const totalBytes = precache.reduce((sum, file) => sum + fs.statSync(path.join(outDir, file)).size, 0);
      config.logger.info(
        `[offline] sw.js precaches ${precache.length} files (${(totalBytes / 1024 / 1024).toFixed(1)} MB), version ${version}`,
      );
    },
  };
}
