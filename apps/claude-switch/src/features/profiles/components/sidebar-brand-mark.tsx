/**
 * Brand mark — the pixel mark beside the wordmark, in shizue's style: the
 * mark swaps to its dark variant with the theme, the wordmark is Bricolage
 * Grotesque 800. 32px keeps the 16-cell mark on whole pixels.
 */
export function SidebarBrandMark() {
  return (
    <div className="flex items-center gap-2 px-1.5 pt-0.5 pb-3" translate="no">
      <img src="/icon.svg" alt="" aria-hidden className="size-8 dark:hidden" />
      <img src="/icon-dark.svg" alt="" aria-hidden className="hidden size-8 dark:block" />
      <span className="font-display text-[19px] leading-none font-extrabold tracking-[-0.03em] text-ink">
        claude-switch
      </span>
    </div>
  )
}
