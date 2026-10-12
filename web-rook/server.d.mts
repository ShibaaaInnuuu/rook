import type { IncomingMessage, ServerResponse } from "node:http";

export const SECURITY: Record<string, string>;
export const MAX_IMAGE_BYTES: number;
export const MAX_BODY_BYTES: number;
export const UPLOADS_PER_HOUR: number;
export const FOLDER: string;
export type CoinFields = { name: string; symbol: string; description: string; website: string; twitter: string; telegram: string; collection: string };
export const COLLECTION_SLUGS: ReadonlySet<string>;
export function sniffImage(b: Uint8Array | null | undefined): "image/png" | "image/jpeg" | "image/gif" | "image/webp" | null;
export function coinMetadata(f: CoinFields, image: string, mime: string): Record<string, unknown>;
export function checkFields(j: unknown): { fields: CoinFields; error?: undefined } | { error: string; fields?: undefined };
export function decodeImage(v: unknown): Buffer | null;
export function cloudinarySignature(params: Record<string, string>, secret: string): string;
export function cloudinaryEnv(env: Record<string, string | undefined>): { cloudName: string; apiKey: string; apiSecret: string } | null;
export function rateLimiter(limit?: number, windowMs?: number): { take(key: string, now: number): { ok: boolean; retryAfter: number } };
export function clientIp(req: IncomingMessage): string;
export function createHandler(opts?: {
  root?: string; env?: Record<string, string | undefined>; fetch?: typeof fetch; now?: () => number;
  limiter?: ReturnType<typeof rateLimiter>;
}): (req: IncomingMessage, res: ServerResponse) => Promise<void>;
