slitherWasm
================

A copy of [slither](https://github.com/jonathanolson/slither) by Jonathan Olson (MIT, see `LICENSE`) that keeps the same UI
and moves the hot puzzle logic (solving and generation) into a Rust → WebAssembly core under `wasm/`.

`npm run dev` at the root of the project to start the server, then navigate to the URL (e.g. http://localhost:5173/).

`npm run build` to build the project (can then be served from the `dist` directory).

`npx eslint .` to lint.

## WebAssembly core

`wasm/` is a Rust crate compiled to `wasm32-unknown-unknown`: a compact CDCL SAT solver (`sat.rs`) and the Slitherlink
encoding on top of it (`solver.rs`), which finds loops satisfying the clues by excluding sub-loops lazily. It replaces the
MiniSat (Emscripten) solver used by `satSolve`, without MiniSat's fixed 64 MB heap or keeping JS copies of clauses.

- `npm run build:wasm` builds it (needs Rust with the `wasm32-unknown-unknown` target) and embeds the module in
  `src/wasm/slitherCoreWasm.ts`, which is committed, so the app builds without Rust.
- `src/wasm/slitherCore.ts` is the JS side: each board is registered once, and solves pass face values and edge states
  through linear memory.
- `npm run test:solver` checks it against the original MiniSat-based solver on random states.
- `node test/node/run.mjs test/node/generationBenchmark.ts <size> <difficulty> <count>` times puzzle generation.
