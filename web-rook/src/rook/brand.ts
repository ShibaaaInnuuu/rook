import type { BirdId } from "./marks";

/**
 * Which rook mark the site wears (docs/v6/site-design.md §5). Four birds are built (`perched`, `head`, `glyph`, `flight`; the
 * chess `piece` is kept, not offered); all are on #/brand at 16, 24, 32 and 96 px on both grounds. Switching is this one line,
 * plus the favicon: copy public/brand/favicon-<mark>.svg over public/favicon.svg.
 */
export const BRAND_MARK: BirdId | "piece" = "perched";   // the user's pick (2026-09-23): the first bird, the full-body perched rook
