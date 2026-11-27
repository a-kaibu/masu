# Startup measurements

The `cpu-kernels` suite's `startup` metric for `nop.exe` measures from navigation to
the guest's `BENCH-END` console line. The samples below use the same endpoints.

## 2026-11-27

Headless Chromium 159.0.8154.6 with Playwright 1.65.1 on Ubuntu 24.04 under WSL2
(Core i7-11700). The local `masu serve` and Web runtime came from CI for `1ebe4bc7`.
The fixed-memory `sake.wasm` had build ID `d6fc16c2a439bcd4` and was 29.7 MiB
decoded (6.3 MiB as zstd). A fixture catalog provided `nop.exe` and the 32-bit
prefix. The fixture excluded installed RTPs.

| Startup | Samples (ms, in run order) | Median (ms) |
|---|---|---:|
| Saved on device, offline | 1787, 1886, 1562, 1868, 1741 | 1787 |
| Cold, CDN route | 8986, 24113, 24189, 24025, 23925 | 24025 |
| Cold, bucket route | 9417, 24426, 24239, 9305, 24224 | 24224 |
| Cold, server route | 23695, 23714, 23656, 23681, 23750 | 23695 |

Offline used the library's **端末に保存** action, then `context.setOffline(true)`.
The five runs shared the saved profile. To repeat, build `web/dist` with the
runtime in `web/dist/sake` and run:

```sh
EXE=/path/to/nop.exe FONTS=/path/to/drive_c/windows/Fonts \
  EXPECT='^BENCH-END$' OFFLINE_ROUNDS=5 \
  node tests/offline-browser.mjs /path/to/masu
```

Cold runs alternated CDN, bucket and server, with a fresh browser context per
sample. CDP `Network.emulateNetworkConditions` set 8 Mbit/s in both directions
and 80 ms latency, as with `NET=8000,80` in `tests/play.mjs`. The page used
`autostart&headless`. The CDN route used `wasmcdn` with a localhost HTTP origin
serving the release layout `sake/<build-id>/<name>.wasm.zst` with CORS. The
bucket route used `masu serve --dev-bucket`, with its Wasm publication confirmed
at `/sake-direct/sake.wasm` before timing. The server route used `wasm=server`.
All three kept the same bucket for game chunks. Each run's flight recorder
confirmed `sake wasm: cdn`, `bucket` or `server` respectively.

The longer cold samples spent about 15.8 seconds waiting on a game chunk after
the Wasm had loaded. They measure the full startup over the shaped connection.
The CDN and bucket Wasm fetch and decoding took around 7 seconds in those runs.
