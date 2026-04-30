import { describe, expect, it, vi, beforeEach } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { MobileToolbarMenu } from './MobileToolbarMenu'

beforeEach(() => {
  vi.stubGlobal(
    'matchMedia',
    vi.fn().mockImplementation(() => ({
      matches: false,
      media: '',
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }))
  )
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0)
    return 1
  })
  vi.stubGlobal('cancelAnimationFrame', vi.fn())
})

describe('MobileToolbarMenu', () => {
  it('shows a single backend/model row and opens the shared picker', async () => {
    const user = userEvent.setup()
    const onOpenBackendModelPicker = vi.fn()

    render(
      <MobileToolbarMenu
        isDisabled={false}
        hasOpenPr={false}
        selectedBackend="claude"
        selectedProvider={null}
        backendModelLabel="Claude · Sonnet"
        backendModelLabelText="Claude · Sonnet"
        selectedEffortLevel="medium"
        selectedThinkingLevel="think"
        useAdaptiveThinking={false}
        isCodex={false}
        customCliProfiles={[]}
        uncommittedAdded={0}
        uncommittedRemoved={0}
        branchDiffAdded={0}
        branchDiffRemoved={0}
        prUrl={undefined}
        prNumber={undefined}
        displayStatus={undefined}
        checkStatus={undefined}
        activeWorktreePath={undefined}
        onSaveContext={vi.fn()}
        onLoadContext={vi.fn()}
        onCommit={vi.fn()}
        onCommitAndPush={vi.fn()}
        onOpenPr={vi.fn()}
        onReview={vi.fn()}
        onMerge={vi.fn()}
        onMergePr={vi.fn()}
        onResolveConflicts={vi.fn()}
        onOpenBackendModelPicker={onOpenBackendModelPicker}
        handlePullClick={vi.fn()}
        handlePushClick={vi.fn()}
        handleUncommittedDiffClick={vi.fn()}
        handleBranchDiffClick={vi.fn()}
        handleProviderChange={vi.fn()}
        handleEffortLevelChange={vi.fn()}
        handleThinkingLevelChange={vi.fn()}
        loadedIssueContexts={[]}
        loadedPRContexts={[]}
        loadedSecurityContexts={[]}
        loadedAdvisoryContexts={[]}
        loadedLinearContexts={[]}
        attachedSavedContexts={[]}
        handleViewIssue={vi.fn()}
        handleViewPR={vi.fn()}
        handleViewSecurityAlert={vi.fn()}
        handleViewAdvisory={vi.fn()}
        handleViewLinear={vi.fn()}
        handleViewSavedContext={vi.fn()}
        availableMcpServers={[]}
        enabledMcpServers={[]}
        activeMcpCount={0}
        onToggleMcpServer={vi.fn()}
      />
    )

    await user.click(screen.getByRole('button', { name: /more actions/i }))

    expect(screen.getByText('Backend / Model')).toBeInTheDocument()
    expect(screen.queryByText(/^Backend$/)).not.toBeInTheDocument()
    expect(screen.queryByText(/^Model$/)).not.toBeInTheDocument()
    expect(screen.queryByText(/^Mode$/)).not.toBeInTheDocument()

    await user.click(screen.getByText('Backend / Model'))

    expect(onOpenBackendModelPicker).toHaveBeenCalledTimes(1)
  })
})
