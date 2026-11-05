# Vendored libraries

Committed copies of the files the dashboard loads, so it needs no `npm install` and no bundler. Update them by hand from the npm packages and keep the versions here current.

| File | Package | Version | Source file | License |
|---|---|---|---|---|
| `preact.js` | `preact` | 10.29.8 | `dist/preact.module.js` | MIT, `LICENSE-preact` |
| `hooks.js` | `preact` | 10.29.8 | `hooks/dist/hooks.module.js` | MIT, `LICENSE-preact` |
| `htm.js` | `htm` | 3.1.1 | `dist/htm.module.js` | Apache-2.0, `LICENSE-htm` |

The source map comment is removed from the Preact files because the maps are not served. `index.html` maps the bare names `preact`, `preact/hooks` and `htm` to these files with an import map.

The server serves only the files listed in `ASSETS` in `src/server.rs`; add a file there when you add one here. The licenses are also in `third-party/`.
