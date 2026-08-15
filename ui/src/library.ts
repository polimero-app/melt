export const LARGE_LIBRARY_THRESHOLD = 50

export function shouldDeferLibraryCards(fileCount: number): boolean {
  return fileCount > LARGE_LIBRARY_THRESHOLD
}

export type SortKey = 'name' | 'size' | 'modified'
export type FileSort = { key: SortKey; descending: boolean }

// The direction each key is most useful in: newest and largest first, but
// names read best from A to Z.
export const naturallyDescending: Record<SortKey, boolean> = { name: false, size: true, modified: true }

// ponytail: structural, so App.vue's FileEntry fits without moving the type out.
type SortableFile = { name: string; sizeBytes?: number; modifiedAt?: string }

// Always ascending; sortedBy applies the direction.
export function compareFiles(left: SortableFile, right: SortableFile, key: SortKey): number {
  if (key === 'size') return (left.sizeBytes ?? 0) - (right.sizeBytes ?? 0)
  // An absent or unparseable date sorts as the oldest rather than poisoning the
  // comparison with NaN, which would leave the order undefined.
  if (key === 'modified') return (Date.parse(left.modifiedAt ?? '') || 0) - (Date.parse(right.modifiedAt ?? '') || 0)
  return left.name.localeCompare(right.name)
}

export function sortedBy<T extends SortableFile>(files: T[], sort: FileSort): T[] {
  const direction = sort.descending ? -1 : 1
  return [...files].sort((left, right) => compareFiles(left, right, sort.key) * direction)
}

// Clicking the active column flips it; clicking a new one starts in its
// natural direction.
export function nextSort(current: FileSort, key: SortKey): FileSort {
  return current.key === key ? { key, descending: !current.descending } : { key, descending: naturallyDescending[key] }
}
