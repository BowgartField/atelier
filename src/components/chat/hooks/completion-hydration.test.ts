import { describe, expect, it } from 'vitest'
import { shouldHydrateCompletedSessionFromBackend } from './completion-hydration'

describe('shouldHydrateCompletedSessionFromBackend', () => {
  it('requests hydration when plain-text plan content exists without a CodexPlan tool', () => {
    expect(
      shouldHydrateCompletedSessionFromBackend(
        'Repo inspected.\n\nPlan:\n- Implement changes\n- Add tests',
        [{ type: 'text', text: 'Repo inspected.' }],
        []
      )
    ).toBe(true)
  })

  it('requests hydration when a CodexPlan tool is present so cache picks up backend message id', () => {
    expect(
      shouldHydrateCompletedSessionFromBackend(
        'Repo inspected.\n\nPlan:\n- Implement changes\n- Add tests',
        [{ type: 'tool_use', tool_call_id: 'plan-1' }],
        [{ id: 'plan-1', name: 'CodexPlan', input: {} }]
      )
    ).toBe(true)
  })

  it('requests hydration when an ExitPlanMode tool is present (Claude plan)', () => {
    expect(
      shouldHydrateCompletedSessionFromBackend(
        '',
        [{ type: 'tool_use', tool_call_id: 'plan-1' }],
        [
          {
            id: 'plan-1',
            name: 'ExitPlanMode',
            input: { plan: '- step one\n- step two' },
          },
        ]
      )
    ).toBe(true)
  })
})
