import { createFileRoute, redirect } from "@tanstack/react-router";
import { EpisodesPage } from "@/features/episodes/episodes-page";
import { useAppStore } from "@/stores/app-store";

export const Route = createFileRoute("/episodes")({
  beforeLoad: () => {
    // Without a folder, Episodes means Welcome: anything navigating here lands
    // on "/" instead of a dead-end view.
    if (!useAppStore.getState().workdir) throw redirect({ to: "/" });
  },
  component: EpisodesPage,
});
