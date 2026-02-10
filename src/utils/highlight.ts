import type { HighlightRange } from "../types";

/**
 * Split text into segments based on byte-offset highlight ranges.
 * Returns array of { text, highlighted } segments for rendering.
 */
export function applyHighlights(
  text: string,
  ranges: HighlightRange[],
): { text: string; highlighted: boolean }[] {
  if (ranges.length === 0) {
    return [{ text, highlighted: false }];
  }

  // Convert byte offsets to char offsets using TextEncoder
  const encoder = new TextEncoder();
  const bytes = encoder.encode(text);
  const byteToChar = new Map<number, number>();
  let byteIdx = 0;
  for (let charIdx = 0; charIdx < text.length; charIdx++) {
    byteToChar.set(byteIdx, charIdx);
    const codePoint = text.codePointAt(charIdx)!;
    if (codePoint > 0xffff) {
      charIdx++; // surrogate pair
    }
    const charBytes = encoder.encode(String.fromCodePoint(codePoint));
    byteIdx += charBytes.length;
  }
  byteToChar.set(bytes.length, text.length);

  const segments: { text: string; highlighted: boolean }[] = [];
  let lastCharEnd = 0;

  for (const range of ranges) {
    const charStart = byteToChar.get(range.start) ?? 0;
    const charEnd = byteToChar.get(range.end) ?? text.length;

    if (charStart > lastCharEnd) {
      segments.push({ text: text.slice(lastCharEnd, charStart), highlighted: false });
    }
    segments.push({ text: text.slice(charStart, charEnd), highlighted: true });
    lastCharEnd = charEnd;
  }

  if (lastCharEnd < text.length) {
    segments.push({ text: text.slice(lastCharEnd), highlighted: false });
  }

  return segments;
}
