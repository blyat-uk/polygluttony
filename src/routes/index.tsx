import { createFileRoute } from "@tanstack/react-router";
import { WelcomePage } from "@/features/welcome/welcome-page";
import {
  connectionsQuery,
  providersQuery,
  syncConnectionStore,
} from "@/features/connections/use-connections";

export const Route = createFileRoute("/")({
  beforeLoad: async ({ context }) => {
    // Seed the connections (Translate's model) before first paint so the status bar label
    // and the rail badge / Translate gating are right everywhere without visiting Connections.
    // The results land in the query cache the Connections page reads.
    const [cfg, providers] = await Promise.all([
      context.queryClient.ensureQueryData(connectionsQuery),
      context.queryClient.ensureQueryData(providersQuery),
    ]);
    syncConnectionStore(cfg, providers);
  },
  component: WelcomePage,
});
