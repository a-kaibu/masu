# x86 翻訳器の作り直し (2026-11-01〜)

本編の CPU は bench.exe のようなループ中心のコードでは直接 wasm の 1.1〜1.3 倍まで来たが、
スクリプト VM のような「フラット」なコード (小さな関数・分岐・呼び出しが平らに広がる) では
桁違いに遅い。ゲームの起動・シーン切り替え・スクリプトの多い場面はこちらの形をしている。

## 計測: flatbench (Lua 5.4 + SQLite)

`flatbench.c` (Lua のスクリプト 5 種: 再帰 fib、表、文字列、比較関数つきソート、オブジェクト生成と、
SQLite のメモリ上 DB に 3 万行入れて検索・集計) を同じソースから 3 通りにビルドした。1 回あたり:

| | 秒 |
|---|---|
| ネイティブ (masu の Wine、32bit、このマシン) | 0.73 |
| 直接 wasm (emcc -O2、node) | 1.0 |
| JIT (node) | 8.0〜9.0 |
| JIT (Chromium 153 headless) | 7.8〜9.1 |

目標 (こた、2026-11-01): Chromium で 1 回 2 秒以内。

ゲームのスレッドの 80% が翻訳したコードの中 (Wine の wasm 側は 10%)。

### 何に時間を使っているか

- 制御の移り方 (`MASU_JIT_COUNT=<秒>` のカウンタ、1 秒あたり): 翻訳したブロックの実行が約 1.7 億
  (1 ブロック平均 2〜3 命令、1 ブロック約 6ns、ネイティブは約 1.5ns)、リージョンへの入口 2,200 万、
  ハッシュ表を引く間接ジャンプ・戻り 1,500 万、リージョン間の乗り換え 1,300〜2,700 万。
- V8 の出した機械語のサンプル (perf + `--perf-prof`、翻訳したコードの分だけ):
  - レジスタ間の mov/lea 40%
  - スタックへの退避・読み戻し 23%
  - 比較・分岐 18%
  - ゲストのメモリの読み書き 6%
  - 関数の入口 (先頭 0x60 バイト) 16%
- 上限を測った実験 (どれも同じ条件で A/B):
  - 範囲チェックを全部外す (`MASU_JIT_NOCHECK=1`、測定専用): -7%。
  - 保存リージョン (1 モジュール) だけで走らせる: 差なし。
  - リージョンの大きさの上限を 150〜3000 命令に: 差なし。
  - call/ret を wasm の call/return にする (`MASU_JIT_CALLS=1`、下記): 差なし。bench.exe の sort は
    0.9 → 2.7 秒 (qsort ↔ 比較関数がマージされなくなる)。
  - 出口の多いリージョンをその後走ったブロック込みで作り直す: 約 200 回作り直して遅くなった (外した)。
- マイクロベンチ (/tmp の hop、node): ゲストの call+ret 1 組は、今のリージョン間の末尾呼び出し
  (16 引数の return_call_indirect + ハッシュ表) で 16ns、wasm の call で 3.7ns、別モジュールへの
  call_indirect で 4.7ns。呼び出し先のモジュールが多いほど遅い (Chromium: 1 個 5ns、16 個 22ns、
  1024 個 96ns)。ただし flatbench では 1 モジュールにしても変わらなかったので、主因ではない。

結論: 遅さの主因は乗り換えの回数ではなく、**出している wasm の形**。リージョンは x86 の 8 レジスタ・
遅延フラグ 5 個・予算・Cpu・入口番号などが全部生きたままの大きな関数で、ブロックの境目ごとに
それらが合流する。TurboFan のレジスタ割り当てがこれをさばけず、移動と退避が仕事の 10 倍になる。
直接 wasm の C の関数は小さく、生きている値が少ないので速い。

## このコミットで入れたもの (既定の動作は変わらない)

- リージョン関数の型を「16 引数 → 16 戻り値」に (結果の並びは引数と同じ、先頭が状態)。 (2026-11-06: `MASU_JIT_CALLS` と一緒に戻して、戻り値は状態だけ)
  `RET_VALUES` を返したら状態は戻り値にある (EIP は 2 番目)。それ以外は今まで通り Cpu にある。
  ディスパッチャは `RET_VALUES` を Cpu に書いて続ける。
- `MASU_JIT_CALLS=1` (直接 call)、`=2` (間接 call も): 他のリージョンへの CALL を呼び出し先の
  リージョンの call_indirect にし、RET は状態を戻り値で返す (`RET_VALUES`)。差が出なかったので
  2026-11-06 に削除 (関数単位の翻訳が同じことをする)。
- `jmp [index*4 + 表]` の表の飛び先 (走ったもの) をリージョンに入れる (switch、インタプリタの
  オペコード分岐)。
- `MASU_JIT_COUNT=<秒>`: 生成コードが数える制御の移り方のカウンタ (call、ret、ハッシュ表、入口、
  ブロック、トップ/サイドのディスパッチ)。`MASU_JIT_STATS=1` ではリージョン間の乗り換えの回数と
  多い組 (`vnjit hop pairs`) も出る。
- `MASU_JIT_NOCHECK=1`: 範囲チェックを出さない (測定専用、安全でない)。

## 新しい翻訳器 (方針)

こたの指示 (2026-11-01): 「V8 に最適化された wasm を出力する Rust の wasm コンパイラを作成せよ。
パフォーマンス第一、徹底的にチューニング」。

1. **x86 → SSA の中間表現**。ゲストのレジスタ・フラグは IR の値で、ブロック引数で合流する。
2. **最適化**: フラグの生存解析を関数全体で (使われないフラグは計算しない、CALL/RET では死んで
   いるとみなすのは手書き asm に備えて選べるように)、定数・コピーの伝播、冗長なロードと範囲チェックの
   削除 (同じ基底の近いアクセスは 1 回)、ESP 相対のスタックの値をレジスタに (別名が無いと分かる範囲)。
3. **V8 向けの出力**: 関数単位 (ゲストの関数 = wasm の関数、call は wasm の call)、生きている値だけを
   ローカルに、構造化した制御 (トップのディスパッチを経ない)、関数の入口で受け渡すのは必要なレジスタ
   だけ。合流点で生きている値を最小にする。
4. **段階**: まず IR とその上の出力を、今の翻訳器と同じ入口・出口の約束で 1 リージョン単位に差し替えて
   flatbench と bench.exe で比べる。次に関数単位の呼び出し。各段で V8 の機械語の移動・退避の割合を
   測って次を決める。

## 関数単位の翻訳 (`MASU_JIT_FN`、2026-11-01、2026-11-02 から既定で有効。`MASU_JIT_FN=0` で従来のリージョン)

- 翻訳の単位をゲストの関数 (呼び出し先、または入口がミスしたアドレス) にした。1 単位が wasm の
  関数 `(cpu, block) -> status` 1 つで、状態は入口と出口で Cpu にある。表には単位ごとに
  リージョンの呼び出し規約のアダプタ (f0) と本体 (f1) を置き、ハッシュ表の項目に `FN_TAG` を付ける。
- CALL はハッシュ表を引いて `call_indirect` (呼び出し先も関数)、RET は `RET_FN` を返す。呼んだ側は
  EIP が戻りアドレスなら続け、違えば (longjmp、SEH、深すぎ、途中で抜けた) 状態をそのまま上へ返す。
  RET はステータスフラグを Cpu に書く。CALL は戻り先がフラグを読むときだけ先に書く (手書きの
  コードは結果をフラグで返す: Delphi RTL の GetDynaMethod は ZF。これを死んでいるとみなして
  Reunion が初回起動で壊れた。intops.c の flgret)。
- 小さい直接の呼び出し先 (64 命令まで、call なし) は、走ったかに関係なく展開する。
- ブロックの並べ方は支配木から作る (Ramsey "Beyond Relooper"、`jit/structure.rs`): ジャンプは
  すべて開いているラベルへの `br`。`jmp [index*4 + 表]` は表ごとの分岐ノード 1 つ (`br_table`、
  読んだ飛び先が表の値と同じか確かめる)。既約にならない辺と単位に無いブロックへは関数から出る。
- 関数から出た先 (と関数の無い呼び出し先、`jmp [IAT]` の飛び先) はリングに書き、ホストが
  走ったブロックを足して作り直す (1 関数 8 回まで)、または呼び出し先を関数にする。
- 8/16 ビットの ROL/ROR を JIT に (フラグが死んでいるとき)。sqlite のバイト入れ替えが
  インタプリタに落ちていた。intops.c に rotate を追加。

flatbench (node、1 回あたり、3 回の 1/2/3 回目): リージョン 9.1 / 9.1 / 8.8 秒 →
関数単位 8.3 / 5.5 / 5.2 秒。1 回目は V8 のコンパイル待ちと作り直しが重い。bench.exe は
calls が同じ (0.12 秒)、sort (qsort の比較関数を関数ポインタで呼ぶ) が 0.84 → 2.2 秒、
crc32 が遅い。チェックサムは全部同じ。

その後 (Chromium): レジスタのマスク、呼び出しの深さを引数に、`mov r,[i*4+T]; jmp r` の
ハッシュ分岐、BSF/TZCNT/BT、x87 キャッシュを関数全体に、リングの間引き。呼び出し 1 回
15.5 → 7.1 ns (node)、flatbench は 1 回目 6.2–7 秒、2 回目以降 3.2–3.8 秒 (リージョンは
6.8–7.5 秒、ネイティブ 0.6–0.73 秒)。ただし実ゲームでは負ける: game.html (Reunion、
40 秒、Chromium) で関数単位はタイトル 13.3–13.5 秒 / ストーリー 0.35 コア、リージョンは
12.3–12.7 秒 / 0.29–0.31 コア。既定にするのはこれを逆転させてから。
  原因はコンパイル量: 初回 50 秒で関数 約 9000 回・計 10 秒 (翻訳 98 万命令)、リージョンは
  3200 回・2.4 秒 (12 万命令)。呼び出し先ごとに別の関数になり、小さい呼び出し先を
  走ったかに関係なく展開するため。`MASU_JIT_FN_CALLED=1` (呼ばれた所だけ関数) でタイトル
  22 → 16 秒。段階式 (リージョンを先に作り、よく呼ばれる呼び出し先だけ関数に上げ、
  リージョンから関数を直接呼ぶ) も試したが、リージョンと関数の行き来が高く flatbench が
  7.3–9.5 秒 (リージョン以下) になったので戻した。`MASU_JIT_COUNT` が 1 秒あたりの
  コンパイル回数と時間も出す。関数はスレッド間で共有する (効果は小さい)。
- 関数も保存する (index "VNJ2": 項目ごとに build と間接呼び出し箇所、関数は表の 2 枠 =
  アダプタと本体。アダプタは本体を `call_indirect` で次の枠として呼ぶので、他のモジュールと
  まとめても番号がずれない)。Reunion 2 回目: タイトル 23 → 17.7 秒、ストーリー 0.44 →
  0.375 コア (リージョンは 13.7 秒 / 0.34)。保存サイズはリージョンの約 5 倍 (66 MB 対 14 MB)。

- コンパイル量をさらに削る (2026-11-02): 呼ばれていない所 (関数の途中) から作る関数は、
  コンパイル済みのブロックを含めない (d5140ff)。さらに関数に複数の入口を持たせた
  (f5e06e1): 戻り先と外側のループ先頭は、先頭の分岐ノード (`br_table`) から入れる。
  ハッシュ表のブロック欄は starts の何番目か (0 が先頭) で、アダプタが深さ引数の上位
  16 ビットで渡す。呼び出しは先頭にしか行かない。ループの内側は入口にできない (既約で
  なくなる) ので、そこからの分は今も別の関数になる。呼び出しのレジスタ保存は、前回の
  読み込み以降に書いたものだけ (5187716、効果は小)。
  Reunion 初回 (100 秒): コンパイル 8100 → 5600 回、タイトル 25 → 14.5 秒前後 (リージョン
  14.3–14.7 秒)。本編の CPU は全体 0.34–0.36 対 0.31–0.33 コア、最後の 20 秒は 0.26–0.27
  対 0.25–0.26 (序盤のコンパイルの差が残る)。flatbench (Chromium) は 1 回目 5.1 秒、
  2 回目以降 3.5–3.7 秒。試してだめだったもの: インライン量を減らす (変化なし)、小さい
  関数や途中からの関数のコンパイルを遅らせる (インタプリタが増えて CPU 増)、途中への
  脱出のたびに元の関数を作り直す (作り直しが重くタイトル +2 秒)。

- 生成コードの改善 (2026-11-02): ジャンプテーブル分岐の前のフラグを不要と判定、
  `and r,imm`/movzx の直後の表引き [表 + r*s] は範囲チェックなし、64 ビット比較
  (cmp+sbb) の B/BE/L/LE と ADC の CF をホストの EFLAGS 計算なしに (fib −15%、sort −9%、
  `tests/cmp64.c`)、関数内でレジスタの範囲チェック済みをブロックをまたいで引き継ぐ
  (フレームポインタのチェックが関数に 1 回)。flatbench の perf では Worker の 95% が
  生成コードで、特定の関数に偏らない (最大は luaV_execute の命令分岐ループで約 8%)。
  試してだめだったもの: 呼び出しでレジスタと予算を wasm の引数・複数戻り値で渡す
  (Cpu を経由しない)。V8 は 6 個を超える引数と複数の戻り値をスタックに置くので速く
  ならず、呼び出し 1 回のコードは 261 → 286 バイトに増えた。

- 自己書き換え: JIT したページを `CODE_PAGES` に記録し、mprotect (PROT_WRITE)、munmap、
  MAP_FIXED の mmap、NtFlushInstructionCache (`vnhost_jit_code_changed`) がそこに触れたら
  世代を上げ、各スレッドの表を捨てる。`tests/smc.c`。
- 誤翻訳の二分探索: `MASU_JIT_ONLY=lo-hi` (範囲外はインタプリタ)、`MASU_JIT_FN_ONLY=lo-hi`
  (範囲外はリージョン)。初回だけ壊れる不具合は、保存済みの `_shared-<版>` を消してから試す。

## Android: V8's bounds checks (2026-11-02)

Android's Chrome cannot enable V8's trap handler, so every wasm load and store gets an explicit
bounds check (docs/v8-wasm-shape.md). Measured on desktop Chromium with
`--js-flags=--wasm-enforce-bounds-checks` (flatbench, FN mode, median rep):

| | bounds checks | desktop |
|---|---|---|
| before | 7.05 s | 3.5 s |
| JIT modules import memory with minimum = wine.wasm's initial size (9527650) | 5.1 s | same |
| memory never grows: 65535 pages fixed, modules import min = max | 3.8-4.1 s | same |

A growable shared memory's size is loaded again for every access; a fixed one's once per function.
wine.wasm (and sake) now start with the whole 4 GB (-sINITIAL_MEMORY=4294901760
-sALLOW_MEMORY_GROWTH=0). **To check on a real Android device at the final check**: V8 commits the
initial memory read-write up front; Android's overcommit heuristics could refuse 4 GB on a device
with little RAM (2.06 GB worked before). If it does, go back to a growable memory with the minimum
at 33792 pages (5.1 s above).

### The fallback: sake-grow.wasm (2026-11-07)

crates/sake/build.sh links sake.wasm twice from the same objects: `sake.wasm` (4 GB fixed, as
above) and `sake-grow.wasm` (-sINITIAL_MEMORY = koji::jit::wasm::GROW_MIN_PAGES = 33792 pages,
2.06 GB: the guest's low 2 GB and 64 MB for sake; grows to 4 GB). The page (web/src/runtime.ts
`sakeBuild`) tries `new WebAssembly.Memory({initial: 65535, maximum: 65535, shared: true})` itself
and hands it to sake.js (`wasmMemory`, so it is reserved once); when the browser refuses it, it
loads sake-grow. The JIT's modules declare `memory_min_pages()` (decided at the first module from
the memory's size: 65535, or 33792 for sake-grow) to 65535, and their saved regions go by the
translator's version + `g` (they would not link the other way round). `&memory=grow` forces it;
`&grow=<MiB>` (tests) mallocs that much mid-run so that the memory grows. The run log's `build:`
note says which ran.

flatbench, desktop Chromium (Playwright's), FN mode, ms, 5 runs:

| | default | `--wasm-enforce-bounds-checks` (Android-like) |
|---|---|---|
| sake.wasm (4 GB fixed) | 2511 2783 2685 2559 2647 | 3178 3031 3188 3093 3023 |
| sake-grow.wasm | 2580 2597 2494 2668 2516 | 3401 3422 3401 3382 3382 |
| sake-grow.wasm, grown 512 MiB mid-run | 2621 2534 2698 2666 2806 | 3444 3277 3400 3421 3394 |

The fallback costs ~10 % with bounds checks, nothing without; node 22's V8 shows no difference
(2.75 s both with checks). host/tests pass on the node sake-grow.js too; bench group
`memory-grow` runs kernels on it (grown mid-run) with the same checks as cpu-game. Still open, on
the real device: whether 4 GB is refused there at all (the run log tells), and the memory a long
play uses.

## FN モードを既定に (2026-11-02)

flatbench (Chromium、中央値): FN 3.4 s / リージョン 9.0 s。範囲チェックあり (Android 相当) で 4.6 / 10.8 s。
Reunion (キャッシュなし): FN タイトル 15〜17 s・本編 0.35〜0.37 コア、リージョン 14〜16 s・0.32〜0.36 コア (ノイズの範囲で
ほぼ同じ)。直接呼び出しだけの合成ベンチ (cb.exe) はリージョンの方が速い (リージョンは呼び先を合併する)。

## Freeing modules nothing refers to (2026-11-07)

Each load (a region, a function's adapter and body, a batch of functions) is a module of its
own, referenced only by this thread's table. A merge, a function built again, or a function
taking a region's blocks leaves the old one unreachable: every jump into translated code reads
its table index from the hash table or the call cache (koji never bakes one into code). Every
64 loads (`MASU_JIT_RECLAIM`, 1 = every load, 0 = never) kura's `reclaim` marks the slots those
two name and clears the slots of loads none of whose slots is named (`vnvm_jit_free`), so V8 can
collect the module. The saved bundle stays (one instance; its regions are entered on demand).
Slots are not reused. Counts: `MASU_JIT_COUNT` (`freed N (KiB)`) and the crash log's
`jit modules` line. flatbench reps=3: 869 modules loaded (8.4 MB), 105 freed (1.5 MB).
host/tests pass with `MASU_JIT_RECLAIM=1` (CI runs them so on sake-grow).

## Why modules are compiled synchronously (2026-11-07)

A review asked for `WebAssembly.compile` (asynchronous) instead of `new WebAssembly.Module`.
It cannot help here: a promise resolves on its thread's event loop, and the program's threads
(pthreads, workers) never return to theirs; a module compiled on another worker reaches this
one only by `postMessage`, which the same loop delivers. The compiler thread already does the
translation off the program's thread; V8 compiles each function only when it is first called
(lazy compilation), so `vnvm_jit_load` is decoding, validation and instantiation.

Measured (`MASU_JIT_COUNT`'s `loading N ms`, the crash log's `jit modules` line), flatbench
reps=3 in node: 844 modules (8.9 MB) loaded in 505 ms in all, about 0.6 ms each: 5 to 15% of
each second while the code warms up (the first 5 s), nothing once the hot code is loaded. Less
of it means fewer and smaller modules (batches, merges, the saved bundle with
`compileStreaming`, which the page compiles asynchronously before the program starts).

## Functions take the registers as arguments (2026-11-14)

Android's Chrome has no trap handler for wasm memory, so every load and store the JIT emits
is bounds-checked; the review of 2026-11-13 asked for less Cpu-struct traffic.
`MASU_JIT_MEMCOUNT=<secs>` (measuring only: slow) makes every load and store the JIT emits
from then on count itself, by site (code, call, ret, entry, exit, call-rest, reload, flags)
and, for Cpu fields, by offset. flatbench, one window of 4 s:

| | Cpu loads + stores | guest and tables | Cpu share |
|---|---|---|---|
| before | 1 268 M (call 377, entry 255, ret 216, flags 141, code 183, exit 92) | 1 390 M | 48% |
| after | 168 M (code 83, call 49: the call cache's address, flags 28, call-rest 6) | 2 151 M | 7% |

A call stored the dirty registers and the budget, the callee loaded the ones it names, its
RET stored the dirty ones, the budget, EIP and the lazy flags, the caller loaded them all
again. Now `T_FN` is (cpu, ESP, EAX ECX EDX EBX EBP ESI EDI, depth | entry << 16, budget,
return address) -> (status, EIP, 8 registers, budget): nothing goes through the Cpu but the
flags, and those only when the caller may read them (the return address passed is 0 then; a
RET elsewhere stores them too). Inside a function the Cpu's registers are stale, so every
exit stores all 8.

ESP comes first: V8 passes the first five (x64) or six (arm64) parameters in registers and
leaves the others in the caller's frame, where it reads them again at each use. With ESP
eighth, cpu.exe's compose (a blend loop reading its arguments from [esp+n]) was 17% slower
with explicit bounds checks. Taking the registers through the Cpu at a function's entry
fixed that but lost the rest (flatbench 5% slower than before), for functions with loops
only too.

Speed (node, `--wasm-enforce-bounds-checks` as on Android, A/B with the build before, this
machine's noise is about 5%): flatbench 3-7% faster, cpu.exe script about 5-10%, compose
within 5%, the other cpu.exe cases the same. Without explicit checks (desktop): the same
or a little faster. Removing most of the Cpu traffic gave much less than its share of the
accesses: the bounds checks are not where the time goes.

tools/jit32test's fixed program "calls: flags, registers, RET elsewhere, tail jump, deep"
covers the convention (six mutants of it fail it: flags left out at a RET to a caller that
reads them, flags always left out, the state not stored on a RET elsewhere, the adapter not
storing it, a result dropped, two registers swapped between caller and callee).

## x87 loops (2026-11-14)

Profiling the x87 cases (`perf record -k 1` of node with `--perf-prof`, `perf inject --jit`)
showed that the FPU code spent its time on bookkeeping, not on arithmetic. The cache of the
x87 registers in wasm locals (FCache, function-wide since 2026-11-01) was there, but:

- L_MX, L_MXR and L_MXT (the control word's "arithmetic needs the slow path" and rounding
  masks) were loaded from the Cpu at the start of every block that does x87 arithmetic, a
  loop's too. Now a translation with x87 instructions loads them once at its start and again
  only after what may change the control word: FLDCW, a fallback to the interpreter, a call.
- Every FLD/FADD/FMUL m32/m64 read and wrote the status word in the Cpu to set IE for a
  signalling NaN and DE for a denormal. Now that happens in an `if` taken only for a NaN or a
  denormal operand.
- A product or quotient whose result is an exact zero (0 * y, x * 0, 0 / y) went to the
  rounding slow path every time (its result is below the smallest normal). An exact zero needs
  no rounding in any mode, so the condition now leaves it out.
- The function's root kept no cache (`k != root`): a function made from a loop's head (the
  common case, since the head is where the ring samples it) went through the Cpu at every
  iteration. A root that is entered at its first instruction only now keeps the cache; the
  way in from the caller checks that the cache's pending tags agree with the Cpu (written
  registers full and not exact, popped ones empty) and otherwise starts the interpreter there.
  Functions with more entries are as before.
- FXCH of a slot whose tags are in the Cpu swapped the tags (fempty, fexact) and the exact
  integer there. GCC's and MSVC's x87 code is full of FXCH pairs. Now FCache::src records the
  permutation and the swaps are made (as cycles) only before an instruction that reads or
  writes those tags, at a jump to another block, or at a flush; the instructions that leave
  them alone (FXCH, FLDZ/FLD1, FST/FSTP, loads, arithmetic on known slots) let it wait.

Speed (node, five interleaved rounds of the build before and this one, min / median in ms;
this machine is noisy, so the medians move by 20% or more between runs):

| | before | after |
|---|---|---|
| x87b chain (dependent FMUL/FADD m32) | 58 / 76 | 19 / 27 |
| x87b div | 34 / 63 | 18 / 29 |
| x87b arr | 21 / 30 | 15 / 21 |
| x87b mat (4x4 float) | 32 / 55 | 26 / 46 |
| x87b ftol | 19 / 37 | 17 / 31 |
| bench.exe float | 65 / 70 | 50 / 52 |
| cpu.exe script / mix | 117 / 154, 343 / 617 | 106 / 133, 340 / 498 |

Integer code (bench.exe calls/blend/lz, cpu.exe compose/transform, flatbench) stays the same
within the noise. Plain wasm does chain in 11 ms and float in 9: what is left is mostly the
80-bit semantics (the precision and rounding checks, the tags), not the bookkeeping.

tools/jit32test has new fixed programs: "x87 root loop" (four loops at a function's root,
starting at a random TOP with an exact integer near 2^60 in ST3: FXCH pairs, a single FXCH,
FILD after FXCH, zero and denormal products, FISTP m64 of the exact integer afterwards; each
with pads so that the root is a function and not a region) and "x87 fldcw call" (a callee
that flips the rounding mode with FLDCW and calls a leaf, so it is not inlined). Mutants they
catch: no moves for the put-off FXCHs (14 failures), no reload of the modes after a call, the
root's guard left out ("x87 loop push" fails in FN mode). The random runs (3000 programs,
seeds 7, 11, 31; 4000 seed 21) and TestFloat (302 371 cases) show no difference from the
interpreter.

## The exit contract: flags at mid-block exits (2026-11-22)

A translation leaves its code at many places: a bail to the interpreter before an instruction
(an address outside the guest range, a page to check, an instruction it does not translate), a
check after a store into code or a write of ESP, the budget at a jump back, a jump out of the
region or to a computed address. Until now the lazy flags were kept only where later code of
the same translation read them, so at an exit the Cpu could hold stale status flags:
`xor eax,eax / mov ecx,[0] / xor edx,edx` faulted at the load with EFLAGS 0x202 where the
interpreter has 0x246, and a guest's exception handler (masu's SEH hands it the context) saw
the wrong ZF/PF. jit32test and fuzz/x86_jit skipped the state comparison at exceptions for
this reason.

The contract (`jit::ExitAt`, `Gen::exit_contract`):

- **Before / After** an instruction (`bail`, `fallback`, `check_esp`, the store checks): every
  status flag exact. What follows is unknown here: an exception handler, or code that reads
  them.
- **Jump(need)** to code at an address (budget, a loop's hoisted checks, a callee missing from
  the hash table, an irreducible edge, a return): only the flags that code reads first, its
  live-in (`block_liveness`), or for a call `callee_reads`. A dynamic jump needs all of them.

The liveness (`flag_effects` / `liveness_of`) takes every exit as a read of all flags:
`may_leave_before` (memory operands other than the stack near ESP, LOCK, what Gen::emit does not
translate inline), `may_leave_after` (stores, ESP written), the stores of a block that checks
for code written (WATCH, FRESH), and ESP accesses after an instruction the interpreter ran that
set ESP to something new (`esp_unknown`; `Gen::fallback` keeps ESP's track over the
instructions that only push or pop). An exit the liveness did not see is recorded (`Exits`)
and the translation made again with it (`translate_exits`, at most 4 rounds; `EXIT_AGAIN`,
`EXIT_REFUSED`). With the predictions above jit32test's random programs need none: 1063
translations were made again in 600 programs before ESP's track was kept.

Keeping the flags at every exit has a cost in V8 where the flags would have been dead: the
lazy locals become loop phis. Two measures keep it small:

- `callee_reads` is recorded for every direct call, 0 included (a callee that writes the
  flags before it reads or leaves): before, a 0 was left out and taken as "all", which made the
  call keep and store them (calls +22%).
- A block every predecessor of which leaves its flags from the same 32-bit CMP or TEST of
  registers and immediates, none of them written after it (`remat_heads`), does that CMP/TEST
  again (`Remat`): the blocks before keep no flags for it, the jumps there set L_OP to
  L_OP_REMAT (-2) and the lazy locals to 0, and the CMP/TEST is done where something reads the
  flags (`remat_now`, into the locals) or where code leaves (`remat_out`, into the Cpu with
  L_OP -1). Into the Cpu, not the locals: register values in the lazy locals on the exits'
  shared code made sort's inner loop 25% slower in V8 although those paths never run (found by
  swapping modules, JB_SWAP). An entry of the function (its flags the Cpu's) does it only where
  L_OP says so. L_OP_REMAT never reaches the Cpu. The CMP/TEST stays pending over what
  neither reads nor writes the status flags, has no condition and keeps its registers
  (`remat_keeps`: MOVs, SSE, MMX, x87): done at the first of those that would read them, SSE's
  and x87's loops had it done at their start with selects on L_OP (sse_int +31%, x87_dot +21%
  in Chromium as an Android phone).

x64 (`long.rs`) keeps the same rule for its flag skip: `may_leave_before64` (memory operands
other than near RSP, LOCK, what it does not translate inline) reads every flag, so a write of
flags before such an instruction is never left out.

ROL/ROR with live flags no longer go to the interpreter: `rotate_flags` writes EFLAGS exactly
(crate::flags' bits; iced's RflagsBits are the liveness' and differ: masu-lint
jit-exit-contract forbids emitting them).

Tests: jit32test's fixed programs "exit fault between flag writers", "exit fault in a loop",
"rotate flags"; at both runs faulting it compares the whole state, flags included; fuzz/x86_jit
compares the status flags where the JIT stopped for the host; jit64test compares the state at
both runs faulting too. The harnesses define env.pages (masu-lint jit-harness-imports): without
it every module failed to load, jit32test took that for a refusal and compared the interpreter
with itself; a load error now fails it, and so does a run with no translation loaded.

tools/jitbench (README.md) measures it in V8: node, and Chromium as an Android phone (Pixel 7,
CPU throttled 4x, wasm bounds checks). Against main before it, 7 rounds, hot time B/A:

| program | node | Android-like |
|---|---:|---:|
| crc32 | 1.00 | 1.00 |
| bigadd | 1.00 | 1.01 |
| calls | 0.99 | 1.02 |
| vm | 1.05 | 1.00 |
| sse_int | 1.00 | 1.03 |
| x87_dot | 0.96 | 0.96 |
| sort | 1.03 | 1.11 |
| flags_mem | 0.97 | 1.11 |

Interpreter steps and exits the same, wasm +0-30% (sort's Remat), translation +3-14% (tenths
of a millisecond in node), peak memory the same. What is left (sort, flags_mem on the phone) is
the flags kept for the exits: the cost of the state being right where it was wrong.

## The translation context (2026-11-22)

What a translation is made under besides the guest's bytes is one value, `jit::TranslationContext`
(`jit/context.rs`), read once when the translation begins (`TranslationContext::now`):

- `arch`: 16-bit code with its code segment's base, 32-bit flat, long mode
- `calls`: regions, or guest functions as wasm functions (FN / FN64)
- `float`: the MXCSR flag checks (MX_EXACT); `pages`: the page checks (PAGE_CHECK); `ranges`:
  the range checks (off only when measuring, NOCHECK)
- `measure` (counters, profile, layout text, memory sites, probes), `budget` (loop slots,
  hoisting, function entries), `debug64` (long mode's debugging switches)

The generators (32-bit and long mode, packed, mxguard) read these only from it: masu-lint
jit-context fails a read of the statics anywhere in `crates/koji/src/jit/` but context.rs.
Before, they were read where used: a mode the host turned on while another thread translated
(the flag checks, the page checks) made a translation part one, part the other, kept as made
with the first; and the 16-bit code segment's base was a static every translation read, so
16-bit translations ran alone behind a gate (formal/tla/TranslateGate.tla checked it; gone with
the static). An instruction tells 16-bit code by its own decoding (`code16`: iced's code size).

A region keeps its context (`Region::ctx`): kura's shared cache keeps builds apart by it, runs
one only where `ctx.runs_now()` (the flag checks and page checks as they are now), and saves
only what `ctx.saveable()` (no flag checks, no counters of this run). Code (`wasm::Code`)
carries what it counts (`Code::measured`): memory sites and transfer counters of that
translation, not of whatever the statics say later.

`structure::Layout` is the one control flow graph both generators lay out with: its
predecessor lists (`pred`, every edge; and the reached ones in reverse postorder) are made once
and read by the Remat heads, the flag predictions, loop bodies and function entries.

The generated code is the same bytes as before (golden_jit unchanged, jitbench's modules the
same size); node, 7 rounds, hot time B/A 0.98-1.00, translation time within the noise (a cold
run's ±15%).
