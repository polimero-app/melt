// Temperature ceilings and the user-editable material presets that drive the
// control view's quick-set chips.
//
// Presets never leave the UI: applying one just calls `printer_temperature_set`
// like the steppers do, so they live in localStorage next to theme and locale
// rather than in backend preferences, which exist for state the Rust side
// actually consumes (spawning slicers, emitting notifications).

export type TemperaturePreset = {
  id: string
  name: string
  nozzleCelsius: number
  bedCelsius: number
}

/** Fallback ceilings when a printer does not advertise its own limits. */
export const temperatureMaximums: Record<'nozzle' | 'bed' | 'chamber', number> = {
  nozzle: 300,
  bed: 120,
  chamber: 60,
}

export const PRESET_NAME_MAX_LENGTH = 32

export const defaultPresets: TemperaturePreset[] = [
  { id: 'pla', name: 'PLA', nozzleCelsius: 210, bedCelsius: 60 },
  { id: 'petg', name: 'PETG', nozzleCelsius: 240, bedCelsius: 85 },
  { id: 'abs', name: 'ABS', nozzleCelsius: 255, bedCelsius: 100 },
]

export type PresetValidation = {
  nameRequired: boolean
  nameTooLong: boolean
  nozzleOutOfRange: boolean
  bedOutOfRange: boolean
}

export function presetValidationFailed(validation: PresetValidation): boolean {
  return Object.values(validation).some(Boolean)
}

function withinRange(value: unknown, maximum: number): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= maximum
}

export function validatePresetDraft(draft: {
  name: string
  nozzleCelsius: number | string
  bedCelsius: number | string
}): PresetValidation {
  const name = draft.name.trim()
  const nozzle = Number(draft.nozzleCelsius)
  const bed = Number(draft.bedCelsius)
  return {
    nameRequired: name.length === 0,
    nameTooLong: name.length > PRESET_NAME_MAX_LENGTH,
    nozzleOutOfRange: !withinRange(nozzle, temperatureMaximums.nozzle),
    bedOutOfRange: !withinRange(bed, temperatureMaximums.bed),
  }
}

function isValidPreset(value: unknown): value is TemperaturePreset {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Partial<TemperaturePreset>
  return (
    typeof candidate.id === 'string' &&
    candidate.id.length > 0 &&
    typeof candidate.name === 'string' &&
    candidate.name.trim().length > 0 &&
    candidate.name.length <= PRESET_NAME_MAX_LENGTH &&
    withinRange(candidate.nozzleCelsius, temperatureMaximums.nozzle) &&
    withinRange(candidate.bedCelsius, temperatureMaximums.bed)
  )
}

/**
 * localStorage is a trust boundary: the value survives upgrades and can be
 * hand-edited, so every entry is validated before it can drive a temperature
 * command. Absent or unparseable storage falls back to the shipped defaults;
 * a well-formed but empty list is honoured, since deleting every preset is a
 * legitimate choice.
 */
export function parseStoredPresets(raw: string | null | undefined): TemperaturePreset[] {
  if (raw === null || raw === undefined) return [...defaultPresets]
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return [...defaultPresets]
  }
  if (!Array.isArray(parsed)) return [...defaultPresets]
  return parsed.filter(isValidPreset).map((preset) => ({
    id: preset.id,
    name: preset.name.trim(),
    nozzleCelsius: preset.nozzleCelsius,
    bedCelsius: preset.bedCelsius,
  }))
}

/** Targets a preset applies, keyed by the control kinds it knows about. */
export function presetTargets(preset: TemperaturePreset): Record<'nozzle' | 'bed', number> {
  return { nozzle: preset.nozzleCelsius, bed: preset.bedCelsius }
}
