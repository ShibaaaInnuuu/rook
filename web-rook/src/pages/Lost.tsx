import { href } from "../lib/env";

export function Lost() {
  return (
    <div className="pagehead">
      <h1>Nothing here</h1>
      <p className="lede">That address names no page. <a href={href()}>Back to the board</a>.</p>
    </div>
  );
}
