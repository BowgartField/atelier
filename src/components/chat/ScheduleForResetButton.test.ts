import { describe, expect, it } from 'vitest'
import { getScheduleTriggerAvailability } from './ScheduleForResetButton'
import type { CodexUsageSnapshot } from '@/types/codex-cli'

function codexUsage(
  overrides: Partial<CodexUsageSnapshot>
): CodexUsageSnapshot {
  return {
    planType: 'pro',
    session: null,
    weekly: null,
    reviews: null,
    creditsRemaining: null,
    rateLimitReachedType: null,
    modelLimits: [],
    fetchedAt: 1_771_450_000,
    ...overrides,
  }
}

describe('getScheduleTriggerAvailability', () => {
  it('hides Codex session reset when only the weekly window is reported', () => {
    const availability = getScheduleTriggerAvailability(
      'codex',
      codexUsage({
        weekly: {
          usedPercent: 65,
          resetsAt: 1_772_023_891,
          limitWindowSeconds: 604_800,
        },
      })
    )

    expect(availability).toEqual({ session: false, weekly: true })
  })

  it('uses the session reset when both Codex windows exist and weekly is available', () => {
    const availability = getScheduleTriggerAvailability(
      'codex',
      codexUsage({
        session: {
          usedPercent: 12,
          resetsAt: 1_771_456_509,
          limitWindowSeconds: 18_000,
        },
        weekly: {
          usedPercent: 65,
          resetsAt: 1_772_023_891,
          limitWindowSeconds: 604_800,
        },
      })
    )

    expect(availability).toEqual({ session: true, weekly: false })
  })

  it('uses the weekly reset when the Codex weekly window is exhausted', () => {
    const availability = getScheduleTriggerAvailability(
      'codex',
      codexUsage({
        session: {
          usedPercent: 100,
          resetsAt: 1_771_456_509,
          limitWindowSeconds: 18_000,
        },
        weekly: {
          usedPercent: 100,
          resetsAt: 1_772_023_891,
          limitWindowSeconds: 604_800,
        },
      })
    )

    expect(availability).toEqual({ session: false, weekly: true })
  })

  it('uses the session reset when Codex weekly usage is below 100%', () => {
    const availability = getScheduleTriggerAvailability(
      'codex',
      codexUsage({
        session: {
          usedPercent: 100,
          resetsAt: 1_771_456_509,
          limitWindowSeconds: 18_000,
        },
        weekly: {
          usedPercent: 99.9,
          resetsAt: 1_772_023_891,
          limitWindowSeconds: 604_800,
        },
      })
    )

    expect(availability).toEqual({ session: true, weekly: false })
  })

  it('hides reset scheduling for backends without usage windows', () => {
    expect(getScheduleTriggerAvailability('opencode')).toEqual({
      session: false,
      weekly: false,
    })
  })
})
