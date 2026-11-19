// Types for palette.js (plain JavaScript so scripts/check-contrast.mjs can use it).
export interface SlidePalette {
  bg1: string;
  bg2: string;
  ink: string;
  dim: string;
  hi: string;
  track: string;
  glow: string;
  /** Every background text can sit on (gradient stops, glow over them). */
  bgs: string[];
}
export declare const MIN_CONTRAST: number;
export declare const GLOW_ALPHA: number;
export declare function mix(top: string, alpha: number, under: string): string;
export declare const HUE_SHIFT: Record<string, number>;
export declare function hexRgb(hex: string): [number, number, number];
export declare function rgbHex(rgb: number[]): string;
export declare function contrast(a: string, b: string): number;
export declare function slidePalette(accentHex: string, theme: "dark" | "light", kind: string): SlidePalette;
export declare function allPalettes(accentHex: string, theme: "dark" | "light"): (SlidePalette & { kind: string })[];
