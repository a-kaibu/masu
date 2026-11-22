// BLAKE3 for store chunk names: host/net-core.mjs's, the one the network worker runs too (one
// source: review 22bb061 §7), checked against the blake3 crate's answers (testdata/blake3.tsv).
import { masuBlake3, masuBlake3Js, masuBlake3Simd } from "../../host/net-core.mjs";

/** The first 16 bytes of `data`'s BLAKE3 hash, as hex: a store chunk's name (crates/masu/src/pack.rs). */
export const blake3Hex: (data: Uint8Array) => string = masuBlake3;
/** blake3Hex in plain JavaScript (where there is no wasm SIMD, and small inputs). */
export const blake3Scalar: (data: Uint8Array) => string = masuBlake3Js;
/** blake3Hex with wasm SIMD, or null without it. */
export const blake3Simd = (): ((data: Uint8Array) => string) | null => masuBlake3Simd();
/** Tests: the SIMD module's memory, in bytes (0 before it is made). */
export const simdMemoryBytes = (): number => masuBlake3Simd.memory?.buffer.byteLength ?? 0;
