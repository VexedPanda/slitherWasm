slitherWasm
================

A copy of [slither](https://github.com/jonathanolson/slither) by Jonathan Olson (MIT, see `LICENSE`) that keeps the same UI
and moves the hot puzzle logic (solving and generation) into a Rust → WebAssembly core under `wasm/`.

`npm run dev` at the root of the project to start the server, then navigate to the URL (e.g. http://localhost:5173/).

`npm run build` to build the project (can then be served from the `dist` directory).

`npx eslint .` to lint.
