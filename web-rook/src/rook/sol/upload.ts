/**
 * Hosting a new coin's image and metadata JSON: the page posts the image (base64 in JSON) and the coin's fields to the site's own
 * server (POST /api/upload, server.mjs), which uploads both to Cloudinary with a signed upload and answers the metadata JSON's https
 * URL. The visitor never sees JSON, URLs or hosting steps. The uri is written into the coin's Metaplex metadata at launch and can never
 * change (Rook's configs make the metadata immutable).
 */
import { META_LIMITS } from "./dbc";

export type UploadFields = { name: string; symbol: string; description: string; website: string; twitter: string; telegram: string; collection: string };
export type Uploaded = { uri: string; image: string };
export class UploadError extends Error {
  constructor(message: string, readonly code: "not-configured" | "rate-limited" | "rejected" | "network") { super(message); }
}

/** Upload the image and the metadata JSON; resolves to the JSON's URL (the coin's uri) and the image's. */
export async function uploadCoin(image: string, f: UploadFields, fetchImpl: typeof fetch = fetch): Promise<Uploaded> {
  let r: Response;
  try {
    r = await fetchImpl("/api/upload", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ ...f, image }), credentials: "same-origin" });
  } catch {
    throw new UploadError("Couldn't reach the server to upload your image. Check your connection and try again.", "network");
  }
  const j = (await r.json().catch(() => ({}))) as { uri?: string; image?: string; error?: string; code?: string };
  if (r.status === 503) throw new UploadError("Image upload isn't set up yet, so coins can't be launched from this site right now.", "not-configured");
  if (r.status === 429) throw new UploadError(j.error ?? "Too many uploads from your connection. Try again later.", "rate-limited");
  if (!r.ok || !j.uri || !j.image) throw new UploadError(j.error ?? `The upload failed (HTTP ${r.status}). Try again.`, "rejected");
  const bad = uriProblem(j.uri);
  if (bad) throw new UploadError(`The upload returned an unusable address: ${bad}`, "rejected");
  return { uri: j.uri, image: j.image };
}

/** What is wrong with a metadata URL, or null. https, ipfs:// or ar:// only; at most 200 bytes (Metaplex). */
export function uriProblem(uri: string): string | null {
  const u = uri.trim();
  if (!u) return "The coin needs a metadata URL.";
  if (!/^(https:\/\/|ipfs:\/\/|ar:\/\/)\S+$/i.test(u)) return "The metadata URL must be https://, ipfs:// or ar://.";
  if (new TextEncoder().encode(u).length > META_LIMITS.uri) return `The metadata URL is at most ${META_LIMITS.uri} bytes.`;
  return null;
}

/** true / false when the server says whether uploads are configured; null when it could not be asked (the dev server has no /api). */
export async function uploadReady(fetchImpl: typeof fetch = fetch): Promise<boolean | null> {
  try {
    const r = await fetchImpl("/api/upload", { credentials: "same-origin" });
    if (!r.ok || !/json/.test(r.headers.get("content-type") ?? "")) return null;
    const j = (await r.json()) as { ready?: unknown };
    return typeof j.ready === "boolean" ? j.ready : null;
  } catch { return null; }
}
