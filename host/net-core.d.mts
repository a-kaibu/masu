// Types of host/net-core.mjs for the page (web/src).
export declare var MASU_RETRY_STATUS: readonly number[];
export declare function masuIsChunk(url: string): boolean;
/** the first 16 bytes of the BLAKE3 hash, as hex */
export declare function masuBlake3(data: Uint8Array): string;
export declare function masuBlake3Js(data: Uint8Array): string;
export declare const masuBlake3Simd: {
  (): ((data: Uint8Array) => string) | null;
  /** the SIMD module's memory, once made */
  memory?: WebAssembly.Memory;
};
export declare function masuBackoff(attempt: number): number;
export declare var MASU_CLASS: { readonly game: 0; readonly ahead: 1; readonly hint: 2; readonly warm: 3; readonly save: 4 };
export declare var MASU_RATE: number;
export declare var MASU_BURST: number;
export declare var MASU_RESERVE: readonly number[];
export declare var MASU_BUDGET_BYTES: number;
export declare function masuBudgetShare(v: Float64Array): number;
export declare function masuBudgetFresh(v: Float64Array, now: number): void;
export declare function masuBudgetTakeStep(v: Float64Array, cls: number, now: number): boolean;
export declare function masuBudgetDrainStep(v: Float64Array, until: number): void;
export interface MasuDirectInfo {
  url: string;
  key: string;
  object: string;
  pack?: unknown;
}
export declare function masuDirectValid(info: unknown): info is MasuDirectInfo;
export interface MasuPackSpan {
  id: string;
  url: string;
  offset: number;
  len: number;
}
export declare var MASU_PACK_SPAN: readonly number[];
export declare var MASU_PACK_MAX: number;
export declare function masuPackValid(p: unknown): p is MasuPackSpan;
