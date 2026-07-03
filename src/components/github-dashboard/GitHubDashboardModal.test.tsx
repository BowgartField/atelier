import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { screen, waitFor, render } from '@/test/test-utils'
import { useUIStore } from '@/store/ui-store'
import { useProjectsStore } from '@/store/projects-store'
import { GitHubDashboardModal } from './GitHubDashboardModal'

const mockInvoke = vi.hoisted(() => vi.fn())
const mockUseProjects = vi.hoisted(() => vi.fn())
const mockUseGhCliAuth = vi.hoisted(() => vi.fn())

vi.mock('@/lib/transport', () => ({
  invoke: mockInvoke,
  listen: vi.fn(),
}))

vi.mock('@/services/projects', () => ({
  isTauri: () => true,
  isFolder: () => false,
  useProjects: mockUseProjects,
  useCreateWorktree: () => ({ mutateAsync: vi.fn() }),
}))

vi.mock('@/hooks/useGhLogin', () => ({
  useGhLogin: () => ({ triggerLogin: vi.fn(), isGhInstalled: true }),
}))

vi.mock('@/services/gh-cli', () => ({
  useGhCliAuth: mockUseGhCliAuth,
}))

vi.mock('@/components/shared/GhAuthError', () => ({
  GhAuthError: () => <div data-testid="gh-auth-error">GitHub auth prompt</div>,
}))

vi.mock('@/components/worktree/IssuePreviewModal', () => ({
  IssuePreviewModal: () => null,
}))

vi.mock('sonner', () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
    loading: vi.fn(),
  },
}))

const project = {
  id: 'project-1',
  name: 'Project 1',
  path: '/tmp/project-1',
}

function renderDashboard() {
  useUIStore.setState({ githubDashboardOpen: true })
  render(<GitHubDashboardModal />)
}

function emptyIssueResult() {
  return { issues: [], totalCount: 0 }
}

function resolveEmptyDashboardCommand(command: string) {
  if (command === 'list_github_issues')
    return Promise.resolve(emptyIssueResult())
  if (command === 'list_github_prs') return Promise.resolve([])
  if (command === 'list_dependabot_alerts') return Promise.resolve([])
  if (command === 'list_repository_advisories') return Promise.resolve([])
  if (command === 'list_workflow_runs')
    return Promise.resolve({ runs: [], failedCount: 0 })
  return Promise.resolve(null)
}

describe('GitHubDashboardModal auth error handling', () => {
  beforeEach(() => {
    globalThis.ResizeObserver = class ResizeObserver {
      observe = vi.fn()
      unobserve = vi.fn()
      disconnect = vi.fn()
    }

    mockInvoke.mockReset()
    mockUseProjects.mockReset()
    mockUseGhCliAuth.mockReset()

    mockUseProjects.mockReturnValue({ data: [project] })
    mockUseGhCliAuth.mockReturnValue({
      data: undefined,
      isLoading: false,
      isFetching: false,
    })
    useProjectsStore.setState({
      githubDashboardProjectCollapseOverrides: {},
    })
  })

  it('does not show the login prompt for unsupported GitHub remotes that mention gh auth login', async () => {
    mockInvoke.mockImplementation((command: string) => {
      if (command === 'list_github_issues') {
        return Promise.reject(
          'gh issue list failed: none of the git remotes configured for this repository point to a known GitHub host. To tell gh about a new GitHub host, please use `gh auth login`'
        )
      }
      return resolveEmptyDashboardCommand(command)
    })

    renderDashboard()

    expect(await screen.findByText('No open issues found')).toBeInTheDocument()
    expect(screen.queryByTestId('gh-auth-error')).not.toBeInTheDocument()
    expect(mockUseGhCliAuth).toHaveBeenLastCalledWith(
      expect.objectContaining({ enabled: false })
    )
  })

  it('shows a command error instead of a login prompt when gh is authenticated', async () => {
    mockUseGhCliAuth.mockReturnValue({
      data: { authenticated: true, error: null },
      isLoading: false,
      isFetching: false,
    })
    mockInvoke.mockImplementation((command: string) => {
      if (command === 'list_github_issues') {
        return Promise.reject(
          "GitHub CLI not authenticated. Run 'gh auth login' first."
        )
      }
      return resolveEmptyDashboardCommand(command)
    })

    renderDashboard()

    expect(
      await screen.findByText(
        "GitHub CLI not authenticated. Run 'gh auth login' first.",
        {},
        { timeout: 3000 }
      )
    ).toBeInTheDocument()
    expect(screen.queryByTestId('gh-auth-error')).not.toBeInTheDocument()
    await waitFor(() => {
      expect(mockUseGhCliAuth).toHaveBeenLastCalledWith(
        expect.objectContaining({ enabled: true })
      )
    })
  })

  it('shows the login prompt only when gh auth status reports unauthenticated', async () => {
    mockUseGhCliAuth.mockReturnValue({
      data: { authenticated: false, error: 'not logged in' },
      isLoading: false,
      isFetching: false,
    })
    mockInvoke.mockImplementation((command: string) => {
      if (command === 'list_github_issues') {
        return Promise.reject(
          "GitHub CLI not authenticated. Run 'gh auth login' first."
        )
      }
      return resolveEmptyDashboardCommand(command)
    })

    renderDashboard()

    expect(
      await screen.findByTestId('gh-auth-error', {}, { timeout: 3000 })
    ).toBeInTheDocument()
  })

  it('shows the workflows dashboard with per-project summaries', async () => {
    mockInvoke.mockImplementation((command: string) => {
      if (command === 'list_workflow_runs') {
        return Promise.resolve({
          runs: [
            {
              databaseId: 3,
              name: 'deploy',
              displayTitle: 'Deploy production',
              status: 'in_progress',
              conclusion: null,
              event: 'push',
              headBranch: 'main',
              createdAt: '2026-07-03T09:30:00Z',
              url: 'https://github.com/acme/project/actions/runs/3',
              workflowName: 'Deploy',
            },
            {
              databaseId: 2,
              name: 'test',
              displayTitle: 'Test suite',
              status: 'completed',
              conclusion: 'failure',
              event: 'push',
              headBranch: 'main',
              createdAt: '2026-07-03T09:00:00Z',
              url: 'https://github.com/acme/project/actions/runs/2',
              workflowName: 'CI',
            },
            {
              databaseId: 1,
              name: 'build',
              displayTitle: 'Build app',
              status: 'completed',
              conclusion: 'success',
              event: 'push',
              headBranch: 'main',
              createdAt: '2026-07-03T08:30:00Z',
              url: 'https://github.com/acme/project/actions/runs/1',
              workflowName: 'Build',
            },
          ],
          failedCount: 1,
        })
      }
      return resolveEmptyDashboardCommand(command)
    })

    const user = userEvent.setup()
    renderDashboard()

    await user.click(screen.getByRole('button', { name: /Workflows/i }))

    expect(
      await screen.findByText('Deploy', {}, { timeout: 3000 })
    ).toBeInTheDocument()
    expect(screen.getByText('1 running')).toBeInTheDocument()
    expect(screen.getByText('1 failed')).toBeInTheDocument()
    expect(screen.getByText('1 success')).toBeInTheDocument()
    expect(screen.getByText('Open runs')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /Deploy/ }))
    expect(useUIStore.getState().workflowRunsModalWorkflowName).toBe('Deploy')
    expect(useUIStore.getState().workflowRunsModalProjectPath).toBe(
      '/tmp/project-1'
    )
  })

  it('collapses and expands a project in the workflows tab', async () => {
    mockInvoke.mockImplementation((command: string) => {
      if (command === 'list_workflow_runs') {
        return Promise.resolve({
          runs: [
            {
              databaseId: 1,
              name: 'build',
              displayTitle: 'Build app',
              status: 'completed',
              conclusion: 'success',
              event: 'push',
              headBranch: 'main',
              createdAt: '2026-07-03T08:30:00Z',
              url: 'https://github.com/acme/project/actions/runs/1',
              workflowName: 'Build',
            },
          ],
          failedCount: 0,
        })
      }
      return resolveEmptyDashboardCommand(command)
    })

    const user = userEvent.setup()
    renderDashboard()

    await user.click(screen.getByRole('button', { name: /Workflows/i }))
    expect(await screen.findByText('Build')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /Project 1/i }))
    await waitFor(() => {
      expect(screen.queryByText('Build')).not.toBeInTheDocument()
    })

    await user.click(screen.getByRole('button', { name: /Project 1/i }))
    expect(await screen.findByText('Build')).toBeInTheDocument()
  })
})
