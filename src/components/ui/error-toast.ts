import { toast } from "sonner"

/** Window event that asks the app shell to go to a page (`detail`: the route path). Lets a
 *  toast's action — or a store, which has no router — move the user to where the fix is. */
export const NAVIGATE_EVENT = "pg:navigate"

/** The pages an error can send someone to, by how messages name them. */
const PAGES: { pattern: RegExp; to: string; label: string }[] = [
  { pattern: /\bModels page\b|\bConnect a model\b/i, to: "/connections", label: "Open Models" },
  { pattern: /\bPrompts page\b/i, to: "/prompts", label: "Open Prompts" },
  { pattern: /\bGlossary page\b/i, to: "/glossary", label: "Open Glossary" },
  { pattern: /\bEpisodes page\b/i, to: "/episodes", label: "Open Episodes" },
]

/** Pure: the page a message points at, unless that's where the user already is. */
export function pageNamedIn(message: string, here: string): { to: string; label: string } | null {
  const page = PAGES.find((p) => p.pattern.test(message))
  return page && !here.startsWith(page.to) ? { to: page.to, label: page.label } : null
}

/** An error toast; when the message names a page ("…on the Models page"), its action goes
 *  there ("Open Models"). */
export function errorToast(message: string, options?: { description?: string }) {
  const page = pageNamedIn(message, window.location.pathname)
  toast.error(message, {
    ...options,
    action: page
      ? {
          label: page.label,
          onClick: () => window.dispatchEvent(new CustomEvent(NAVIGATE_EVENT, { detail: page.to })),
        }
      : undefined,
  })
}
