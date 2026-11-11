import { Skeleton } from '@/components/ui/skeleton'

import { TopBar } from './shell'

/**
 * The shapes of a vault's page while it loads: the tree on the left, a record on the right.
 *
 * Same boxes, radii and paddings as the real cards, so the page does not jump when the content
 * arrives. An AWS vault is the one that makes this worth having: listing parameters and opening one
 * takes a round trip to the account, often a second or more.
 */

const WIDTHS = ['w-2/3', 'w-1/2', 'w-3/4', 'w-2/5', 'w-3/5', 'w-1/2', 'w-2/3', 'w-2/5']

export function TreeSkeleton() {
  return (
    <aside className="flex min-h-0 flex-col rounded-[14px] border bg-card" aria-busy="true" aria-label="Carregando o cofre">
      <div className="flex items-center gap-2 px-3 pt-3 pb-2.5">
        <Skeleton className="h-8 flex-1 rounded-lg" />
        <Skeleton className="h-8 w-[62px] rounded-lg" />
      </div>
      <div className="flex gap-1.5 border-b px-3 pb-2.5">
        <Skeleton className="h-7 w-14 rounded-full" />
        <Skeleton className="h-7 w-20 rounded-full" />
        <Skeleton className="h-7 w-[72px] rounded-full" />
      </div>
      <div className="min-h-0 flex-1 overflow-hidden px-1.5 py-2">
        {WIDTHS.map((width, index) => (
          <div key={index} className="flex h-[34px] items-center gap-2 px-2" style={{ paddingLeft: index % 3 === 0 ? 8 : 30 }}>
            <Skeleton className="size-[22px] shrink-0" />
            <Skeleton className={`h-3.5 ${width}`} />
          </div>
        ))}
      </div>
      <div className="flex h-11 shrink-0 items-center border-t px-4">
        <Skeleton className="h-3.5 w-20" />
      </div>
    </aside>
  )
}

export function RecordSkeleton() {
  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden rounded-[14px] border bg-card" aria-busy="true" aria-label="Abrindo o registro">
      <div className="flex items-start gap-4 px-5 pt-5">
        <Skeleton className="size-12 shrink-0 rounded-xl" />
        <div className="min-w-0 flex-1 space-y-2.5 pt-1">
          <Skeleton className="h-5 w-56 max-w-full" />
          <div className="flex gap-1.5">
            <Skeleton className="h-5 w-28 rounded-full" />
            <Skeleton className="h-5 w-32 rounded-full" />
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2 px-5 pt-4 pb-4">
        <Skeleton className="h-9 w-40 rounded-lg" />
        <Skeleton className="h-9 w-24 rounded-lg" />
        <Skeleton className="size-9 rounded-lg" />
      </div>
      <div className="mx-5 mb-5 rounded-xl border">
        {[0, 1, 2, 3].map((row) => (
          <div key={row} className="flex items-center gap-4 border-b px-4 py-3.5 last:border-b-0">
            <div className="flex-1 space-y-2">
              <Skeleton className="h-3 w-20" />
              <Skeleton className={`h-4 ${WIDTHS[row + 2]}`} />
            </div>
            <Skeleton className="size-8 rounded-lg" />
          </div>
        ))}
      </div>
    </div>
  )
}

/** A whole vault page: while a new vault is being created, or a tab opens straight into one. */
export function VaultSkeleton() {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <TopBar>
        <Skeleton className="size-6 rounded-md" />
        <Skeleton className="h-4 w-36" />
      </TopBar>
      <div className="kh-wide grid min-h-0 flex-1 grid-cols-[var(--kh-tree,300px)_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] gap-5 p-6">
        <TreeSkeleton />
        <RecordSkeleton />
      </div>
    </div>
  )
}
