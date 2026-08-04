export const LARGE_LIBRARY_THRESHOLD = 50

export function shouldDeferLibraryCards(fileCount: number): boolean {
  return fileCount > LARGE_LIBRARY_THRESHOLD
}
