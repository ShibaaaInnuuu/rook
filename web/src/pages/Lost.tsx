import { href } from "../lib/env";

export function Lost() {
  return (
    <section className="lost">
      <h1>Nothing is cut here.</h1>
      <p>That address does not lead to a page on this site. The link may be mistyped, or it may point at something that was never made.</p>
      <a className="btn" href={href()}>Go to the overview</a>
    </section>
  );
}
