import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { useUIStore } from '@/store/ui-store'
import { WorkflowRunsModal } from './WorkflowRunsModal'

const mockUseWorkflowRuns = vi.hoisted(() => vi.fn())
const mockUseWorkflowRun = vi.hoisted(() => vi.fn())
const mockUseWorkflowJobLogs = vi.hoisted(() => vi.fn())
const mockUsePreferences = vi.hoisted(() => vi.fn())
const mockUseCreateSession = vi.hoisted(() => vi.fn())
const mockUseSendMessage = vi.hoisted(() => vi.fn())
const mockUseSetSessionBackend = vi.hoisted(() => vi.fn())
const mockUseSetSessionModel = vi.hoisted(() => vi.fn())
const mockUseSetSessionProvider = vi.hoisted(() => vi.fn())

vi.mock('@/services/github', async () => {
  const actual = await vi.importActual('@/services/github')
  return {
    ...actual,
    useWorkflowRuns: mockUseWorkflowRuns,
    useWorkflowRun: mockUseWorkflowRun,
    useWorkflowJobLogs: mockUseWorkflowJobLogs,
  }
})

vi.mock('@/services/preferences', () => ({
  usePreferences: mockUsePreferences,
}))

vi.mock('@/services/chat', () => ({
  useCreateSession: mockUseCreateSession,
  useSendMessage: mockUseSendMessage,
  useSetSessionBackend: mockUseSetSessionBackend,
  useSetSessionModel: mockUseSetSessionModel,
  useSetSessionProvider: mockUseSetSessionProvider,
  chatQueryKeys: {
    sessions: vi.fn(),
  },
}))

vi.mock('@/lib/transport', () => ({
  invoke: vi.fn(),
}))

vi.mock('@/lib/platform', async () => {
  const actual = await vi.importActual('@/lib/platform')
  return {
    ...actual,
    openExternal: vi.fn(),
  }
})

function makeMutation() {
  return { mutate: vi.fn(), mutateAsync: vi.fn() }
}

function makeRun(id: number, workflowName: string, displayTitle: string) {
  return {
    databaseId: id,
    name: workflowName.toLowerCase(),
    displayTitle,
    status: 'completed',
    conclusion: 'success',
    event: 'push',
    headBranch: 'main',
    createdAt: '2026-07-03T09:00:00Z',
    url: `https://github.com/acme/project/actions/runs/${id}`,
    workflowName,
  }
}

function renderModal() {
  useUIStore.setState({
    workflowRunsModalOpen: true,
    workflowRunsModalProjectPath: '/tmp/project-1',
    workflowRunsModalBranch: null,
    workflowRunsModalWorkflowName: null,
  })
  render(<WorkflowRunsModal />)
}

describe('WorkflowRunsModal', () => {
  beforeEach(() => {
    HTMLElement.prototype.scrollIntoView = vi.fn()
    mockUseWorkflowRuns.mockReset()
    mockUseWorkflowRun.mockReset()
    mockUseWorkflowJobLogs.mockReset()
    mockUsePreferences.mockReset()
    mockUseCreateSession.mockReset()
    mockUseSendMessage.mockReset()
    mockUseSetSessionBackend.mockReset()
    mockUseSetSessionModel.mockReset()
    mockUseSetSessionProvider.mockReset()

    mockUsePreferences.mockReturnValue({
      data: {
        magic_prompts: {},
        magic_prompt_models: {},
        magic_prompt_modes: {},
        magic_prompt_providers: {},
        custom_cli_profiles: [],
        parallel_execution_prompt_enabled: false,
        chrome_enabled: false,
      },
      isLoading: false,
      isFetching: false,
    })

    mockUseCreateSession.mockReturnValue(makeMutation())
    mockUseSendMessage.mockReturnValue(makeMutation())
    mockUseSetSessionBackend.mockReturnValue(makeMutation())
    mockUseSetSessionModel.mockReturnValue(makeMutation())
    mockUseSetSessionProvider.mockReturnValue(makeMutation())
    mockUseWorkflowJobLogs.mockReturnValue({
      data: [
        {
          stepName: 'Checkout',
          timestamp: '2026-07-03T09:01:02Z',
          message: 'Fetching repository',
        },
        {
          stepName: 'Checkout',
          timestamp: '2026-07-03T09:01:03Z',
          message: '##[group]Runner Image',
        },
        {
          stepName: 'Checkout',
          timestamp: '2026-07-03T09:01:04Z',
          message: '##[endgroup]',
        },
        {
          stepName: 'Checkout',
          timestamp: '2026-07-03T09:01:05Z',
          message: '##[error]Process completed with exit code 1.',
        },
      ],
      isLoading: false,
      error: null,
    })

    useUIStore.setState({
      workflowRunsModalOpen: false,
      workflowRunsModalProjectPath: null,
      workflowRunsModalBranch: null,
      workflowRunsModalWorkflowName: null,
    })
  })

  it('opens a dedicated run modal with job nodes and selectable steps', async () => {
    const runs = [
      makeRun(101, 'CI', 'CI workflow'),
      makeRun(102, 'Deploy', 'Deploy workflow'),
    ]

    mockUseWorkflowRuns.mockReturnValue({
      data: { runs, failedCount: 0 },
      isLoading: false,
      isFetching: false,
    })
    mockUseWorkflowRun.mockImplementation((projectPath, runId) => {
      if (!projectPath || !runId) {
        return { data: undefined, isLoading: false, isFetching: false }
      }

      if (runId === 101) {
        return {
          data: {
            jobDefinitions: [{ id: 'build', name: 'build', needs: [] }],
            jobs: [
              {
                databaseId: 201,
                name: 'build',
                status: 'completed',
                conclusion: 'success',
                startedAt: '2026-07-03T09:01:00Z',
                completedAt: '2026-07-03T09:03:00Z',
                url: 'https://github.com/acme/project/actions/jobs/201',
                steps: [
                  {
                    name: 'Checkout',
                    number: 1,
                    status: 'completed',
                    conclusion: 'success',
                  },
                  {
                    name: 'Run tests',
                    number: 2,
                    status: 'completed',
                    conclusion: 'success',
                  },
                ],
              },
            ],
          },
          isLoading: false,
          isFetching: false,
        }
      }

      return {
        data: {
          jobDefinitions: [{ id: 'deploy', name: 'deploy', needs: [] }],
          jobs: [
            {
              databaseId: 202,
              name: 'deploy',
              status: 'completed',
              conclusion: 'success',
              startedAt: '2026-07-03T09:05:00Z',
              completedAt: '2026-07-03T09:07:00Z',
              url: 'https://github.com/acme/project/actions/jobs/202',
              steps: [
                {
                  name: 'Prepare release',
                  number: 1,
                  status: 'completed',
                  conclusion: 'success',
                },
                {
                  name: 'Ship release',
                  number: 2,
                  status: 'completed',
                  conclusion: 'success',
                },
              ],
            },
          ],
        },
        isLoading: false,
        isFetching: false,
      }
    })

    const user = userEvent.setup()
    renderModal()

    expect(screen.queryByText('Checkout')).not.toBeInTheDocument()
    await user.click(screen.getByText('CI workflow'))
    await user.click(
      await screen.findByRole('button', {
        name: 'View build job details',
      })
    )

    expect(await screen.findByText('Checkout')).toBeInTheDocument()
    expect(screen.getByText('Run tests')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Checkout' }))
    expect(await screen.findByText('Fetching repository')).toBeInTheDocument()
    expect(screen.getByText('Runner Image')).toBeInTheDocument()
    expect(screen.queryByText('##[endgroup]')).not.toBeInTheDocument()
    expect(screen.getByText('Process completed with exit code 1.')).toHaveClass(
      'text-red-400'
    )
    expect(screen.queryByText(/##\[error\]/)).not.toBeInTheDocument()

    await user.click(screen.getByLabelText('Back to workflow runs'))
    await user.click(screen.getByText('Deploy workflow'))
    await user.click(
      await screen.findByRole('button', {
        name: 'View deploy job details',
      })
    )

    expect(await screen.findByText('Prepare release')).toBeInTheDocument()
    expect(screen.getByText('Ship release')).toBeInTheDocument()
    expect(screen.queryByText('Checkout')).not.toBeInTheDocument()
  })
})
