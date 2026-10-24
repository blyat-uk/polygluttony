import type { ComponentProps } from "react";
import { FolderOpen } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { usePickFolder } from "./use-pick-folder";

/** "Open folder": the folder dialog, then the folder's episodes. Gold by default — it's the
 *  next step wherever a page needs a folder first; pass `variant` where something else is. */
export function OpenFolderButton({
  children = "Open folder",
  ...props
}: Omit<ComponentProps<typeof Button>, "onClick">) {
  const { pick, pending } = usePickFolder();
  return (
    <Button disabled={pending} onClick={() => void pick()} {...props}>
      <FolderOpen weight="fill" className="size-4" />
      {children}
    </Button>
  );
}
