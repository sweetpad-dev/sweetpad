export type PickItemRow<T> = {
  label: string;
  detail?: string;
  description?: string;
  iconId?: string;
  context: T;
};

export type PickItemSeparator = {
  kind: "separator";
  label: string;
};

export type PickItem<T> = PickItemRow<T> | PickItemSeparator;

export class UserPickCancelledError extends Error {}

export interface UserAsker {
  pick<T>(options: { title: string; items: PickItem<T>[] }): Promise<PickItemRow<T>>;
  input(options: { title: string; value?: string }): Promise<string | undefined>;
}
