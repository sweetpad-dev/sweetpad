import { BuildTreeItem } from "./tree.js";

export async function buildScheme(item: BuildTreeItem) {
  await item.build();
}
