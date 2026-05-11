import { describe, expect, it } from 'vitest'
import { isGhAuthError, isUnsupportedGitHubRepoError } from './github'

describe('GitHub service error classification', () => {
  it('does not treat unknown GitHub host remotes as auth errors', () => {
    const error =
      'none of the git remotes configured for this repository point to a known GitHub host.\n' +
      'To tell gh about a new GitHub host, please use `gh auth login`'

    expect(isUnsupportedGitHubRepoError(error)).toBe(true)
    expect(isGhAuthError(error)).toBe(false)
  })

  it('does not treat missing git remotes as auth errors', () => {
    const error = 'no git remotes found'

    expect(isUnsupportedGitHubRepoError(error)).toBe(true)
    expect(isGhAuthError(error)).toBe(false)
  })

  it('detects raw GitHub CLI auth prompts after excluding repo eligibility errors', () => {
    expect(
      isGhAuthError('To get started with GitHub CLI, please run: gh auth login')
    ).toBe(true)
  })

  it('detects standardized GitHub CLI auth errors', () => {
    expect(
      isGhAuthError("GitHub CLI not authenticated. Run 'gh auth login' first.")
    ).toBe(true)
  })
})
