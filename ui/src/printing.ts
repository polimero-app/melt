export type PrintTargetState = 'empty' | 'ready' | 'busy'

export function printTargetState(printerCount: number, busy: boolean): PrintTargetState {
  if (printerCount === 0) return 'empty'
  return busy ? 'busy' : 'ready'
}
