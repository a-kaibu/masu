# Bombadil browser checks

Build the packaged web runtime and install the web dependencies with pnpm. With
Chromium from Playwright and the i686 MinGW compiler installed, run from the
repository root:

```sh
BOMBADIL_CHROME=/path/to/chromium node tests/bombadil-browser.mjs target/release/masu
```

Without `BOMBADIL_CHROME`, Bombadil manages its own Chrome. The runner creates
two independent games and a disposable store. The browser stays on the app
origin for navigation. Games load assets from the local game listener and
development bucket.

Use `BOMBADIL_LIMIT=30s` for a shorter run or `BOMBADIL_PORT` if the default
app port, 9600, is busy. Set `BOMBADIL_TRACE` to a fresh path for each run. The
default path is `target/bombadil/trace.bin`. Use the same app port when
replaying a trace, and write the replay to a separate file:

```sh
BOMBADIL_REPRODUCE=/path/to/trace.bin BOMBADIL_TRACE=target/bombadil/replay.bin \
  BOMBADIL_CHROME=/path/to/chromium node tests/bombadil-browser.mjs target/release/masu
```

To inspect a trace or shrink a failing sequence, use `pnpm exec bombadil browser
inspect` or `pnpm exec bombadil browser shrink` from `web`. CI keeps failing
traces for three days.
