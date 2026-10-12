import { Docs } from "./pages/Docs";
import { HOW_IT_WORKS } from "./rook/routes";
import { Trust } from "./pages/Trust";
import { Brand } from "./pages/Brand";
import { Privacy, Risk, Terms } from "./pages/Legal";

/** Pages that read no chain and need no wallet code: their own small chunk. */
export default function StaticPages({ route }: { route: string[] }) {
  const [at, a] = route;
  return at === "docs" ? <Docs name={a} /> : at === HOW_IT_WORKS ? <Docs name={HOW_IT_WORKS} /> : at === "trust" ? <Trust /> : at === "brand" ? <Brand /> : at === "terms" ? <Terms /> : at === "risk" ? <Risk /> : <Privacy />;
}
