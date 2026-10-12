/** The one place the site's own address is written. index.html, robots.txt, sitemap.xml (vite.config.ts) and the copy (src/config.ts) read it from here. */
export const SITE_URL = "https://userook.fun";
export const SITE_NAME = "Rook";
/** Where to report a vulnerability (launch-gates G6, docs/v6/phase1-scope.md O9): a mailto: or https: URL. Empty means none is
 *  published yet, and the build writes no /.well-known/security.txt rather than a placeholder. */
export const SECURITY_CONTACT = "";
/** RFC 9116's security.txt for a contact, expiring a year after `now`. Pure, so test/phase1.test.ts checks it. */
export function securityTxt(contact: string, now: Date, site = SITE_URL): string | null {
  const c = contact.trim();
  if (!c) return null;
  if (!/^(mailto:[^\s@]+@[^\s@]+|https:\/\/\S+)$/.test(c)) throw new Error(`SECURITY_CONTACT "${c}" is not a mailto: or https: URL`);
  const expires = new Date(now.getTime() + 365 * 86_400_000).toISOString().replace(/\.\d{3}Z$/, ".000Z");
  return `Contact: ${c}\nExpires: ${expires}\nPreferred-Languages: en\nCanonical: ${site}/.well-known/security.txt\n`;
}
