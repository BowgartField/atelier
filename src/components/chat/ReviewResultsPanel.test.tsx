import { beforeEach, describe, expect, it } from 'vitest'
import { render, screen } from '@/test/test-utils'
import { useChatStore } from '@/store/chat-store'
import { ReviewResultsPanel } from './ReviewResultsPanel'
import type { ReviewResponse } from '@/types/projects'

describe('ReviewResultsPanel', () => {
  beforeEach(() => {
    useChatStore.setState({
      reviewResults: {},
      fixedReviewFindings: {},
      reviewSidebarVisible: false,
    })
  })

  it('shows review metadata and failure scenario for structured findings', () => {
    const reviewResults: ReviewResponse = {
      summary: 'One high-confidence correctness issue found.',
      approval_status: 'changes_requested',
      findings: [
        {
          severity: 'warning',
          category: 'correctness',
          confidence: 'high',
          blocking: true,
          introduced_by_diff: true,
          file: 'src/App.tsx',
          line: 42,
          title: 'Null access after guard removal',
          description:
            'The new code dereferences value after removing a guard.',
          failure_scenario: 'When value is null, rendering throws.',
          suggestion: 'Restore the null guard before dereferencing value.',
        },
      ],
    }

    useChatStore.getState().setReviewResults('session-1', reviewResults)

    render(<ReviewResultsPanel sessionId="session-1" />)

    expect(screen.getByText('Correctness')).toBeInTheDocument()
    expect(screen.getByText('High confidence')).toBeInTheDocument()
    expect(screen.getByText('Blocking')).toBeInTheDocument()
    expect(screen.getByText('Introduced by diff')).toBeInTheDocument()
    expect(screen.getByText('Failure Scenario')).toBeInTheDocument()
    expect(
      screen.getByText('When value is null, rendering throws.')
    ).toBeInTheDocument()
    expect(screen.queryByText(/praise/i)).not.toBeInTheDocument()
  })
})
