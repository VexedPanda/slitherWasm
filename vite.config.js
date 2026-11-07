import modify from 'rollup-plugin-modify';
import unassert from 'rollup-plugin-unassert';
import { defineConfig } from 'vite';

import offline from './offline/vite-plugin-offline.js';

/*
  - Build failure, trying `npm run build --max-old-space-size`
    - export NODE_OPTIONS=--max-old-space-size=32768
    - Maybe we should... ditch loading all the data? (remove references for the collections tests?)
 */

let chunkId = 0;

// The window => self rewrites below let our own code run in web workers. They're skipped for dependencies, which handle
// workers themselves (and e.g. paper.js would break, since it declares its own window variable).
const appOnly = (plugin) => ({
  ...plugin,
  transform: (source, id) => (id.includes('/node_modules/') ? null : plugin.transform(source, id)),
});

// https://vitejs.dev/config/
export default defineConfig({
  base: '',
  // Because of https://github.com/vitejs/vite/issues/12434?
  optimizeDeps: {
    // scenerystack's CommonJS dependencies need pre-bundling, since scenerystack itself is excluded
    exclude: ['scenerystack', 'tesseract.js', 'culori', 'pako'],
    include: [
      'scenerystack > base64-js',
      'scenerystack > big.js',
      'scenerystack > file-saver',
      'scenerystack > he',
      'scenerystack > himalaya',
      'scenerystack > linebreak-ts',
      'scenerystack > lodash',
      'scenerystack > paper',
      'scenerystack > seedrandom',
      'scenerystack > text-encoder-lite',
    ],
    entries: [],
  },

  plugins: [
    // Local copies of Tesseract's files, and the service worker (see offline/vite-plugin-offline.js)
    offline(),

    // Redirect window to self (see appOnly above)
    appOnly(
      modify({
        find: /([^.a-zA-Z0-9_])window(\.|\?\.|\[)/g,
        replace: (match, prefix, suffix) => `${prefix}self${suffix}`,
      }),
    ),

    // Standalone window to self
    appOnly(
      modify({
        find: /(\s+)window(\s+)/g,
        replace: (match, prefix, suffix) => `${prefix}self${suffix}`,
      }),
    ),
    appOnly(
      modify({
        find: /\(\s*window\s*\)/g,
        replace: '(self)',
      }),
    ),
    appOnly(
      modify({
        find: /\(\s*window\s*,/g,
        replace: '(self,',
      }),
    ),
    appOnly(
      modify({
        find: /\(\s*!window\s*\)/g,
        replace: '(!self)',
      }),
    ),
    appOnly(
      modify({
        find: /in window;/g,
        replace: 'in self;',
      }),
    ),
  ],

  build: {
    rollupOptions: {
      input: {
        index: '/index.html',
        play: '/play.html',
        'rule-explorer': '/rule-explorer.html',
        rule: '/rule.html',
        'scan-test': '/scan-test.html',
        'solver-fuzz': '/solver-fuzz.html',
        'discover-rules': '/discover-rules.html',
        'rules-test': '/rules-test.html',
        'rule-image': '/rule-image.html',
        'curated-rules': '/curated-rules.html',
        'filtered-rules': '/filtered-rules.html',
        'formal-concept-analysis': '/formal-concept-analysis.html',
        'pattern-boards': '/pattern-boards.html',
        hooks: '/hooks.html',
        'test/model-tests': '/test/model-tests.html',
        'test/correctness-tests': '/test/correctness-tests.html',
      },
      output: {
        chunkFileNames: () => `chunk-${chunkId++}.js`,
        entryFileNames: () => `entry-${chunkId++}.js`,
      },
      plugins: [
        unassert({
          // include: [ '**/**.js', '**/**.ts' ]
        }),

        // TODO: why are these extra ones needed? This is a wreck

        // Redirect window to self
        appOnly(
          modify({
            find: /([^.a-zA-Z0-9_])window(\.|\?\.|\[)/g,
            replace: (match, prefix, suffix) => `${prefix}self${suffix}`,
          }),
        ),

        // Standalone window to self
        appOnly(
          modify({
            find: /(\s+)window(\s+)/g,
            replace: (match, prefix, suffix) => `${prefix}self${suffix}`,
          }),
        ),
        appOnly(
          modify({
            find: /\(\s*window\s*\)/g,
            replace: '(self)',
          }),
        ),
        appOnly(
          modify({
            find: /\(\s*window\s*,/g,
            replace: '(self,',
          }),
        ),
        appOnly(
          modify({
            find: /\(\s*!window\s*\)/g,
            replace: '(!self)',
          }),
        ),
        appOnly(
          modify({
            find: /in window;/g,
            replace: 'in self;',
          }),
        ),
      ],
    },
  },
});
