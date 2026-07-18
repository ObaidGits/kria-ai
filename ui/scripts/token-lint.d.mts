// Type declarations for the token linter (authored as plain ESM so it can run
// as a standalone CI script via `node`). Lets TypeScript type the test import.
export interface RawColorFinding {
  line: number;
  column: number;
  match: string;
  rule: "hex-color" | "rgb-color" | "hsl-color";
}

export declare function findRawColors(text: string): RawColorFinding[];
export declare const INCLUDE_DIRS: string[];
export declare const INCLUDE_EXTENSIONS: string[];
