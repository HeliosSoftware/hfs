// Rollup config for the CodeMirror 6 vendoring ritual. Never invoked by
// `cargo build`, `build.rs`, or CI — see README.md for the by-hand command.
//
// Bundles the ESM packages listed in `dependencies` (package.json) into a
// single minified IIFE that assigns `window.HfsCodeMirror`, then commits it
// at `../../assets/vendor/codemirror.bundle.js`.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import nodeResolve from '@rollup/plugin-node-resolve';
import terser from '@rollup/plugin-terser';

const here = dirname(fileURLToPath(import.meta.url));

/**
 * Reads `node_modules/<name>/package.json` directly off disk rather than
 * through Node's module resolver: several bundled packages (e.g.
 * `codemirror`) declare an `exports` map that does not expose `package.json`
 * as a subpath, so `require(`${name}/package.json`)` fails for them.
 */
function readPackageJson(name) {
  const path = join(here, 'node_modules', ...name.split('/'), 'package.json');
  return JSON.parse(readFileSync(path, 'utf8'));
}

/**
 * Every package bundled into `codemirror.bundle.js`, in the order their
 * exports appear in the output namespace. This list is the single source
 * of truth for both the license banner and `HfsCodeMirror.version` below —
 * add a package here (and to `dependencies`) rather than hand-editing either.
 */
const BUNDLED_PACKAGES = [
  'codemirror',
  '@codemirror/state',
  '@codemirror/view',
  '@codemirror/language',
  '@codemirror/commands',
  '@codemirror/autocomplete',
  '@codemirror/lint',
  '@codemirror/search',
  '@codemirror/lang-json',
  '@codemirror/lang-sql',
  '@lezer/common',
  '@lezer/highlight',
  'lezer-fhirpath',
];

const NOT_DECLARED = 'not declared in package metadata — to be confirmed before adoption';

/**
 * License string to use for a package whose own `package.json` has no
 * `license` field, keyed by package name. `lezer-fhirpath` declares its MIT
 * license only in its published README (see `../README.md` § "What's
 * bundled" for the citation); reaching into a tarball's README at build time
 * isn't worth the added parsing surface for one package, so the resolved
 * license is recorded here instead and used only as a fallback — a package
 * that later adds a proper `license` field keeps taking precedence over its
 * entry here.
 */
const LICENSE_OVERRIDES = {
  'lezer-fhirpath': 'MIT (declared in the package README, not in package.json)',
};

/** `{ package: version }`, read from each package's own `package.json`. */
const packageInfo = Object.fromEntries(
  BUNDLED_PACKAGES.map((name) => {
    const pkg = readPackageJson(name);
    return [name, { version: pkg.version, license: pkg.license ?? LICENSE_OVERRIDES[name] ?? NOT_DECLARED }];
  }),
);

const versions = Object.fromEntries(
  Object.entries(packageInfo).map(([name, info]) => [name, info.version]),
);

/**
 * Resolves the virtual `virtual:hfs-codemirror-versions` module imported by
 * `src/entry.js` to a JSON literal built from `packageInfo` above, so
 * `HfsCodeMirror.version` is generated from each package's `package.json`
 * rather than typed by hand (and drifts with it on every re-run).
 */
function versionsPlugin() {
  const id = 'virtual:hfs-codemirror-versions';
  const resolvedId = `\0${id}`;
  return {
    name: 'hfs-codemirror-versions',
    resolveId(source) {
      return source === id ? resolvedId : null;
    },
    load(source) {
      return source === resolvedId
        ? `export default ${JSON.stringify(versions)};\n`
        : null;
    },
  };
}

/**
 * Prepends the license banner in `generateBundle`, which runs after every
 * plugin's `renderChunk` hook — i.e. after terser. `output.banner` runs
 * *before* `renderChunk`, so terser's `comments: false` would strip it right
 * back out; this plugin is what actually makes the banner survive
 * minification.
 */
function bannerPlugin(banner) {
  return {
    name: 'hfs-codemirror-banner',
    generateBundle(_options, bundle) {
      for (const chunk of Object.values(bundle)) {
        if (chunk.type === 'chunk') {
          chunk.code = `${banner}\n${chunk.code}`;
        }
      }
    },
  };
}

const licenseBanner = [
  '/*!',
  ' * HfsCodeMirror — vendored bundle for the Helios FHIR Server UI.',
  ' * Ritual: crates/ui/vendor/codemirror/ (README.md). Regenerate by hand;',
  ' * never run at build time or in CI.',
  ' *',
  ' * Bundled packages:',
  ...Object.entries(packageInfo).map(
    ([name, info]) => ` *   - ${name}@${info.version} (${info.license})`,
  ),
  ' *',
  ' * The CodeMirror and Lezer packages above are MIT-licensed:',
  ' *   MIT License',
  ' *   Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others',
  ' *   Permission is hereby granted, free of charge, to any person obtaining a copy',
  ' *   of this software and associated documentation files (the "Software"), to deal',
  ' *   in the Software without restriction, including without limitation the rights',
  ' *   to use, copy, modify, merge, publish, distribute, sublicense, and/or sell',
  ' *   copies of the Software, and to permit persons to whom the Software is',
  ' *   furnished to do so, subject to the following conditions:',
  ' *',
  ' *   The above copyright notice and this permission notice shall be included in',
  ' *   all copies or substantial portions of the Software.',
  ' *',
  ' *   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR',
  ' *   IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,',
  ' *   FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE',
  ' *   AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER',
  ' *   LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,',
  ' *   OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN',
  ' *   THE SOFTWARE.',
  ' *',
  ' *   lezer-fhirpath is MIT-licensed too, declared in its published README',
  ' *   rather than a `license` field or a `LICENSE` file — see ../README.md,',
  ' *   section "What’s bundled", for the exact citation.',
  ' */',
].join('\n');

export default {
  input: 'src/entry.js',
  output: {
    file: '../../assets/vendor/codemirror.bundle.js',
    format: 'iife',
    // No `output.name`: `src/entry.js` has no `export` — it assigns
    // `window.HfsCodeMirror` itself as an explicit side effect (see its
    // header comment for why that beats Rollup's dotted iife name here).
  },
  plugins: [
    versionsPlugin(),
    nodeResolve(),
    terser({
      format: { comments: false },
    }),
    bannerPlugin(licenseBanner),
  ],
};
