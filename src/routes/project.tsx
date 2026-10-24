import { createFileRoute, redirect } from "@tanstack/react-router";

/** Project and Translate became Episodes; old links land there. */
export const Route = createFileRoute("/project")({
  beforeLoad: () => {
    throw redirect({ to: "/episodes" });
  },
});
