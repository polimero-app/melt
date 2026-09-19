// Defaults from Bambuddy's print_scheduler presets: [°C, hours] for AMS 2 Pro
// (n3f, units 0-3) and AMS HT (n3s, units 128-135). The firmware owns the
// real limits; these only prefill the form.
export type DryingStatus = 'off' | 'checking' | 'drying' | 'cooling' | 'stopping' | 'error' | 'heatOutOfControl' | 'unknown'

export type AmsDrying = {
  status: DryingStatus
  active: boolean
  minutesRemaining?: number
  setting?: { filament?: string; temperatureC: number; hours: number }
  blockedReasons?: string[]
  controllable: boolean
}

const PRESETS: Record<string, { pro: [number, number]; ht: [number, number] }> = {
  PLA: { pro: [45, 12], ht: [45, 12] },
  PETG: { pro: [65, 12], ht: [65, 12] },
  TPU: { pro: [65, 12], ht: [75, 18] },
  ABS: { pro: [65, 12], ht: [80, 8] },
  ASA: { pro: [65, 12], ht: [80, 8] },
  PA: { pro: [65, 12], ht: [85, 12] },
  PC: { pro: [65, 12], ht: [80, 8] },
  PVA: { pro: [65, 12], ht: [85, 18] },
}
const ALIASES: Record<string, string> = { NYLON: 'PA', PA6: 'PA', PAHT: 'PA' }

export function dryingBounds(unitId: number): { min: number; max: number } | undefined {
  if (unitId >= 0 && unitId <= 3) return { min: 45, max: 65 }
  if (unitId >= 128 && unitId <= 135) return { min: 45, max: 85 }
  return undefined
}

/** True for heater fault states the badge must surface even though they are not `active`. */
export function dryingFault(drying: Pick<AmsDrying, 'status'>): boolean {
  return drying.status === 'error' || drying.status === 'heatOutOfControl'
}

/** True whenever a stop command should be offered: mid-cycle, or stuck in a fault state. */
export function dryingStoppable(drying: Pick<AmsDrying, 'active' | 'status'>): boolean {
  return drying.active || dryingFault(drying)
}

export function dryingDefaults(unitId: number, filament?: string) {
  if (!dryingBounds(unitId)) return undefined
  const raw = (filament ?? '').split(' ')[0].toUpperCase()
  const key = [raw, raw.split('-')[0]].map((candidate) => ALIASES[candidate] ?? candidate).find((candidate) => PRESETS[candidate]) ?? 'PLA'
  const [temperatureC, hours] = PRESETS[key][unitId >= 128 ? 'ht' : 'pro']
  return { temperatureC, hours, filament: key }
}
